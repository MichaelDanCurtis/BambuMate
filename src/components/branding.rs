use leptos::prelude::*;

#[component]
pub fn BrandMark() -> impl IntoView {
    view! {
        <div class="brand-mark" aria-hidden="true">
            // Flat fills only (Nothing rules: no gradients). The tile takes
            // currentColor, which .brand-mark sets to --nd-signal.
            <svg viewBox="0 0 96 96" class="brand-mark-svg">
                <rect x="10" y="10" width="76" height="76" rx="24" fill="currentColor" />
                <path d="M28 30h18c11.6 0 20 6.8 20 17.1 0 10.9-9.3 18.9-22 18.9H28V30Zm14.9 27.4c8.2 0 13.7-3.8 13.7-10.5 0-6.2-4.9-9.7-12.9-9.7h-6.5v20.2h5.7Z" fill="#06273a" />
                <path d="M54.5 29.5h13.8l-8.2 16.6 8.7 19.9H57.2l-4.6-11.4-5.2 11.4H36.2l10-19.8-8.8-16.7h12.1l4 9.4 5-9.4Z" fill="#ffffff" />
                <path d="M30 73c8.8-7.6 17.8-11.4 27-11.4 7.1 0 12.5 1.6 16.3 4.7" fill="none" stroke="rgba(255,255,255,0.52)" stroke-width="4" stroke-linecap="round" />
            </svg>
        </div>
    }
}
