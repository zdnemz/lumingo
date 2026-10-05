//! Conversion between device audio and speech audio.
//!
//! [`MonoResampler`] changes the sample rate of one channel and can be fed in
//! blocks of any size. [`CaptureConverter`] puts a channel downmix in front of
//! it, so any device rate and channel count becomes 16 kHz mono `f32`
//! ([`speech::SPEECH_SAMPLE_RATE`]). Playback uses the same resampler in the
//! other direction.
//!
//! The synchronous FFT resampler is used whenever the two rates reduce to small
//! integers (44.1 kHz to 16 kHz is 441 to 160). For odd rates that would need a
//! huge FFT block, the sinc resampler takes over so a strange device rate costs
//! a little CPU instead of a large allocation.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

/// Rates outside this range are not real audio devices. The bound also keeps the
/// resampler's internal size arithmetic far from overflow.
const MAX_RATE: u32 = 768_000;

/// Largest reduced rate ratio side for which the FFT resampler is used.
const MAX_FFT_BLOCK: usize = 2048;

/// Why a conversion could not be set up or run.
#[derive(Debug, thiserror::Error)]
pub enum ResampleError {
    #[error("sample rate {0} Hz is outside the supported range 1 to {MAX_RATE} Hz")]
    UnsupportedRate(u32),
    #[error("a stream needs at least one channel")]
    NoChannels,
    #[error("the resampler could not be built: {0}")]
    Build(#[from] rubato::ResamplerConstructionError),
    #[error("the resampler failed while processing: {0}")]
    Process(#[from] rubato::ResampleError),
    #[error("the resampler did not produce the expected output")]
    NoProgress,
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

enum Engine {
    Passthrough,
    Rubato(Box<dyn Resampler<f32>>),
}

/// Streaming sample-rate conversion for one channel.
///
/// Feed blocks with [`process`](Self::process) and call [`finish`](Self::finish)
/// at the end of a stream or clip. The startup delay of the filter is trimmed, so
/// the total output is exactly `ceil(input_frames * out_rate / in_rate)` frames.
pub struct MonoResampler {
    engine: Engine,
    in_rate: u32,
    out_rate: u32,
    /// Input not yet consumed because the engine wants a full chunk.
    pending: Vec<f32>,
    scratch: Vec<f32>,
    /// Output frames still to drop to remove the filter delay.
    skip: usize,
    total_in: u64,
    total_out: u64,
}

impl std::fmt::Debug for MonoResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonoResampler")
            .field("in_rate", &self.in_rate)
            .field("out_rate", &self.out_rate)
            .finish_non_exhaustive()
    }
}

impl MonoResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Result<Self, ResampleError> {
        for rate in [in_rate, out_rate] {
            if rate == 0 || rate > MAX_RATE {
                return Err(ResampleError::UnsupportedRate(rate));
            }
        }
        let engine = if in_rate == out_rate {
            Engine::Passthrough
        } else {
            Engine::Rubato(build_engine(in_rate, out_rate)?)
        };
        let (skip, scratch_len) = match &engine {
            Engine::Passthrough => (0, 0),
            Engine::Rubato(r) => (r.output_delay(), r.output_frames_max()),
        };
        Ok(Self {
            engine,
            in_rate,
            out_rate,
            pending: Vec::new(),
            scratch: vec![0.0; scratch_len],
            skip,
            total_in: 0,
            total_out: 0,
        })
    }

    pub fn in_rate(&self) -> u32 {
        self.in_rate
    }

    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// Frames the whole stream will produce for `input_frames` of input.
    fn expected_total(&self, input_frames: u64) -> u64 {
        (input_frames * u64::from(self.out_rate)).div_ceil(u64::from(self.in_rate))
    }

