use crate::VadError;

/// Voice activity detection over fixed-size frames of 16 kHz mono audio.
pub trait Vad: Send {
    /// Consumes one frame and returns the probability that it holds speech.
    fn push_frame(&mut self, frame: &[f32]) -> Result<f32, VadError>;
    /// Forgets all state, for the start of a new utterance or session.
    fn reset(&mut self);
}
