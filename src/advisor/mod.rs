//! Filament → Advise on the frontend: mirrors of the backend's answer, the
//! invoke wrappers and the pure formatting the page uses.

pub mod bridge;
pub mod format;
pub mod types;

use leptos::prelude::*;

use crate::commands::CatalogEntry;

/// A catalog entry the Advise page hands to Search for "Generate profile".
/// Search takes it on mount and runs its usual flow from that entry.
#[derive(Clone, Copy)]
pub struct PendingCatalogPick(pub RwSignal<Option<CatalogEntry>>);

impl PendingCatalogPick {
    pub fn new() -> Self {
        Self(RwSignal::new(None))
    }
}
