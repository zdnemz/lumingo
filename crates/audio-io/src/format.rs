/// The shape of the interleaved `f32` audio a device stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl StreamFormat {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }

    /// Interleaved samples in one second of audio.
    pub fn samples_per_second(&self) -> u64 {
        u64::from(self.sample_rate) * u64::from(self.channels)
    }
}
