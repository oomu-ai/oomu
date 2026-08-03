//! Small, dependency-light primitives shared across native domains.
//!
//! This module is intentionally forbidden from importing application domains so
//! security-sensitive hashing and time semantics cannot drift between features.

pub mod clock;
pub mod digest;
pub(crate) mod public_web_sources;
