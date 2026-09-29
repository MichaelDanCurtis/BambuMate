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
    /// Counts delivered events, so a refresh that started before one can't
    /// overwrite what it brought.
    events: StoredValue<Seen>,
}

/// The event counters when a request started.
#[derive(Debug, Clone, Copy)]
pub struct Ticket(Seen);

/// How many `printer://state` and `printer://connection` events arrived.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Seen {
    views: u64,
    connections: u64,
}

/// What a refresh that started at `seen` may apply once the counters are at
/// `now`: nothing if a newer full view arrived meanwhile, the fetched view
/// with the newer connection state if only that changed, else the fetched
/// view.
fn refreshed(
    seen: Seen,
    now: Seen,
    fetched: PrinterView,
    current: &ConnectionState,
) -> Option<PrinterView> {
    if now.views != seen.views {
        return None;
    }
    let mut view = fetched;
    if now.connections != seen.connections {
        view.connection = current.clone();
    }
    Some(view)
}

impl PrinterShared {
    pub fn new() -> Self {
        Self {
            view: RwSignal::new(PrinterView::default()),
            events: StoredValue::new(Seen::default()),
        }
    }

    /// Re-reads the view from the backend without undoing newer events.
    pub fn refresh(self) {
        let Some(ticket) = self.ticket() else {
            return;
        };
        spawn_local(async move {
            if let Ok(fetched) = bridge::view().await {
                self.apply(ticket, fetched);
            }
        });
    }

    /// Marks the start of a request whose answer is a full view; pass the
    /// ticket to [`apply`](Self::apply) with that answer.
    pub fn ticket(self) -> Option<Ticket> {
        self.events.try_get_value().map(Ticket)
    }

    /// Applies a view a command returned, unless a newer event arrived since
    /// `ticket` was taken (see [`refreshed`]).
    pub fn apply(self, ticket: Ticket, fetched: PrinterView) {
        let (Some(now), Some(current)) = (
            self.events.try_get_value(),
            self.view.try_with_untracked(|v| v.connection.clone()),
        ) else {
            return;
        };
        if let Some(v) = refreshed(ticket.0, now, fetched, &current) {
            self.view.try_set(v);
        }
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
    crate::agent::bridge::listen::<PrinterView>("printer://state", move |v| {
        shared.events.try_update_value(|n| n.views += 1);
        shared.view.try_set(v);
    });
    crate::agent::bridge::listen::<ConnectionState>("printer://connection", move |c| {
        shared.events.try_update_value(|n| n.connections += 1);
        shared.view.try_update(|v| v.connection = c);
    });
    shared.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(connection: ConnectionState) -> PrinterView {
        PrinterView {
            configured: true,
            connection,
            ..Default::default()
        }
    }

    #[test]
    fn a_refresh_never_overwrites_a_newer_event() {
        let start = Seen::default();
        let fetched = || view(ConnectionState::Connecting);
        let live = ConnectionState::Connected;
        // Nothing arrived meanwhile: the fetched view is applied.
        assert_eq!(refreshed(start, start, fetched(), &live), Some(fetched()));
        // A full view arrived: it is newer, so the fetch is dropped.
        let after_view = Seen { views: 1, ..start };
        assert_eq!(refreshed(start, after_view, fetched(), &live), None);
        // Only the connection changed: keep it, take the rest.
        let after_connection = Seen {
            connections: 1,
            ..start
        };
        assert_eq!(
            refreshed(start, after_connection, fetched(), &live),
            Some(view(ConnectionState::Connected))
        );
    }
}
