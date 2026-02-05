use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, HealthReport};
use crate::components::status_badge::{CheckStatus, StatusBadge};

#[component]
pub fn HealthPage() -> impl IntoView {
    let (checking, set_checking) = signal(false);
    let (report, set_report) = signal::<Option<HealthReport>>(None);
    let (error, set_error) = signal::<Option<String>>(None);

    let do_health_check = move || {
        set_checking.set(true);
        set_error.set(None);
        spawn_local(async move {
            match commands::run_health_check().await {
                Ok(r) => {
                    set_report.set(Some(r));
                }
                Err(e) => {
                    set_error.set(Some(format!("Health check failed: {}", e)));
                }
            }
            set_checking.set(false);
        });
    };

    // Auto-run health check on mount
    let auto_check = do_health_check.clone();
    Effect::new(move |_| {
        auto_check();
    });

    let run_check = move |_| {
        do_health_check();
    };

    view! {
        <div class="page health-page">
            <h2>"Health Check"</h2>
            <p class="page-description">
                "Verify that BambuMate can access all required services and directories."
            </p>

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
                    let passed = [
                        r.bambu_studio_installed,
                        r.profile_dir_accessible,
                        r.claude_api_key_set,
                        r.openai_api_key_set,
                    ].iter().filter(|&&v| v).count();

                    let bs_status = if r.bambu_studio_installed { CheckStatus::Pass } else { CheckStatus::Fail };
                    let bs_detail = r.bambu_studio_path.clone().unwrap_or_else(|| "Not found".to_string());

                    let pd_status = if r.profile_dir_accessible { CheckStatus::Pass } else { CheckStatus::Fail };
                    let pd_detail = r.profile_dir_path.clone().unwrap_or_else(|| "Not found".to_string());

                    let claude_status = if r.claude_api_key_set { CheckStatus::Pass } else { CheckStatus::Fail };
                    let claude_detail = if r.claude_api_key_set { "Configured".to_string() } else { "Not configured".to_string() };

                    let openai_status = if r.openai_api_key_set { CheckStatus::Pass } else { CheckStatus::Fail };
                    let openai_detail = if r.openai_api_key_set { "Configured".to_string() } else { "Not configured".to_string() };

                    let summary_class = if passed == 4 { "summary-all-pass" } else if passed == 0 { "summary-all-fail" } else { "summary-partial" };

                    view! {
                        <div class="health-results">
                            <StatusBadge label="Bambu Studio" status=bs_status detail=bs_detail />
                            <StatusBadge label="Profile Directory" status=pd_status detail=pd_detail />
                            <StatusBadge label="Claude API Key" status=claude_status detail=claude_detail />
                            <StatusBadge label="OpenAI API Key" status=openai_status detail=openai_detail />

                            <div class={format!("health-summary {}", summary_class)}>
                                {format!("{} of 4 checks passed", passed)}
                            </div>
                        </div>
                    }
                })
            }}
        </div>
    }
}
