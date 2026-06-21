use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, FeatureFlags};
use crate::components::sidebar::Sidebar;
use crate::pages::batch_generate::BatchGeneratePage;
use crate::pages::filament_search::FilamentSearchPage;
use crate::pages::health::HealthPage;
use crate::pages::home::HomePage;
use crate::pages::print_analysis::PrintAnalysisPage;
use crate::pages::profile_diff::ProfileDiffPage;
use crate::pages::profile_management::ProfileManagementPage;
use crate::pages::settings::SettingsPage;
use crate::theme::{apply_theme, ThemeContext};

/// Shared context for feature flags, reactive so UI updates on toggle.
#[derive(Clone)]
pub struct FeatureFlagsContext {
    pub flags: ReadSignal<FeatureFlags>,
    pub set_flags: WriteSignal<FeatureFlags>,
}

#[component]
pub fn App() -> impl IntoView {
    let (theme, set_theme) = signal(String::from("system"));
    provide_context(ThemeContext { theme, set_theme });

    // Feature flags with both enabled by default
    let (flags, set_flags) = signal(FeatureFlags {
        profiles_enabled: true,
        analysis_enabled: true,
    });
    provide_context(FeatureFlagsContext { flags, set_flags });

    // Load saved theme and feature flags on mount
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(Some(saved)) = commands::get_preference("theme").await {
                set_theme.set(saved);
            }
            if let Ok(loaded_flags) = commands::get_feature_flags().await {
                set_flags.set(loaded_flags);
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
                        <Route path=path!("/profiles") view=ProfileManagementPage />
                        <Route path=path!("/batch") view=BatchGeneratePage />
                        <Route path=path!("/compare") view=ProfileDiffPage />
                        <Route path=path!("/settings") view=SettingsPage />
                        <Route path=path!("/health") view=HealthPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