    /// Converts `input` and appends whatever output is ready to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), ResampleError> {
        self.total_in += input.len() as u64;
        let Engine::Rubato(resampler) = &mut self.engine else {
            out.extend_from_slice(input);
            self.total_out += input.len() as u64;
            return Ok(());
        };
        self.pending.extend_from_slice(input);
        loop {
            let need = resampler.input_frames_next();
            if self.pending.len() < need {
                return Ok(());
            }
            let (used, made) =
                run_chunk(resampler.as_mut(), &self.pending, &mut self.scratch, None)?;
            self.pending.drain(..used);
            append_trimmed(
                &self.scratch[..made],
                &mut self.skip,
                &mut self.total_out,
                out,
            );
        }
    }

    /// Flushes the filter so the last input samples reach the output, appends the
    /// tail to `out` and resets the resampler for the next stream or clip.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), ResampleError> {
        let expected = self.expected_total(self.total_in);
        if let Engine::Rubato(resampler) = &mut self.engine {
            let start = out.len();
            let mut valid = Some(self.pending.len());
            // A flush needs at most the delay plus one chunk of output. The cap
            // turns a resampler that stops producing into an error, not a hang.
            let mut rounds = 0_usize;
            while self.total_out < expected {
                rounds += 1;
                if rounds > 64 {
                    return Err(ResampleError::NoProgress);
                }
                let (used, made) =
                    run_chunk(resampler.as_mut(), &self.pending, &mut self.scratch, valid)?;
                self.pending.drain(..used.min(self.pending.len()));
                valid = Some(self.pending.len());
                append_trimmed(
                    &self.scratch[..made],
                    &mut self.skip,
                    &mut self.total_out,
                    out,
                );
            }
            let excess = (self.total_out - expected) as usize;
            out.truncate(out.len() - excess.min(out.len() - start));
        }
        self.reset();
        Ok(())
    }

    /// Forgets all state so the next samples start a new stream.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.total_in = 0;
        self.total_out = 0;
        if let Engine::Rubato(resampler) = &mut self.engine {
            resampler.reset();
            self.skip = resampler.output_delay();
        }
    }

    /// Converts a whole clip in one call. The resampler is left reset.
    pub fn convert_clip(&mut self, clip: &[f32]) -> Result<Vec<f32>, ResampleError> {
        self.reset();
        let mut out = Vec::with_capacity(self.expected_total(clip.len() as u64) as usize + 1);
        self.process(clip, &mut out)?;
        self.finish(&mut out)?;
        Ok(out)
    }
}

fn build_engine(in_rate: u32, out_rate: u32) -> Result<Box<dyn Resampler<f32>>, ResampleError> {
    let (in_hz, out_hz) = (in_rate as usize, out_rate as usize);
    let divisor = gcd(in_hz, out_hz);
    // About 20 ms of input per call keeps latency low at every rate.
    let chunk = (in_hz / 50).max(64);
    if in_hz / divisor <= MAX_FFT_BLOCK && out_hz / divisor <= MAX_FFT_BLOCK {
        return Ok(Box::new(Fft::<f32>::new(
            in_hz,
            out_hz,
            chunk,
            1,
            FixedSync::Input,
        )?));
    }
    let parameters = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: None,
        oversampling_factor: 128,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    };
    Ok(Box::new(Async::<f32>::new_sinc(
        f64::from(out_rate) / f64::from(in_rate),
        1.0,
        &parameters,
        chunk,
        1,
        FixedAsync::Input,
    )?))
}

/// Runs one engine call on the front of `pending`. `partial` is the number of real
/// frames when this is the last, short chunk: the engine treats the rest as silence
/// and only needs that many input frames to exist.
fn run_chunk(
    resampler: &mut dyn Resampler<f32>,
    pending: &[f32],
    scratch: &mut [f32],
    partial: Option<usize>,
) -> Result<(usize, usize), ResampleError> {
    let frames = partial.map_or_else(|| resampler.input_frames_next(), |n| n.min(pending.len()));
    let input = InterleavedSlice::new(pending, 1, frames).map_err(|_| ResampleError::NoProgress)?;
    let capacity = scratch.len();
    let mut output =
        InterleavedSlice::new_mut(scratch, 1, capacity).map_err(|_| ResampleError::NoProgress)?;
    let indexing = partial.map(|n| rubato::Indexing::new().partial_len(n.min(frames)));
    Ok(resampler.process_into_buffer(&input, &mut output, indexing.as_ref())?)
}

fn append_trimmed(block: &[f32], skip: &mut usize, total_out: &mut u64, out: &mut Vec<f32>) {
    let drop = (*skip).min(block.len());
    *skip -= drop;
    out.extend_from_slice(&block[drop..]);
    *total_out += (block.len() - drop) as u64;
}

/// Device audio in, 16 kHz mono `f32` out.
#[derive(Debug)]
pub struct CaptureConverter {
    channels: usize,
    resampler: MonoResampler,
    /// Samples of an incomplete frame left over from the previous block.
    carry: Vec<f32>,
    mono: Vec<f32>,
}

