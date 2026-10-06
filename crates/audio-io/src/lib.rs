//! Audio plumbing for the server process. Hardware adapters (CPAL) will sit
//! behind an off-by-default feature; everything here is pure and testable.
#![forbid(unsafe_code)]

mod resample;
mod ring;

pub use resample::Resampler;
pub use ring::{Consumer, Producer, ring};
