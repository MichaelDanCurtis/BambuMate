//! Monoline icons: 24×24, 1.5px stroke, round caps, drawn in currentColor.

use leptos::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Home,
    Create,
    Analyze,
    Profiles,
    Batch,
    Compare,
    Settings,
    Health,
    About,
    Lock,
}

#[component]
pub fn Icon(kind: IconKind) -> impl IntoView {
    let body = match kind {
        IconKind::Home => view! { <path d="M4 11l8-6 8 6v8a1 1 0 0 1-1 1h-4v-5h-6v5H5a1 1 0 0 1-1-1z"/> }.into_any(),
        IconKind::Create => view! { <circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="2.5"/><path d="M12 4v3M12 17v3"/> }.into_any(),
        IconKind::Analyze => view! { <rect x="3.5" y="6" width="17" height="13" rx="2"/><circle cx="12" cy="12.5" r="3.5"/><path d="M8 6l1.5-2h5L16 6"/> }.into_any(),
        IconKind::Profiles => view! { <path d="M5 6h14M5 12h14M5 18h9"/> }.into_any(),
        IconKind::Batch => view! { <rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/> }.into_any(),
        IconKind::Compare => view! { <path d="M8 4v16M16 4v16M4 8h4M16 16h4"/> }.into_any(),
        IconKind::Settings => view! { <circle cx="12" cy="12" r="3"/><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1"/> }.into_any(),
        IconKind::Health => view! { <path d="M3 12h4l2-5 4 10 2-5h6"/> }.into_any(),
        IconKind::About => view! { <circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8v.01"/> }.into_any(),
        IconKind::Lock => view! { <rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/> }.into_any(),
    };
    view! {
        <svg class="nd-icon" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor"
            stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {body}
        </svg>
    }
}
