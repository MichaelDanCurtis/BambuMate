//! Filament → Advise (spec 2026-10-23 §2). Task 7 builds the page.

use leptos::prelude::*;

use crate::advisor::format::BEFORE_ADVISING;
use crate::components::patterns::{StateKind, StateText};

#[component]
pub fn AdvisePage() -> impl IntoView {
    view! {
        <div class="page advise-page">
            <StateText kind=StateKind::Empty message=BEFORE_ADVISING />
        </div>
    }
}
