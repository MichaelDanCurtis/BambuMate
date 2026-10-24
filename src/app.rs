use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;
use leptos_router::NavigateOptions;
use wasm_bindgen_futures::spawn_local;

use crate::advisor::PendingCatalogPick;
use crate::agent::drawer::{AgentDrawer, AgentEvents, AgentRefresh, AgentShared};
use crate::commands::{self, FeatureFlags, UpdateInfo};
use crate::components::patterns::{StateKind, StateText};
use crate::components::sidebar::Sidebar;
use crate::journal::{JournalEvents, JournalShared};
use crate::pages::about::AboutPage;
use crate::pages::advise::AdvisePage;
use crate::pages::batch_generate::BatchGeneratePage;
use crate::pages::filament_search::FilamentSearchPage;
use crate::pages::health::HealthPage;
use crate::pages::home::HomePage;
use crate::pages::hub::HubPage;
use crate::pages::print_analysis::PrintAnalysisPage;
use crate::pages::printer::PrinterPage;
use crate::pages::profile_diff::ProfileDiffPage;
use crate::pages::profile_management::ProfileManagementPage;
use crate::pages::settings::SettingsPage;
use crate::pages::setup_wizard::SetupWizard;
use crate::pages::slice::SlicePage;
use crate::printer::{PrinterEvents, PrinterShared};
use crate::routes::{legacy_target, FILAMENT_TABS, PROFILES_TABS, SETTINGS_TABS};
use crate::slicer::{SlicerEvents, SlicerShared};
use crate::theme::{apply_theme, normalize_theme, ThemeContext};

/// Shared context for feature flags, reactive so UI updates on toggle.
#[derive(Clone)]
pub struct FeatureFlagsContext {
    pub flags: ReadSignal<FeatureFlags>,
    pub set_flags: WriteSignal<FeatureFlags>,
}

/// Shared context for the startup update check result.
#[derive(Clone)]
pub struct UpdateContext {
    pub update_info: RwSignal<Option<UpdateInfo>>,
}

/// What the router shows when no route matches: a route from before the
/// hubs is replaced in history by the tab it moved to, so Back never lands
/// on it and bounces; anything else is "not found".
///
/// The redirect waits a tick. Router navigations finish only after the
/// destination renders, so a `<Redirect>` rendered here navigates before
/// the click that brought us here has been pushed, and replaces the page the
/// user came from instead.
#[component]
fn NoRoute() -> impl IntoView {
    let path = use_location().pathname.get_untracked();
    match legacy_target(&path) {
        Some(to) => {
            let navigate = use_navigate();
            set_timeout(
                move || {
                    navigate(
                        to,
                        NavigateOptions {
                            replace: true,
                            ..Default::default()
                        },
                    )
                },
                std::time::Duration::ZERO,
            );
            ().into_any()
        }
        None => view! {
            <div class="page">
                <StateText kind=StateKind::Empty message="Page not found" />
                <a href="/" class="btn btn-secondary">"Back to Home"</a>
            </div>
        }
        .into_any(),
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (theme, set_theme) = signal(String::from("dark"));
    provide_context(ThemeContext { theme, set_theme });

    let update_info = RwSignal::new(Option::<UpdateInfo>::None);
    provide_context(UpdateContext { update_info });
    provide_context(AgentRefresh(RwSignal::new(0)));
    provide_context(AgentShared::default());
    provide_context(PrinterShared::new());
    provide_context(JournalShared::new());
    provide_context(SlicerShared::default());
    provide_context(PendingCatalogPick::new());

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
            // Run update check in background; errors are silently ignored on startup
            if let Ok(info) = commands::check_for_updates().await {
                update_info.set(Some(info));
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
        // Refresh feature flags: the newly-selected model determines whether
        // analysis is available.
        spawn_local(async move {
            if let Ok(flags) = commands::get_feature_flags().await {
                set_flags.set(flags);
            }
        });
    });

    let on_wizard_cancel = Callback::new(move |()| {
        setup_complete.set(Some(true));
        spawn_local(async move {
            if let Ok(flags) = commands::get_feature_flags().await {
                set_flags.set(flags);
            }
        });
    });

    view! {
        <Router>
            // Mounted once, outside the Shows below: its listeners can't be removed.
            <AgentEvents />
            <PrinterEvents />
            <JournalEvents />
            <SlicerEvents />

            // Show wizard overlay if setup is not complete
            <Show when=move || setup_complete.get() == Some(false)>
                <SetupWizard on_complete=on_wizard_complete.clone() on_cancel=on_wizard_cancel.clone() />
            </Show>

            // Show main app when setup is complete (or still loading)
            <Show when=move || setup_complete.get() != Some(false)>
                <div class="app-layout">
                    <Sidebar />
                    <main class="content">
                        <Routes fallback=|| view! { <NoRoute /> }>
                            <Route path=path!("/") view=HomePage />
                            <Route path=path!("/printer") view=PrinterPage />
                            <ParentRoute path=path!("/filament") view=|| view! { <HubPage title="Filament" tabs=FILAMENT_TABS /> }>
                                <Route path=path!("") view=FilamentSearchPage />
                                <Route path=path!("advise") view=AdvisePage />
                                <Route path=path!("batch") view=BatchGeneratePage />
                            </ParentRoute>
                            <ParentRoute path=path!("/profiles") view=|| view! { <HubPage title="Profiles" tabs=PROFILES_TABS /> }>
                                <Route path=path!("") view=ProfileManagementPage />
                                <Route path=path!("compare") view=ProfileDiffPage />
                            </ParentRoute>
                            <Route path=path!("/slice") view=SlicePage />
                            <Route path=path!("/analysis") view=PrintAnalysisPage />
                            <ParentRoute path=path!("/settings") view=|| view! { <HubPage title="Settings" tabs=SETTINGS_TABS /> }>
                                <Route path=path!("") view=SettingsPage />
                                <Route path=path!("health") view=HealthPage />
                                <Route path=path!("about") view=AboutPage />
                            </ParentRoute>
                        </Routes>
                    </main>
                    <AgentDrawer />
                </div>
            </Show>
        </Router>
    }
}