impl CaptureConverter {
    pub fn new(device_rate: u32, channels: u16) -> Result<Self, ResampleError> {
        if channels == 0 {
            return Err(ResampleError::NoChannels);
        }
        Ok(Self {
            channels: usize::from(channels),
            resampler: MonoResampler::new(device_rate, speech::SPEECH_SAMPLE_RATE)?,
            carry: Vec::new(),
            mono: Vec::new(),
        })
    }

    /// Converts a block of interleaved device samples. Blocks may be any length,
    /// including ones that end in the middle of a frame.
    pub fn process(
        &mut self,
        interleaved: &[f32],
        out: &mut Vec<f32>,
    ) -> Result<(), ResampleError> {
        self.mono.clear();
        let channels = self.channels;
        let mut rest = interleaved;
        if !self.carry.is_empty() {
            let take = (channels - self.carry.len()).min(rest.len());
            self.carry.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.carry.len() == channels {
                self.mono.push(average(&self.carry));
                self.carry.clear();
            }
        }
        let whole = rest.len() - rest.len() % channels;
        self.mono
            .extend(rest[..whole].chunks_exact(channels).map(average));
        self.carry.extend_from_slice(&rest[whole..]);
        self.resampler.process(&self.mono, out)
    }

    /// Flushes the tail at the end of a stream and resets the converter.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), ResampleError> {
        self.carry.clear();
        self.resampler.finish(out)
    }
}

