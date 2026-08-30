//! Live model catalog for BambuMate.
//!
//! - [`catalog`] — fetch / cache / normalize model metadata (models.dev + OpenRouter).
//! - [`recommend`] — tier classification + recommendation picker.

pub mod catalog;
pub mod recommend;

pub use catalog::{
    bundled_catalog_summary, detect_preview, fetch_now, get_catalog, lookup, parse_models_dev,
    parse_openrouter, resolve_id, CatalogEntry, BUNDLED_PROVIDERS,
};
pub use recommend::{classify_tier, pick_recommended, MIN_QUALITY_TIER};
