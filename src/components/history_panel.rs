//! History panel showing refinement sessions.
//!
//! Displays past analysis sessions for a profile with ability to revert
//! to previous states via backup restoration.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, SessionSummary};

/// History panel component for viewing and managing refinement sessions.
///
/// Shows a list of past analysis sessions for a specific profile,
/// with the ability to revert applied changes back to their previous state.
#[component]
pub fn HistoryPanel(
    /// Path to the profile to show history for.
    profile_path: String,
    /// Callback invoked when user clicks Revert on a session.
    on_revert: Callback<i64>,
) -> impl IntoView {
    let (sessions, set_sessions) = signal::<Option<Vec<SessionSummary>>>(None);
    let (loading, set_loading) = signal(true);
    let (error, set_error) = signal::<Option<String>>(None);

    // Load sessions on mount
    {
        let path = profile_path.clone();
        spawn_local(async move {
            match commands::list_history_sessions(&path).await {
                Ok(s) => set_sessions.set(Some(s)),
                Err(e) => set_error.set(Some(e)),
            }
            set_loading.set(false);
        });
    }

    view! {
        <div class="history-panel">
            <style>{include_str!("history_panel.css")}</style>
            <h4 class="history-title">"Refinement History"</h4>

            {move || {
                if loading.get() {
                    view! { <p class="history-loading">"Loading history..."</p> }.into_any()
                } else if let Some(err) = error.get() {
                    view! { <p class="history-error">{err}</p> }.into_any()
                } else if let Some(sessions) = sessions.get() {
                    if sessions.is_empty() {
                        view! { <p class="history-empty">"No history for this profile yet."</p> }.into_any()
                    } else {
                        view! {
                            <div class="history-list">
                                {sessions.iter().map(|s| {
                                    let id = s.id;
                                    let date = s.created_at.clone();
                                    let applied = s.was_applied;
                                    view! {
                                        <div class="history-item">
                                            <div class="history-item-info">
                                                <span class="history-date">{date}</span>
                                                <span class=format!("history-status {}", if applied { "status-applied" } else { "status-analyzed" })>
                                                    {if applied { "Applied" } else { "Analyzed" }}
                                                </span>
                                            </div>
                                            {applied.then(|| view! {
                                                <button
                                                    class="btn btn-small btn-secondary"
                                                    on:click=move |_| on_revert.run(id)
                                                >
                                                    "Revert"
                                                </button>
                                            })}
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                } else {
                    view! { <p class="history-empty">"No data"</p> }.into_any()
                }
            }}
        </div>
    }
}
