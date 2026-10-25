//! Printer → Journal on the frontend (spec 2026-10-24): mirrors of the
//! backend's entries, the invoke wrappers, pure formatting, and the
//! `journal://changed` listener the pages reload on.

pub mod bridge;
pub mod format;
pub mod types;

use std::sync::atomic::{AtomicU64, Ordering};

use leptos::prelude::*;

static REVISION: AtomicU64 = AtomicU64::new(0);

/// A number that rises with every journal request the page makes, taken
/// when the request is sent. A list copy made from a later request is the
/// newer one, whatever order the answers arrive in; the backend's
/// `updated_at` is whole seconds and can't tell them apart.
pub fn next_revision() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed) + 1
}

/// Bumped on every `journal://changed`. The Journal page and Home track it.
#[derive(Clone, Copy)]
pub struct JournalShared {
    pub changes: RwSignal<u64>,
}

impl JournalShared {
    pub fn new() -> Self {
        Self {
            changes: RwSignal::new(0),
        }
    }
}

impl Default for JournalShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Registers the `journal://changed` listener. It is removed when this
/// unmounts, and a bump after the owner is gone is dropped (`try_update`).
/// App mounts it once, next to `PrinterEvents`.
#[component]
pub fn JournalEvents() -> impl IntoView {
    let shared = expect_context::<JournalShared>();
    crate::agent::bridge::listen_scoped::<serde_json::Value>("journal://changed", move |_| {
        shared.changes.try_update(|n| *n = n.wrapping_add(1));
    });
}
