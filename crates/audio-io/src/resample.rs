use std::f64::consts::PI;

/// Zero crossings of the low-pass kernel on each side. 16 gives well over 60 dB
/// of stop-band rejection with a Blackman window, plenty for speech.
const ZERO_CROSSINGS: f64 = 16.0;

/// Converts interleaved device audio of any rate and channel count to 16 kHz
/// mono `f32`, one chunk at a time. Channels are averaged, then a windowed-sinc
/// low-pass keeps frequencies above the new Nyquist from folding back down.
///
/// The output does not depend on how the input is split into chunks. The last
/// few milliseconds of a stream stay buffered (the filter needs future samples),
/// and a stream starts as if preceded by silence.
#[derive(Debug)]
pub struct Resampler {
    channels: usize,
    /// Input samples advanced per output sample.
    step: f64,
    /// Low-pass cutoff as a fraction of the input Nyquist.
    cutoff: f64,
    /// Kernel half-width in input samples.
    half: f64,
    /// Mono input not yet fully consumed.
    buf: Vec<f32>,
    /// Absolute index of `buf[0]` in the mono input stream.
    buf_start: u64,
    /// Position of the next output sample, in the same absolute coordinates.
    next: f64,
    /// A trailing partial frame carried between chunks.
    partial: Vec<f32>,
}

impl Resampler {
    /// Returns `None` for a zero rate or zero channels.
    pub fn new(input_rate: u32, channels: u16, output_rate: u32) -> Option<Self> {
        if input_rate == 0 || channels == 0 || output_rate == 0 {
            return None;
        }
        let cutoff = (f64::from(output_rate) / f64::from(input_rate)).min(1.0);
        Some(Self {
            channels: usize::from(channels),
            step: f64::from(input_rate) / f64::from(output_rate),
            cutoff,
            half: ZERO_CROSSINGS / cutoff,
            buf: Vec::new(),
            buf_start: 0,
            next: 0.0,
            partial: Vec::new(),
        })
    }

    /// Feeds interleaved samples and returns every output sample that is ready.
    pub fn push(&mut self, interleaved: &[f32]) -> Vec<f32> {
        self.partial.extend_from_slice(interleaved);
        let whole = self.partial.len() / self.channels * self.channels;
        let scale = 1.0 / self.channels as f32;
        self.buf.extend(
            self.partial[..whole]
                .chunks_exact(self.channels)
                .map(|frame| frame.iter().sum::<f32>() * scale),
        );
        self.partial.drain(..whole);

        let mut out = Vec::new();
        let end = self.buf_start + self.buf.len() as u64;
        while self.next + self.half < end as f64 {
            out.push(self.sample_at(self.next));
            self.next += self.step;
        }
        // Drop what no later output can reach.
        let keep_from = ((self.next - self.half).floor().max(0.0) as u64).max(self.buf_start);
        self.buf.drain(..(keep_from - self.buf_start) as usize);
        self.buf_start = keep_from;
        out
    }

    fn sample_at(&self, pos: f64) -> f32 {
        let first = (pos - self.half).ceil().max(self.buf_start as f64) as u64;
        let last = (pos + self.half).floor() as u64;
        let mut acc = 0.0;
        for k in first..=last {
            let x = pos - k as f64;
            let Some(&v) = self.buf.get((k - self.buf_start) as usize) else {
                break;
            };
            acc += f64::from(v) * self.cutoff * sinc(self.cutoff * x) * blackman(x / self.half);
        }
        acc as f32
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Blackman window over `t` in [-1, 1].
fn blackman(t: f64) -> f64 {
    if t.abs() >= 1.0 {
        return 0.0;
    }
    let u = (t + 1.0) / 2.0;
    0.42 - 0.5 * (2.0 * PI * u).cos() + 0.08 * (4.0 * PI * u).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f64, seconds: f64, channels: usize) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (2.0 * PI * hz * i as f64 / f64::from(rate)).sin() as f32 * 0.5;
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    /// Frequency from rising zero crossings, ignoring filter edges.
    fn frequency(x: &[f32], rate: f64) -> f64 {
        let x = &x[200..x.len() - 200];
        let crossings = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        crossings as f64 * rate / x.len() as f64
    }

    #[test]
    fn rejects_zero_rate_or_channels() {
        assert!(Resampler::new(0, 1, 16_000).is_none());
        assert!(Resampler::new(48_000, 0, 16_000).is_none());
    }

    #[test]
    fn length_is_the_input_length_scaled_to_16k() {
        for rate in [44_100, 48_000, 16_000, 8_000] {
            let mut r = Resampler::new(rate, 1, 16_000).unwrap();
            let out = r.push(&tone(rate, 1000.0, 3.0, 1));
            let expected = 48_000_i64;
            // The filter holds back about one kernel half-width at the end.
            assert!(
                (out.len() as i64 - expected).abs() < 40,
                "{rate}: {}",
                out.len()
            );
        }
    }

    #[test]
    fn a_1khz_tone_stays_at_1khz() {
        for rate in [44_100, 48_000] {
            let mut r = Resampler::new(rate, 1, 16_000).unwrap();
            let out = r.push(&tone(rate, 1000.0, 2.0, 1));
            let f = frequency(&out, 16_000.0);
            assert!((f - 1000.0).abs() < 10.0, "{rate}: {f}");
            assert!((rms(&out[200..out.len() - 200]) - 0.5 / 2f64.sqrt()).abs() < 0.01);
        }
    }

    #[test]
    fn a_tone_above_the_new_nyquist_is_removed_not_folded() {
        // 10 kHz at 48 kHz would alias to 6 kHz at 16 kHz without the filter.
        let mut r = Resampler::new(48_000, 1, 16_000).unwrap();
        let out = r.push(&tone(48_000, 10_000.0, 1.0, 1));
        assert!(rms(&out[200..out.len() - 200]) < 0.005);
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        let mut r = Resampler::new(16_000, 2, 16_000).unwrap();
        let left_right: Vec<f32> = (0..4000).flat_map(|_| [0.4, 0.0]).collect();
        let out = r.push(&left_right);
        assert!((out[2000] - 0.2).abs() < 1e-3);
    }

    #[test]
    fn output_does_not_depend_on_chunking() {
        let input = tone(44_100, 440.0, 1.0, 2);
        let whole = Resampler::new(44_100, 2, 16_000).unwrap().push(&input);
        let mut r = Resampler::new(44_100, 2, 16_000).unwrap();
        // 333 is odd, so chunks split stereo frames in half.
        let pieces: Vec<f32> = input.chunks(333).flat_map(|c| r.push(c)).collect();
        assert_eq!(whole, pieces);
    }

    #[test]
    fn same_rate_mono_is_the_identity_away_from_the_start() {
        let input = tone(16_000, 700.0, 0.5, 1);
        let out = Resampler::new(16_000, 1, 16_000).unwrap().push(&input);
        assert!(out.iter().zip(&input).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn buffer_stays_bounded_over_a_long_stream() {
        let mut r = Resampler::new(48_000, 1, 16_000).unwrap();
        let chunk = tone(48_000, 300.0, 0.1, 1);
        for _ in 0..200 {
            r.push(&chunk);
        }
        assert!(r.buf.len() < 500);
    }
}
