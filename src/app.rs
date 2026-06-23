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
use crate::pages::setup_wizard::SetupWizard;
use crate::theme::{apply_theme, normalize_theme, ThemeContext};

/// Shared context for feature flags, reactive so UI updates on toggle.
#[derive(Clone)]
pub struct FeatureFlagsContext {
    pub flags: ReadSignal<FeatureFlags>,
    pub set_flags: WriteSignal<FeatureFlags>,
}

#[component]
pub fn App() -> impl IntoView {
    let (theme, set_theme) = signal(String::from("bambu"));
    provide_context(ThemeContext { theme, set_theme });

    // Feature flags with both enabled by default
    let (flags, set_flags) = signal(FeatureFlags {
        profiles_enabled: true,
        analysis_enabled: true,
    });
    provide_context(FeatureFlagsContext { flags, set_flags });

    // Setup wizard state: None = loading, Some(true) = complete, Some(false) = show wizard
    let setup_complete = RwSignal::new(Option::<bool>::None);

    // Load saved theme and feature flags on mount, and check setup status
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(Some(saved)) = commands::get_preference("theme").await {
                set_theme.set(normalize_theme(&saved).to_string());
            }
            if let Ok(loaded_flags) = commands::get_feature_flags().await {
                set_flags.set(loaded_flags);
            }
            // Check setup status
            match commands::check_setup_complete().await {
                Ok(status) => setup_complete.set(Some(status.setup_complete)),
                Err(_) => setup_complete.set(Some(false)),
            }
        });
    });

    // Apply theme to DOM whenever the signal changes
    Effect::new(move |_| {
        let t = theme.get();
        apply_theme(&t);
    });

    let on_wizard_complete = Callback::new(move |()| {
        setup_complete.set(Some(true));
    });

    let on_wizard_cancel = Callback::new(move |()| {
        setup_complete.set(Some(true));
        spawn_local(async move {
            let _ = commands::set_preference("setup_complete", "true").await;
        });
    });

    view! {
        <Router>
            // Show wizard overlay if setup is not complete
            <Show when=move || setup_complete.get() == Some(false)>
                <SetupWizard on_complete=on_wizard_complete.clone() on_cancel=on_wizard_cancel.clone() />
            </Show>

            // Show main app when setup is complete (or still loading)
            <Show when=move || setup_complete.get() != Some(false)>
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
            </Show>
        </Router>
    }
}
