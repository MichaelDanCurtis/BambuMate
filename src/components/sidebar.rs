use leptos::prelude::*;

use crate::app::FeatureFlagsContext;
use crate::components::stl_indicator::StlIndicator;

#[component]
pub fn Sidebar() -> impl IntoView {
    let ff_ctx = use_context::<FeatureFlagsContext>().expect("FeatureFlagsContext not provided");

    view! {
        <nav class="sidebar">
            <div class="sidebar-header">
                <h1 class="sidebar-title">"BambuMate"</h1>
                <p class="sidebar-subtitle">"Filament Profile Manager"</p>
            </div>
            <ul class="nav-list">
                <li class="nav-item">
                    <a href="/" class="nav-link">"Home"</a>
                </li>
                <Show when=move || ff_ctx.flags.get().profiles_enabled>
                    <li class="nav-item">
                        <a href="/filament" class="nav-link">"Create Profile"</a>
                    </li>
                </Show>
                <li class="nav-item">
                    <a href="/analysis" class="nav-link">"Print Analysis"</a>
                </li>
                <Show when=move || ff_ctx.flags.get().profiles_enabled>
                    <li class="nav-item">
                        <a href="/profiles" class="nav-link">"Profiles"</a>
                    </li>
                    <li class="nav-item">
                        <a href="/batch" class="nav-link">"Batch Generate"</a>
                    </li>
                    <li class="nav-item">
                        <a href="/compare" class="nav-link">"Compare Profiles"</a>
                    </li>
                </Show>
                <li class="nav-item">
                    <a href="/settings" class="nav-link">"Settings"</a>
                </li>
                <li class="nav-item">
                    <a href="/health" class="nav-link">"Health Check"</a>
                </li>
            </ul>
            <div class="sidebar-footer">
                <StlIndicator />
            </div>
        </nav>
    }
}
