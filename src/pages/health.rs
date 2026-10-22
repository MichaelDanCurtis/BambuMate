use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, HealthReport};
use crate::components::diagnostics_panel::DiagnosticsPanel;
use crate::components::health_hero::HealthHero;
use crate::components::status_badge::{CheckStatus, StatusBadge};

#[component]
pub fn HealthPage() -> impl IntoView {
    let (checking, set_checking) = signal(false);
    // Shared with the hero, which runs the first check on mount.
    let report = RwSignal::new(None::<HealthReport>);
    let ran_at = RwSignal::new(None::<f64>);
    let (error, set_error) = signal::<Option<String>>(None);

    let do_health_check = move || {
        set_checking.set(true);
        set_error.set(None);
        spawn_local(async move {
            match commands::run_health_check().await {
                Ok(r) => {
                    report.set(Some(r));
                    ran_at.set(Some(js_sys::Date::now()));
                }
                Err(e) => {
                    set_error.set(Some(format!("Health check failed: {}", e)));
                }
            }
            set_checking.set(false);
        });
    };

    let run_check = move |_| {
        do_health_check();
    };

    view! {
        <div class="page health-page">
            <HealthHero report=report ran_at=ran_at />

            <button
                class="btn btn-primary"
                on:click=run_check
                disabled=move || checking.get()
            >
                {move || if checking.get() { "Checking..." } else { "Run Health Check" }}
            </button>

            {move || {
                error.get().map(|e| {
                    view! {
                        <div class="health-error">
                            <span class="status-text status-error">{e}</span>
                        </div>
                    }
                })
            }}

            {move || {
                report.get().map(|r| {
                    let bs_status = if r.bambu_studio_installed { CheckStatus::Pass } else { CheckStatus::Fail };
                    let bs_detail = r.bambu_studio_path.clone().unwrap_or_else(|| "Not found".to_string());

                    let pd_status = if r.profile_dir_accessible { CheckStatus::Pass } else { CheckStatus::Fail };
                    let pd_detail = r.profile_dir_path.clone().unwrap_or_else(|| "Not found".to_string());

                    let claude_status = if r.claude_api_key_set { CheckStatus::Pass } else { CheckStatus::Unknown };
                    let claude_detail = if r.claude_api_key_set { "Configured".to_string() } else { "Not set (optional)".to_string() };

                    let openai_status = if r.openai_api_key_set { CheckStatus::Pass } else { CheckStatus::Unknown };
                    let openai_detail = if r.openai_api_key_set { "Configured".to_string() } else { "Not set (optional)".to_string() };

                    let kimi_status = if r.kimi_api_key_set { CheckStatus::Pass } else { CheckStatus::Unknown };
                    let kimi_detail = if r.kimi_api_key_set { "Configured".to_string() } else { "Not set (optional)".to_string() };

                    let openrouter_status = if r.openrouter_api_key_set { CheckStatus::Pass } else { CheckStatus::Unknown };
                    let openrouter_detail = if r.openrouter_api_key_set { "Configured".to_string() } else { "Not set (optional)".to_string() };

                    view! {
                        <div class="health-results">
                            <StatusBadge label="Bambu Studio" status=bs_status detail=bs_detail />
                            <StatusBadge label="Profile Directory" status=pd_status detail=pd_detail />
                            <StatusBadge label="Claude API key (optional)" status=claude_status detail=claude_detail />
                            <StatusBadge label="OpenAI API key (optional)" status=openai_status detail=openai_detail />
                            <StatusBadge label="Kimi K2 API key (optional)" status=kimi_status detail=kimi_detail />
                            <StatusBadge label="OpenRouter API key (optional)" status=openrouter_status detail=openrouter_detail />
                        </div>
                    }
                })
            }}

            <DiagnosticsPanel />
        </div>
    }
}
