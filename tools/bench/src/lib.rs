//! Benchmark harness: the result format, the statistics rules, machine and
//! memory measurement, and the guard that keeps a `floor` tag honest. Suites for
//! real engines are added as the engines exist.
#![forbid(unsafe_code)]

pub mod machine;
pub mod memory;
pub mod profile;
pub mod result;
pub mod selftest;
pub mod stats;
