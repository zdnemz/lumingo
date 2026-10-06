//! Fake engines for tests. They exist only for this crate's tests and with the
//! `test-support` feature, which no release build enables.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::{EngineProblem, Engines, PartStatus, SpeechSource, SpeechStatus};
use crate::voice::testing::{EnergyVad, FakeAudio, FakeStt, FakeTts, TtsLog, fake_audio};
use crate::voice::{ListenParts, TtsLoad};

/// Speech engines that follow a script. Every `listen` builds a new fake
/// recogniser with the whole list of transcripts, like a recogniser that was
/// loaded again for a new session.
pub struct FakeSpeech {
    transcripts: Vec<String>,
    end_silence_ms: u32,
    /// What the fake synthesiser was asked to say, across every session.
    pub tts_log: Arc<TtsLog>,
    loads: Mutex<usize>,
    fail_stt_load: bool,
}

impl FakeSpeech {
    pub fn new(transcripts: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            transcripts: transcripts.iter().map(|t| (*t).to_owned()).collect(),
            end_silence_ms: 600,
            tts_log: Arc::new(TtsLog::default()),
            loads: Mutex::new(0),
            fail_stt_load: false,
        })
    }

    /// Like [`FakeSpeech::new`], with a recogniser whose load fails.
    pub fn failing_stt() -> Arc<Self> {
        Arc::new(Self {
            transcripts: Vec::new(),
            end_silence_ms: 600,
            tts_log: Arc::new(TtsLog::default()),
            loads: Mutex::new(0),
            fail_stt_load: true,
        })
    }

    /// How many times engines were asked for.
    pub fn loads(&self) -> usize {
        *self.loads.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SpeechSource for FakeSpeech {
    fn status(&self) -> SpeechStatus {
        SpeechStatus {
            vad: PartStatus::Configured,
            stt: PartStatus::Configured,
            tts: PartStatus::Configured,
        }
    }

    fn listen(&self) -> Result<ListenParts, EngineProblem> {
        *self.loads.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        let refs: Vec<&str> = self.transcripts.iter().map(String::as_str).collect();
        let mut stt = FakeStt::new(&refs, None, Duration::ZERO);
        if self.fail_stt_load {
            stt = stt.failing();
        }
        Ok(ListenParts {
            vad: Box::new(EnergyVad::new(None, self.end_silence_ms)),
            stt: Box::new(move || Ok(Box::new(stt) as _)),
        })
    }

    fn tts(&self) -> Result<TtsLoad, EngineProblem> {
        let tts = FakeTts::with_log(None, Duration::ZERO, Arc::clone(&self.tts_log));
        Ok(Box::new(move || Ok(Box::new(tts) as _)))
    }
}

/// A fake audio set-up and fake speech engines, as one [`Engines`].
pub struct FakeEngines {
    pub engines: Engines,
    pub audio: FakeAudio,
    pub speech: Arc<FakeSpeech>,
}

/// Fake devices (a 16 kHz microphone, a 48 kHz stereo speaker) and fake speech
/// that recognises the given transcripts in order.
pub fn fake_engines(transcripts: &[&str]) -> FakeEngines {
    engines_over(FakeSpeech::new(transcripts))
}

/// Fake devices and the given speech engines, as one [`Engines`].
pub fn engines_over(speech: Arc<FakeSpeech>) -> FakeEngines {
    let audio = fake_audio();
    let engines = Engines::new(
        Ok(Arc::clone(&audio.registry)),
        Ok(Arc::clone(&speech) as Arc<dyn SpeechSource>),
        None,
    );
    FakeEngines {
        engines,
        audio,
        speech,
    }
}
