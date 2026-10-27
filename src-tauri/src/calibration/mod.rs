//! Calibration mode (spec 2026-10-25): Bambu Studio prints the tests,
//! BambuMate guides, reads, saves and records.
//!
//! - `tests`: the pure maths of each test.
//! - `apply`: saving a value into the preset, and undoing it.
//! - `store`: `calibration.db` and the photos.
//! - `suggest`: the AI's pick from a photo, which the user must confirm.

pub mod apply;
pub mod store;
pub mod suggest;
pub mod tests;
