use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;

use crate::components::sidebar::Sidebar;
use crate::pages::health::HealthPage;
use crate::pages::home::HomePage;
use crate::pages::settings::SettingsPage;

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <div class="app-layout">
                <Sidebar />
                <main class="content">
                    <Routes fallback=|| view! { <p>"Page not found"</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/settings") view=SettingsPage />
                        <Route path=path!("/health") view=HealthPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
