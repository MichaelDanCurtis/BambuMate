//! A hub: one rail destination whose pages are tabs (spec 2026-10-17 §1).
//! Wave 1 mounts the existing pages unchanged under the shared header.

use leptos::prelude::*;

use crate::components::patterns::PageHeader;
use crate::routes::Tab;

#[component]
pub fn HubPage(title: &'static str, tabs: &'static [Tab], children: Children) -> impl IntoView {
    view! {
        <div class="hub" data-hub=title.to_ascii_lowercase()>
            <PageHeader title=title tabs=tabs />
            {children()}
        </div>
    }
}
