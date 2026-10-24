//! The filament advisor (spec 2026-10-23): which material, and which
//! products, to print a part with. Scoring is offline and deterministic;
//! AI only turns a description into needs.

pub mod interpret;
pub mod materials;
pub mod needs;
pub mod products;
pub mod score;
