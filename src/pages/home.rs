use leptos::prelude::*;

use crate::app::FeatureFlagsContext;

#[component]
pub fn HomePage() -> impl IntoView {
    let ff_ctx = use_context::<FeatureFlagsContext>().expect("FeatureFlagsContext not provided");

    view! {
        <div class="page home-page">
            <h2>"Welcome to BambuMate"</h2>
            <p class="page-description">
                "Optimize your Bambu Studio filament profiles with AI-powered analysis."
            </p>

            <div class="card-grid">
                <Show when=move || ff_ctx.flags.get().profiles_enabled>
                    <div class="card">
                        <h3>"Search Filament"</h3>
                        <p>"Find filament specs and generate optimized profiles"</p>
                        <a href="/filament" class="btn btn-primary">"Search Now"</a>
                    </div>
                </Show>
                <Show when=move || ff_ctx.flags.get().analysis_enabled>
                    <div class="card">
                        <h3>"Analyze Print"</h3>
                        <p>"Upload a photo for AI defect analysis and recommendations"</p>
                        <a href="/analysis" class="btn btn-primary">"Analyze Now"</a>
                    </div>
                </Show>
                <Show when=move || ff_ctx.flags.get().profiles_enabled>
                    <div class="card">
                        <h3>"View Profiles"</h3>
                        <p>"Browse and manage your generated filament profiles"</p>
                        <a href="/profiles" class="btn btn-primary">"Browse Profiles"</a>
                    </div>
                </Show>
            </div>

            <div class="how-it-works">
                <h3>"How It Works"</h3>
                <div class="steps">
                    <Show when=move || ff_ctx.flags.get().profiles_enabled>
                        <div class="step">
                            <span class="step-number">"1"</span>
                            <div class="step-content">
                                <strong>"Search"</strong>
                                <p>"Find your filament from our catalog or let AI look it up"</p>
                            </div>
                        </div>
                        <div class="step">
                            <span class="step-number">"2"</span>
                            <div class="step-content">
                                <strong>"Generate"</strong>
                                <p>"We create an optimized Bambu Studio profile from the specs"</p>
                            </div>
                        </div>
                    </Show>
                    <Show when=move || ff_ctx.flags.get().analysis_enabled>
                        <div class="step">
                            <span class="step-number">{move || {
                                let profile_steps = if ff_ctx.flags.get().profiles_enabled { 2 } else { 0 };
                                format!("{}", profile_steps + 1)
                            }}</span>
                            <div class="step-content">
                                <strong>"Refine"</strong>
                                <p>"Print a test, photograph it, and get AI-powered tuning suggestions"</p>
                            </div>
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    }
}
