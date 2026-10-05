use crate::{CancelFlag, EngineInfo, TtsError};

/// A block of mono audio at the engine's own sample rate.
#[derive(Debug, Clone, PartialEq)]
pub struct PcmChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl PcmChunk {
    /// Length of the audio in milliseconds, rounded down.
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        self.samples.len() as u64 * 1000 / u64::from(self.sample_rate)
    }
}

pub trait TtsEngine: Send {
    /// Synthesises one sentence. Returns PCM at the engine's sample rate.
    fn synthesize(&mut self, text: &str, cancel: &CancelFlag) -> Result<PcmChunk, TtsError>;
    fn info(&self) -> EngineInfo;
}

#[cfg(test)]
mod tests {
    use super::PcmChunk;

    #[test]
    fn duration_is_samples_over_rate() {
        let chunk = PcmChunk {
            samples: vec![0.0; 24_000],
            sample_rate: 24_000,
        };
        assert_eq!(chunk.duration_ms(), 1000);
        let short = PcmChunk {
            samples: vec![0.0; 100],
            sample_rate: 16_000,
        };
        assert_eq!(short.duration_ms(), 6);
    }

    #[test]
    fn a_zero_rate_does_not_divide_by_zero() {
        let chunk = PcmChunk {
            samples: vec![0.0; 10],
            sample_rate: 0,
        };
        assert_eq!(chunk.duration_ms(), 0);
    }
}