fn average(frame: &[f32]) -> f32 {
    frame.iter().sum::<f32>() / frame.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// Linear sine sweep from `f0` to `f1` Hz over `seconds`.
    fn sweep(rate: u32, seconds: f32, f0: f32, f1: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        let k = (f1 - f0) / seconds;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (2.0 * PI * (f0 * t + 0.5 * k * t * t)).sin() * 0.8
            })
            .collect()
    }

    fn tone(rate: u32, seconds: f32, hz: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    /// Amplitude of the `hz` component of `x` at sample rate `rate`, by the
    /// Goertzel recurrence. A unit sine at exactly `hz` gives about 1.0.
    fn goertzel_amplitude(x: &[f32], rate: u32, hz: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * f64::from(hz) / f64::from(rate);
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
        for &sample in x {
            let s0 = f64::from(sample) + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
        (2.0 * power.max(0.0).sqrt() / x.len() as f64) as f32
    }

    /// The frequency in `lo..=hi` with the most energy, scanned in 5 Hz steps.
    fn peak_hz(x: &[f32], rate: u32, lo: f32, hi: f32) -> f32 {
        let mut best = (lo, 0.0_f32);
        let mut hz = lo;
        while hz <= hi {
            let a = goertzel_amplitude(x, rate, hz);
            if a > best.1 {
                best = (hz, a);
            }
            hz += 5.0;
        }
        best.0
    }

    fn to_speech(rate: u32, channels: u16, interleaved: &[f32], block: usize) -> Vec<f32> {
        let mut conv = CaptureConverter::new(rate, channels).expect("converter builds");
        let mut out = Vec::new();
        // Block sizes that are not a multiple of the channel count exercise the carry.
        for part in interleaved.chunks(block) {
            conv.process(part, &mut out).expect("block converts");
        }
        conv.finish(&mut out).expect("flush works");
        out
    }

    fn interleave(mono: &[f32], channels: usize) -> Vec<f32> {
        mono.iter()
            .flat_map(|&s| std::iter::repeat_n(s, channels))
            .collect()
    }

    fn expected_len(frames: usize, rate: u32) -> usize {
        (frames as u64 * 16_000).div_ceil(u64::from(rate)) as usize
    }

    #[test]
    fn sweeps_at_44k_and_48k_convert_to_exactly_the_expected_length() {
        for (rate, channels) in [(44_100_u32, 1_u16), (44_100, 2), (48_000, 1), (48_000, 2)] {
            let mono = sweep(rate, 2.0, 100.0, 7_000.0);
            let input = interleave(&mono, usize::from(channels));
            for block in [input.len(), 4410, 1001, 7] {
                let out = to_speech(rate, channels, &input, block);
                assert_eq!(
                    out.len(),
                    expected_len(mono.len(), rate),
                    "rate {rate} channels {channels} block {block}"
                );
                assert!(out.iter().all(|s| s.is_finite() && s.abs() < 1.1));
            }
        }
    }

    #[test]
    fn a_one_khz_tone_stays_at_one_khz() {
        for (rate, channels) in [(44_100_u32, 1_u16), (44_100, 2), (48_000, 1), (48_000, 2)] {
            let mono = tone(rate, 1.5, 1_000.0);
            let input = interleave(&mono, usize::from(channels));
            let out = to_speech(rate, channels, &input, 2_048);
            // Skip the edges so the filter ramps do not colour the measurement.
            let middle = &out[4_000..out.len() - 4_000];
            let peak = peak_hz(middle, 16_000, 500.0, 2_000.0);
            assert!(
                (peak - 1_000.0).abs() <= 5.0,
                "peak at {peak} Hz for {rate} Hz x {channels}"
            );
            let amplitude = goertzel_amplitude(middle, 16_000, 1_000.0);
            assert!(
                (amplitude - 1.0).abs() < 0.05,
                "amplitude {amplitude} for {rate} Hz x {channels}"
            );
        }
    }

    #[test]
    fn tones_above_the_new_nyquist_are_removed_not_folded_down() {
        // 10 kHz would alias to 6 kHz at 16 kHz if the filter did not remove it.
        let mono = tone(44_100, 1.0, 10_000.0);
        let out = to_speech(44_100, 1, &mono, 4_410);
        let middle = &out[2_000..out.len() - 2_000];
        let leaked = goertzel_amplitude(middle, 16_000, 6_000.0);
        assert!(leaked < 0.02, "aliased amplitude {leaked}");
    }

    #[test]
    fn stereo_with_one_silent_channel_averages_to_half_amplitude() {
        let mono = tone(48_000, 1.0, 1_000.0);
        let input: Vec<f32> = mono.iter().flat_map(|&s| [s, 0.0]).collect();
        let out = to_speech(48_000, 2, &input, 960);
        let middle = &out[2_000..out.len() - 2_000];
        let amplitude = goertzel_amplitude(middle, 16_000, 1_000.0);
        assert!((amplitude - 0.5).abs() < 0.03, "amplitude {amplitude}");
    }

    #[test]
    fn streaming_in_odd_blocks_matches_one_shot_conversion() {
        let mono = sweep(44_100, 1.0, 200.0, 6_000.0);
        let input = interleave(&mono, 2);
        let whole = to_speech(44_100, 2, &input, input.len());
        let pieces = to_speech(44_100, 2, &input, 777);
        assert_eq!(whole.len(), pieces.len());
        let worst = whole
            .iter()
            .zip(&pieces)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 1e-4, "block size changed the result by {worst}");
    }

    #[test]
    fn the_device_rate_of_sixteen_khz_passes_through_untouched() {
        let mono = tone(16_000, 0.5, 440.0);
        let out = to_speech(16_000, 1, &mono, 513);
        assert_eq!(out, mono);
    }

    #[test]
    fn common_and_unusual_device_rates_all_give_the_expected_length() {
        for rate in [
            8_000_u32, 11_025, 22_050, 32_000, 88_200, 96_000, 192_000, 44_101,
        ] {
            let mono = tone(rate, 0.5, 700.0);
            let out = to_speech(rate, 1, &mono, 3_000);
            assert_eq!(out.len(), expected_len(mono.len(), rate), "rate {rate}");
            let middle = &out[1_000..out.len() - 1_000];
            let amplitude = goertzel_amplitude(middle, 16_000, 700.0);
            assert!((amplitude - 1.0).abs() < 0.08, "rate {rate}: {amplitude}");
        }
    }

    #[test]
    fn a_clip_shorter_than_one_chunk_still_comes_out_whole() {
        let mut resampler = MonoResampler::new(24_000, 48_000).expect("builds");
        let clip = tone(24_000, 0.004, 1_000.0);
        let out = resampler.convert_clip(&clip).expect("converts");
        assert_eq!(out.len(), clip.len() * 2);
    }

    #[test]
    fn the_resampler_can_be_reused_for_the_next_clip() {
        let mut resampler = MonoResampler::new(24_000, 48_000).expect("builds");
        let clip = tone(24_000, 0.3, 1_000.0);
        let first = resampler.convert_clip(&clip).expect("first");
        let second = resampler.convert_clip(&clip).expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn bad_parameters_are_errors_not_panics() {
        assert!(matches!(
            CaptureConverter::new(48_000, 0),
            Err(ResampleError::NoChannels)
        ));
        assert!(matches!(
            CaptureConverter::new(0, 2),
            Err(ResampleError::UnsupportedRate(0))
        ));
        assert!(matches!(
            MonoResampler::new(16_000, 4_000_000),
            Err(ResampleError::UnsupportedRate(4_000_000))
        ));
    }
}
