//! Calibration mode (spec 2026-10-25): Bambu Studio prints the tests,
//! BambuMate guides, reads, saves and records.
//!
//! - `tests`: the pure maths of each test.
//! - `store`: `calibration.db` and the photos.

pub mod store;
pub mod tests;
