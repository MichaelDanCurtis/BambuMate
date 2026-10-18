//! A hub: one rail destination whose pages are tabs (spec 2026-10-17 §1).
//! Wave 1 mounts the existing pages unchanged under the shared header.
//!
//! Each tabbed hub is a parent route (see `app.rs`) and its tabs are the
//! child routes, shown in the `Outlet`. Switching tabs swaps only the
//! child, so the header, and the tab link the user just activated, stay
//! mounted and keep keyboard focus.

use leptos::prelude::*;
use leptos_router::components::Outlet;

use crate::components::patterns::PageHeader;
use crate::routes::Tab;

#[component]
pub fn HubPage(title: &'static str, tabs: &'static [Tab]) -> impl IntoView {
    view! {
        <div class="hub" data-hub=title.to_ascii_lowercase()>
            <PageHeader title=title tabs=tabs />
            <Outlet />
        </div>
    }
}
