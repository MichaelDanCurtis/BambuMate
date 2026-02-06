use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;
use wasm_bindgen_futures::spawn_local;

use crate::commands;
use crate::components::sidebar::Sidebar;
use crate::pages::filament_search::FilamentSearchPage;
use crate::pages::health::HealthPage;
use crate::pages::home::HomePage;
use crate::pages::print_analysis::PrintAnalysisPage;
use crate::pages::settings::SettingsPage;
use crate::theme::{apply_theme, ThemeContext};

#[component]
pub fn App() -> impl IntoView {
    let (theme, set_theme) = signal(String::from("system"));
    provide_context(ThemeContext { theme, set_theme });

    // Load saved theme preference on mount
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(Some(saved)) = commands::get_preference("theme").await {
                set_theme.set(saved);
            }
        });
    });

    // Apply theme to DOM whenever the signal changes
    Effect::new(move |_| {
        let t = theme.get();
        apply_theme(&t);
    });

    view! {
        <Router>
            <div class="app-layout">
                <Sidebar />
                <main class="content">
                    <Routes fallback=|| view! { <p>"Page not found"</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/filament") view=FilamentSearchPage />
                        <Route path=path!("/analysis") view=PrintAnalysisPage />
                        <Route path=path!("/settings") view=SettingsPage />
                        <Route path=path!("/health") view=HealthPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
