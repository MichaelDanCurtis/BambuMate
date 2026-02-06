use leptos::prelude::*;

#[component]
pub fn Sidebar() -> impl IntoView {
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
                <li class="nav-item">
                    <a href="/filament" class="nav-link">"Filament Search"</a>
                </li>
                <li class="nav-item">
                    <a href="/analysis" class="nav-link">"Print Analysis"</a>
                </li>
                <li class="nav-item">
                    <a href="/profiles" class="nav-link">"Profiles"</a>
                </li>
                <li class="nav-item">
                    <a href="/settings" class="nav-link">"Settings"</a>
                </li>
                <li class="nav-item">
                    <a href="/health" class="nav-link">"Health Check"</a>
                </li>
            </ul>
        </nav>
    }
}
