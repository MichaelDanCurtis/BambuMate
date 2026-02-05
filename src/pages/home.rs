use leptos::prelude::*;

#[component]
pub fn HomePage() -> impl IntoView {
    view! {
        <div class="page home-page">
            <h2>"Welcome to BambuMate"</h2>
            <p class="page-description">
                "Optimize your Bambu Studio filament profiles with AI-powered analysis."
            </p>

            <div class="card-grid">
                <div class="card">
                    <h3>"Search Filament"</h3>
                    <p>"Find filament specs and generate optimized profiles"</p>
                    <button class="btn btn-primary" disabled=true>"Coming Soon"</button>
                </div>
                <div class="card">
                    <h3>"Analyze Print"</h3>
                    <p>"Upload a photo for AI defect analysis and recommendations"</p>
                    <button class="btn btn-primary" disabled=true>"Coming Soon"</button>
                </div>
                <div class="card">
                    <h3>"View Profiles"</h3>
                    <p>"Browse and manage your generated filament profiles"</p>
                    <button class="btn btn-primary" disabled=true>"Coming Soon"</button>
                </div>
            </div>
        </div>
    }
}
