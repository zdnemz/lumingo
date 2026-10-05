#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! Session behaviour against the fake backend: frames flow, the gate reacts to
//! playback, and a lost device is reported and recovered without a restart.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use audio_io::fake::FakeBackend;
use audio_io::{
    AudioEvent, AudioSession, Choice, DeviceId, DeviceRegistry, Direction, MemoryPrefs, Route,
    SessionConfig, SessionError, StreamFormat,
};
use speech::PcmChunk;

const MIC_44K: StreamFormat = StreamFormat {
    sample_rate: 44_100,
    channels: 2,
};
const MIC_16K: StreamFormat = StreamFormat {
    sample_rate: 16_000,
    channels: 1,
};
const OUT_48K: StreamFormat = StreamFormat {
    sample_rate: 48_000,
    channels: 2,
};

#[derive(Default)]
struct Seen {
    vad: AtomicUsize,
    dropped: AtomicUsize,
    ptt: AtomicUsize,
    bad_frame_len: AtomicUsize,
}

struct Rig {
    backend: FakeBackend,
    registry: Arc<DeviceRegistry>,
    seen: Arc<Seen>,
    builtin_mic: DeviceId,
    headset_mic: DeviceId,
    speakers: DeviceId,
    headset_out: DeviceId,
}

fn rig() -> Rig {
    let backend = FakeBackend::new();
    let builtin_mic = backend.add_device("Built-in Mic", Direction::Input, MIC_44K, true);
    let headset_mic = backend.add_device("USB Headset Mic", Direction::Input, MIC_16K, false);
    let speakers = backend.add_device("Speakers", Direction::Output, OUT_48K, true);
    let headset_out = backend.add_device("USB Headset", Direction::Output, OUT_48K, false);
    backend.set_input_tone(Some(300.0));
    let registry = Arc::new(DeviceRegistry::new(
        Arc::new(backend.clone()),
        Box::new(MemoryPrefs::default()),
    ));
    Rig {
        backend,
        registry,
        seen: Arc::new(Seen::default()),
        builtin_mic,
        headset_mic,
        speakers,
        headset_out,
    }
}

