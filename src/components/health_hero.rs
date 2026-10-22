//! The Health summary as a hero: runs the health check on mount and leads
//! Settings · General and Settings · Health with it.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, HealthReport};
use crate::components::patterns::{Hero, StateKind, StateText, Tone};
use crate::pages::health_view::{health_detail, health_summary};
use crate::pages::home_state::format_ago;

#[component]
pub fn HealthHero(
    /// The report, shared with the page so it can list each check.
    #[prop(optional)]
    report: Option<RwSignal<Option<HealthReport>>>,
    /// When the report was last produced (ms), so a re-run on the page keeps
    /// "last run" accurate.
    #[prop(optional)]
    ran_at: Option<RwSignal<Option<f64>>>,
) -> impl IntoView {
    let report = report.unwrap_or_else(|| RwSignal::new(None));
    let ran_at = ran_at.unwrap_or_else(|| RwSignal::new(None));
    let error = RwSignal::new(None::<String>);
    spawn_local(async move {
        match commands::run_health_check().await {
            Ok(r) => {
                report.try_set(Some(r));
                ran_at.try_set(Some(js_sys::Date::now()));
            }
            Err(e) => {
                error.try_set(Some(e));
            }
        }
    });
    view! {
        <div class="health-hero">
            {move || match (report.get(), error.get()) {
                (Some(r), _) => {
                    let s = health_summary(&r);
                    let ago = ran_at
                        .get()
                        .map(|t| format_ago(js_sys::Date::now(), t))
                        .unwrap_or_default();
                    view! {
                        <Hero label="System" value=s.word.clone() detail=health_detail(&s, &ago) tone=s.tone />
                    }
                    .into_any()
                }
                (None, Some(e)) => view! {
                    <StateText kind=StateKind::Error message=format!("Health check failed: {e}") />
                }
                .into_any(),
                (None, None) => view! { <Hero label="System" value="…" tone=Tone::Neutral /> }.into_any(),
            }}
        </div>
    }
}
