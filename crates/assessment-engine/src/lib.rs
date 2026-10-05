//! Scoring a response and estimating a level are two separate steps.
//!
//! This crate holds the pure parts of both: answer normalisation and
//! deterministic scoring, and the level estimator. It never calls a language
//! model, and no model output can set or change a level.
#![forbid(unsafe_code)]

mod normalise;

pub use normalise::{NORM_VERSION, edit_distance, normalize};
