//! Frontend side of the live printer connection: shared state fed by the
//! `printer://state` and `printer://connection` events.

pub mod bridge;
pub mod types;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use types::{ConnectionState, PrinterView};

/// The latest printer view, shared by the rail dot, the Printer page and
/// Settings → Printer.
#[derive(Clone, Copy)]
pub struct PrinterShared {
    pub view: RwSignal<PrinterView>,
}

impl PrinterShared {
    pub fn new() -> Self {
        Self {
            view: RwSignal::new(PrinterView::default()),
        }
    }

    /// Re-reads the view from the backend.
    pub fn refresh(self) {
        spawn_local(async move {
            if let Ok(v) = bridge::view().await {
                self.view.set(v);
            }
        });
    }
}

impl Default for PrinterShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Registers the printer event listeners. Like `AgentEvents`, mount it
/// exactly once, outside anything that can unmount.
#[component]
pub fn PrinterEvents() -> impl IntoView {
    let shared = expect_context::<PrinterShared>();
    crate::agent::bridge::listen::<PrinterView>("printer://state", move |v| shared.view.set(v));
    crate::agent::bridge::listen::<ConnectionState>("printer://connection", move |c| {
        shared.view.update(|v| v.connection = c)
    });
    shared.refresh();
}