fn start(rig: &Rig, config: SessionConfig) -> Result<AudioSession, SessionError> {
    let seen = Arc::clone(&rig.seen);
    AudioSession::start(
        Arc::clone(&rig.registry),
        config,
        Box::new(move |frame, route| {
            if frame.len() != 512 {
                seen.bad_frame_len.fetch_add(1, Ordering::Relaxed);
            }
            let counter = match route {
                Route::Vad => &seen.vad,
                Route::Dropped => &seen.dropped,
                Route::PushToTalk => &seen.ptt,
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }),
    )
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

fn tone_chunk(ms: u32) -> PcmChunk {
    let n = 24_000 * ms as usize / 1000;
    PcmChunk {
        samples: (0..n)
            .map(|i| 0.4 * (2.0 * std::f32::consts::PI * 500.0 * i as f32 / 24_000.0).sin())
            .collect(),
        sample_rate: 24_000,
    }
}

#[test]
fn frames_flow_from_a_44k_stereo_microphone_in_whole_512_sample_frames() {
    let rig = rig();
    let session = start(&rig, SessionConfig::default()).expect("starts");
    assert!(wait_until(Duration::from_secs(3), || rig
        .seen
        .vad
        .load(Ordering::Relaxed)
        >= 20));
    let stats = session.stats();
    assert_eq!(stats.capture.overflow_events, 0);
    assert_eq!(rig.seen.bad_frame_len.load(Ordering::Relaxed), 0);
    assert_eq!(rig.seen.dropped.load(Ordering::Relaxed), 0);
    session.stop();
}

#[test]
fn playback_closes_the_gate_and_it_reopens_150_ms_after() {
    let rig = rig();
    let session = start(&rig, SessionConfig::default()).expect("starts");
    let playback = session.playback();
    assert!(wait_until(Duration::from_secs(3), || rig
        .seen
        .vad
        .load(Ordering::Relaxed)
        >= 5));

    playback.enqueue(&tone_chunk(600)).expect("queued");
    playback.finish_turn();
    assert!(
        wait_until(Duration::from_secs(3), || rig
            .seen
            .dropped
            .load(Ordering::Relaxed)
            >= 10),
        "frames are dropped while the tutor speaks"
    );
    assert!(wait_until(Duration::from_secs(3), || !playback.is_active()));
    let vad_after_playback = rig.seen.vad.load(Ordering::Relaxed);
    assert!(
        wait_until(Duration::from_secs(3), || rig
            .seen
            .vad
            .load(Ordering::Relaxed)
            > vad_after_playback + 5),
        "frames reach the VAD again after the hold"
    );
    let stats = session.stats();
    assert_eq!(stats.playback.underruns, 0);
    assert!(
        rig.backend.rendered().iter().any(|s| s.abs() > 0.1),
        "the speaker received the tone"
    );
    session.stop();
}

#[test]
fn push_to_talk_frames_are_routed_separately_during_playback() {
    let rig = rig();
    let session = start(&rig, SessionConfig::default()).expect("starts");
    session
        .playback()
        .enqueue(&tone_chunk(800))
        .expect("queued");
    session.push_to_talk().set(true);
    assert!(wait_until(Duration::from_secs(3), || rig
        .seen
        .ptt
        .load(Ordering::Relaxed)
        >= 5));
    session.push_to_talk().set(false);
    session.stop();
}

#[test]
fn an_unplugged_headset_is_reported_and_capture_continues_after_one_recover() {
    let rig = rig();
    rig.registry
        .select(Direction::Input, Some(&rig.headset_mic))
        .expect("remembers the headset");
    let mut session = start(&rig, SessionConfig::default()).expect("starts");
    assert_eq!(session.input_device().name, "USB Headset Mic");
    assert!(wait_until(Duration::from_secs(3), || rig
        .seen
        .vad
        .load(Ordering::Relaxed)
        >= 10));

    rig.backend.unplug(&rig.headset_mic);
    let event = session.wait_event(Duration::from_secs(2)).expect("event");
    assert_eq!(
        event,
        AudioEvent::DeviceLost {
            direction: Direction::Input,
            device: "USB Headset Mic".to_owned()
        }
    );
    // Audio stops arriving from the lost device.
    std::thread::sleep(Duration::from_millis(100));
    let frozen = rig.seen.vad.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.seen.vad.load(Ordering::Relaxed) <= frozen + 1);

    // One action: recover. The built-in mic has another rate and channel count.
    let resolved = session.recover(Direction::Input).expect("recovers");
    assert_eq!(resolved.info.id, rig.builtin_mic);
    assert_eq!(
        resolved.choice,
        Choice::DefaultInstead {
            missing: "USB Headset Mic".to_owned()
        }
    );
    assert_eq!(
        session.wait_event(Duration::from_secs(1)),
        Some(AudioEvent::Recovered {
            direction: Direction::Input,
            device: "Built-in Mic".to_owned(),
            choice: Choice::DefaultInstead {
                missing: "USB Headset Mic".to_owned()
            }
        })
    );
    let before = rig.seen.vad.load(Ordering::Relaxed);
    assert!(
        wait_until(Duration::from_secs(3), || rig
            .seen
            .vad
            .load(Ordering::Relaxed)
            >= before + 10),
        "frames flow again from the new device"
    );
    assert_eq!(rig.seen.bad_frame_len.load(Ordering::Relaxed), 0);
    assert_eq!(rig.backend.live_streams(), 2, "one input, one output");
    session.stop();
    assert_eq!(rig.backend.live_streams(), 0);
}

#[test]
fn a_lost_speaker_flushes_playback_reports_and_recovers_on_the_default() {
    let rig = rig();
    rig.registry
        .select(Direction::Output, Some(&rig.headset_out))
        .expect("remembers the headset");
    let mut session = start(&rig, SessionConfig::default()).expect("starts");
    let playback = session.playback();
    playback.enqueue(&tone_chunk(3_000)).expect("queued");
    assert!(playback.is_active());

    rig.backend.unplug(&rig.headset_out);
    let event = session.wait_event(Duration::from_secs(2)).expect("event");
    assert_eq!(
        event,
        AudioEvent::DeviceLost {
            direction: Direction::Output,
            device: "USB Headset".to_owned()
        }
    );
    assert!(
        !playback.is_active(),
        "audio that can no longer play must not hold the microphone gate shut"
    );

    let resolved = session.recover(Direction::Output).expect("recovers");
    assert_eq!(resolved.info.id, rig.speakers);
    playback.enqueue(&tone_chunk(200)).expect("plays again");
    playback.finish_turn();
    assert!(wait_until(Duration::from_secs(3), || !playback.is_active()));
    assert!(rig.backend.rendered().iter().any(|s| s.abs() > 0.1));
    session.stop();
}

#[test]
fn recovery_fails_with_a_typed_error_when_no_device_is_left() {
    let rig = rig();
    let mut session = start(&rig, SessionConfig::default()).expect("starts");
    rig.backend.unplug(&rig.builtin_mic);
    rig.backend.unplug(&rig.headset_mic);
    let err = session
        .recover(Direction::Input)
        .expect_err("nothing to recover to");
    assert!(
        matches!(
            err,
            SessionError::Device(audio_io::DeviceError::NoDevice(Direction::Input))
        ),
        "{err}"
    );
    // Plug something in: recovery works without restarting the session.
    rig.backend
        .add_device("New Mic", Direction::Input, MIC_16K, true);
    assert!(session.recover(Direction::Input).is_ok());
    session.stop();
}

#[test]
fn events_use_a_bounded_channel_and_count_what_they_drop() {
    let rig = rig();
    let config = SessionConfig {
        event_capacity: 1,
        ..SessionConfig::default()
    };
    let session = start(&rig, config).expect("starts");
    rig.backend.unplug(&rig.builtin_mic);
    rig.backend.unplug(&rig.speakers);
    assert!(session.try_event().is_some());
    assert!(session.try_event().is_none());
    assert_eq!(session.stats().events_dropped, 1);
    session.stop();
}

#[test]
fn a_start_that_fails_halfway_leaves_no_stream_or_thread_behind() {
    let rig = rig();
    rig.backend.set_open_failure(Direction::Input, true);
    let err = start(&rig, SessionConfig::default()).expect_err("input cannot open");
    assert!(matches!(err, SessionError::Device(_)), "{err}");
    assert_eq!(rig.backend.live_streams(), 0);
    assert_eq!(rig.backend.live_threads(), 0);

    rig.backend.set_open_failure(Direction::Input, false);
    let session = start(&rig, SessionConfig::default()).expect("starts once it can");
    session.stop();
}

#[test]
fn bad_settings_are_refused() {
    let rig = rig();
    let zero_frame = SessionConfig {
        frame_len: 0,
        ..SessionConfig::default()
    };
    assert!(matches!(
        start(&rig, zero_frame),
        Err(SessionError::Settings(_))
    ));
}
