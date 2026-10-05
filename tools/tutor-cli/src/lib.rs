//! The Lumingo voice loop on the command line (ROADMAP S3-06, S3-12).
//!
//! The orchestration is `app_core::voice`; this crate is the terminal around it:
//! arguments, provider selection, the audio and speech side, scripted runs and
//! the result file.
#![forbid(unsafe_code)]

pub mod args;
pub mod chat;
#[cfg(feature = "cpal-backend")]
mod cpal_devices;
pub mod engines;
pub mod exit;
pub mod observer;
pub mod provider;
pub mod results;
pub mod script;
#[cfg(feature = "sherpa")]
mod sherpa;
pub mod wav;
