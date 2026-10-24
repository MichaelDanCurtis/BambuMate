//! Print analysis page for AI-powered defect detection.
//!
//! Users can drag-and-drop or browse for a print photo to analyze.

use leptos::html::Div;
use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::app::FeatureFlagsContext;
use crate::commands;
use crate::components::change_preview::ChangePreview;
use crate::components::defect_report::DefectReportDisplay;
use crate::components::history_panel::HistoryPanel;
use crate::components::patterns::{Hero, PageHeader, StateKind, StateText};
use crate::journal::bridge as journal_bridge;
use crate::journal::format::{analyze_prefill, data_url_base64, entry_href, NAME_MAX};
use crate::pages::analysis_view::{analysis_detail, quality_tone, quality_word};
use crate::pages::slice::file_name;
use crate::printer::types::untrusted_text;
use crate::slicer::state::newest_source;
use crate::slicer::SlicerShared;
use crate::viewport::ModelViewport;

/// Request payload for print analysis.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeRequest {
    pub image_base64: String,
    pub profile_path: Option<String>,
    pub material_type: Option<String>,
}

/// Detected defect from AI analysis.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DetectedDefect {
    pub defect_type: String,
    pub severity: f32,
    pub confidence: f32,
}

/// Defect report from vision API.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DefectReport {
    pub defects: Vec<DetectedDefect>,
    pub overall_quality: String,
    pub notes: Option<String>,
}

/// Conflict between recommendations.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Conflict {
    pub parameter: String,
    pub conflicting_defects: Vec<String>,
    pub description: String,
}

/// Parameter recommendation with display info.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RecommendationDisplay {
    pub defect: String,
    pub parameter: String,
    pub parameter_label: String,
    pub current_value: f32,
    pub recommended_value: f32,
    pub change_display: String,
    pub unit: String,
    pub priority: u8,
    pub rationale: String,
    pub was_clamped: bool,
}

/// Full analysis response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnalyzeResponse {
    pub defect_report: DefectReport,
    pub recommendations: Vec<RecommendationDisplay>,
    pub conflicts: Vec<Conflict>,
    pub current_values: HashMap<String, f32>,
    pub material_type: String,
    /// Session ID for apply flow (None if history recording failed)
    pub session_id: Option<i64>,
}

/// Analysis state enum.
#[derive(Debug, Clone)]
pub enum AnalysisState {
    /// Waiting for image upload
    Idle,
    /// Image loaded, ready to analyze
    Ready(String), // base64 image
    /// Analysis in progress
    Analyzing,
    /// Analysis complete with results
    Complete(AnalyzeResponse),
    /// Error occurred
    Error(String),
}

/// The material choices: value, then label.
const MATERIAL_OPTIONS: &[(&str, &str)] = &[
    ("", "Auto-detect"),
    ("PLA", "PLA"),
    ("PETG", "PETG"),
    ("ABS", "ABS"),
    ("ASA", "ASA"),
    ("TPU", "TPU"),
    ("PA", "Nylon (PA)"),
    ("PC", "Polycarbonate (PC)"),
];

/// Said when `?journal=<id>` names an entry that has been deleted.
const ENTRY_GONE: &str = "That print isn't in the journal any more.";

/// The entry id in `?journal=<id>`; anything else means no entry.
fn journal_param(raw: Option<&str>) -> Option<i64> {
    raw.and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
}

/// The path of the profile named like a slice's filament preset. Only an
/// exact, single match counts: with two of the same name there is no telling
/// which the print used, so none is chosen.
fn preset_profile(profiles: &[commands::ProfileInfo], preset: &str) -> Option<String> {
    let mut named = profiles.iter().filter(|p| p.name == preset);
    match (named.next(), named.next()) {
        (Some(only), None) => Some(only.path.clone()),
        _ => None,
    }
}

/// Print analysis page component.
#[component]
pub fn PrintAnalysisPage() -> impl IntoView {
    let ff_ctx = use_context::<FeatureFlagsContext>().expect("FeatureFlagsContext not provided");
    let (state, set_state) = signal(AnalysisState::Idle);
    let (image_preview, set_image_preview) = signal::<Option<String>>(None);
    let (material_override, set_material_override) = signal::<Option<String>>(None);
    let (profile_path, set_profile_path) = signal::<Option<String>>(None);
    // The optional model whose 3D view and geometry risks sit beside the
    // analysis. `model_gen` counts resets, so a file picked in a dialog that
    // was still open when the page was reset is dropped.
    let model = RwSignal::new(None::<String>);
    let model_gen = RwSignal::new(0u32);
    let jobs = expect_context::<SlicerShared>().jobs;
    let last_model = Memo::new(move |_| jobs.with(|js| newest_source(js).map(str::to_string)));
    let has_model = Memo::new(move |_| model.with(Option::is_some));
    // Profile list for dropdown
    let (profiles, set_profiles) = signal::<Vec<commands::ProfileInfo>>(Vec::new());
    let (profiles_loading, set_profiles_loading) = signal(true);
    // Apply flow state
    let (show_apply_dialog, set_show_apply_dialog) = signal(false);
    let (current_session_id, set_current_session_id) = signal::<Option<i64>>(None);
    let (apply_message, set_apply_message) = signal::<Option<String>>(None);
    // History/revert state
    let (revert_message, set_revert_message) = signal::<Option<String>>(None);
    let (history_key, set_history_key) = signal(0u32);
    // Opened from a journal entry (`?journal=<id>`): its newest photo, model,
    // material and filament preset are filled in, and the analysis is linked
    // back to the entry. `journal_gen` counts opens and resets, so the answer
    // to an older open can't land over a newer one. `analyze_gen` counts the
    // analyses the page has dropped.
    let journal_link = RwSignal::new(None::<(i64, String)>);
    let journal_note = RwSignal::new(None::<(StateKind, String)>);
    let pending_preset = RwSignal::new(None::<String>);
    let journal_gen = RwSignal::new(0u32);
    let analyze_gen = RwSignal::new(0u32);

    // Load profiles on mount
    spawn_local(async move {
        match commands::list_profiles().await {
            Ok(p) => set_profiles.set(p),
            Err(e) => web_sys::console::error_1(&format!("Failed to load profiles: {}", e).into()),
        }
        set_profiles_loading.set(false);
    });

    // Handle analyze button click
    let on_analyze = move |_| {
        let current_state = state.get();
        if let AnalysisState::Ready(base64) = current_state {
            set_state.set(AnalysisState::Analyzing);

            let material = material_override.get();
            let path = profile_path.get();
            let journal = journal_link.with_untracked(|j| j.as_ref().map(|(id, _)| *id));
            let started = analyze_gen.get_untracked();
            spawn_local(async move {
                let result = commands::analyze_print(base64, path, material, journal).await;
                // An answer for a photo the page has since dropped is ignored.
                if analyze_gen.try_get_untracked() != Some(started) {
                    return;
                }
                match result {
                    Ok(response) => {
                        // Store session ID for apply flow
                        set_current_session_id.try_set(response.session_id);
                        set_state.try_set(AnalysisState::Complete(response));
                    }
                    Err(e) => {
                        set_state.try_set(AnalysisState::Error(e));
                    }
                }
            });
        }
    };

    let on_pick_model = move |_| {
        let started = model_gen.get_untracked();
        spawn_local(async move {
            if let Ok(Some(p)) = crate::slicer::bridge::pick_model().await {
                if model_gen.try_get_untracked() == Some(started) {
                    model.try_set(Some(p));
                }
            }
        });
    };

    // Reset to try another image
    // Drops the photo and everything tied to it, the journal link included:
    // the link says where this photo came from.
    let clear_photo = move || {
        model_gen.update(|g| *g = g.wrapping_add(1));
        journal_gen.update(|g| *g = g.wrapping_add(1));
        analyze_gen.update(|g| *g = g.wrapping_add(1));
        model.set(None);
        set_state.set(AnalysisState::Idle);
        set_image_preview.set(None);
        set_show_apply_dialog.set(false);
        set_current_session_id.set(None);
        set_apply_message.set(None);
        set_revert_message.set(None);
        journal_link.set(None);
        journal_note.set(None);
        pending_preset.set(None);
    };
    let on_reset = move |_| clear_photo();

    // `?journal=<id>` opens that entry, on load and whenever a link changes
    // it. Without the parameter the page is untouched.
    let query = use_query_map();
    let journal_id = Memo::new(move |_| journal_param(query.with(|q| q.get("journal")).as_deref()));
    Effect::new(move |prev: Option<Option<i64>>| {
        let id = journal_id.get();
        if prev.is_none() && id.is_none() {
            return id;
        }
        if prev.is_some() && id.is_some() {
            // Another entry's link: start from a clean page.
            clear_photo();
            set_material_override.set(None);
            set_profile_path.set(None);
        }
        journal_gen.update(|g| *g = g.wrapping_add(1));
        let generation = journal_gen.get_untracked();
        journal_link.set(None);
        journal_note.set(None);
        pending_preset.set(None);
        let Some(id) = id else {
            return id;
        };
        spawn_local(async move {
            let current = move || journal_gen.try_get_untracked() == Some(generation);
            let entry = match journal_bridge::get(id).await {
                Ok(Some(entry)) => entry,
                Ok(None) => {
                    if current() {
                        journal_note.try_set(Some((StateKind::Empty, ENTRY_GONE.to_string())));
                    }
                    return;
                }
                Err(e) => {
                    if current() {
                        journal_note.try_set(Some((StateKind::Error, e)));
                    }
                    return;
                }
            };
            if !current() {
                return;
            }
            let prefill = analyze_prefill(&entry);
            journal_link.try_set(Some((id, entry.name.clone())));
            if let Some(path) = prefill.model_path {
                model.try_set(Some(path));
            }
            if let Some(material) = prefill.material {
                set_material_override.try_set(Some(material.to_string()));
            }
            pending_preset.try_set(prefill.filament_preset);
            let Some(photo_id) = prefill.photo_id else {
                return;
            };
            let Ok(url) = journal_bridge::image(id, Some(photo_id)).await else {
                return;
            };
            if !current() {
                return;
            }
            // A photo the user picked meanwhile wins.
            let idle = state.try_with_untracked(|s| matches!(s, AnalysisState::Idle));
            if let (Some(true), Some(data)) = (idle, data_url_base64(&url)) {
                set_image_preview.try_set(Some(url));
                set_state.try_set(AnalysisState::Ready(data));
            }
        });
        Some(id)
    });
    // The entry's filament preset is chosen once the profile list is in.
    Effect::new(move |_| {
        let Some(name) = pending_preset.get() else {
            return;
        };
        if profiles_loading.get() {
            return;
        }
        let found = profiles.with(|list| preset_profile(list, &name));
        if let Some(path) = found {
            set_profile_path.set(Some(path));
        }
        pending_preset.set(None);
    });

    view! {
        <div class="page print-analysis-page">
            <style>{include_str!("print_analysis.css")}</style>

            <PageHeader title="Analyze" />
            {move || journal_link.get().map(|(id, name)| view! {
                <p class="an-journal">
                    "From the journal: "
                    <a href=entry_href(id)>{untrusted_text(&name, NAME_MAX)}</a>
                </p>
            })}
            {move || journal_note.get().map(|(kind, message)| view! {
                <div class="an-journal-note">
                    <StateText kind=kind message=message />
                </div>
            })}

            <Show
                when=move || ff_ctx.flags.get().analysis_enabled
                fallback=move || view! {
                    <div class="ai-required-notice">
                        <p class="nd-label">"AI required"</p>
                        <p>
                            "Print Analysis uses AI vision models to detect defects in your prints "
                            "and suggest parameter corrections. "
                            "To use this feature, go to "
                            <a href="/settings">"Settings"</a>
                            " and enable AI with an API key."
                        </p>
                    </div>
                }
            >
            {move || {
                let current = state.get();
                match current {
                    AnalysisState::Idle => view! {
                        <PhotoDropZone set_state=set_state set_image_preview=set_image_preview />
                    }.into_any(),

                    AnalysisState::Ready(_) => view! {
                        <div class="analysis-preview">
                            {move || image_preview.get().map(|src| view! {
                                <img src=src class="preview-image" alt="Print to analyze" />
                            })}

                            <div class="material-selector">
                                <label>"Material type (optional):"</label>
                                <select class="input" on:change=move |ev| {
                                    let value = event_target_value(&ev);
                                    if value.is_empty() {
                                        set_material_override.set(None);
                                    } else {
                                        set_material_override.set(Some(value));
                                    }
                                }>
                                    {MATERIAL_OPTIONS
                                        .iter()
                                        .map(|&(value, label)| view! {
                                            <option value=value
                                                prop:selected=move || material_override.get().as_deref().unwrap_or("") == value>
                                                {label}
                                            </option>
                                        })
                                        .collect_view()}
                                </select>
                            </div>

                            <div class="profile-selector">
                                <label>"Profile to update (optional):"</label>
                                {move || {
                                    if profiles_loading.get() {
                                        view! { <p class="loading-hint">"Loading profiles..."</p> }.into_any()
                                    } else {
                                        let profile_list = profiles.get();
                                        if profile_list.is_empty() {
                                            view! {
                                                <p class="no-profiles-hint">"No Bambu Studio profiles found. Install Bambu Studio first."</p>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <select
                                                    class="input profile-select"
                                                    on:change=move |ev| {
                                                        let value = event_target_value(&ev);
                                                        if value.is_empty() {
                                                            set_profile_path.set(None);
                                                        } else {
                                                            set_profile_path.set(Some(value));
                                                        }
                                                    }
                                                >
                                                    <option value="">"-- Select a profile --"</option>
                                                    {profile_list.iter().map(|p| {
                                                        let path = p.path.clone();
                                                        let chosen = path.clone();
                                                        let display = format!("{} ({})", p.name, p.filament_type.clone().unwrap_or_else(|| "Unknown".to_string()));
                                                        view! {
                                                            <option value=path
                                                                prop:selected=move || profile_path.get().as_deref() == Some(chosen.as_str())>
                                                                {display}
                                                            </option>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </select>
                                            }.into_any()
                                        }
                                    }
                                }}
                                <p class="input-hint">"Select a profile to enable Apply and track refinement history."</p>
                            </div>

                            <div class="an-model-pick">
                                <p class="nd-label">"Model (optional)"</p>
                                <div class="an-model-actions">
                                    <button class="btn btn-secondary an-pick-model" on:click=on_pick_model>
                                        "Choose model…"
                                    </button>
                                    {move || last_model.get().map(|path| view! {
                                        <button
                                            class="btn btn-secondary an-last-model"
                                            on:click=move |_| model.set(Some(path.clone()))
                                        >
                                            "Use last sliced model"
                                        </button>
                                    })}
                                </div>
                                {move || model.get().map(|path| view! {
                                    <div class="an-model-chosen">
                                        <span class="an-model-name" title=path.clone()>{file_name(&path)}</span>
                                        <button class="btn btn-ghost btn-sm an-model-clear" on:click=move |_| model.set(None)>
                                            "Clear"
                                        </button>
                                    </div>
                                })}
                                <p class="input-hint">"Shows the model in 3D next to the results, with where it may be hard to print."</p>
                            </div>

                            <div class="action-buttons">
                                <button class="btn btn-primary" on:click=on_analyze>
                                    "Analyze Print"
                                </button>
                                <button class="btn btn-secondary" on:click=on_reset>
                                    "Choose Different Photo"
                                </button>
                            </div>
                        </div>
                    }.into_any(),

                    AnalysisState::Analyzing => view! {
                        <div class="analyzing-state">
                            {move || image_preview.get().map(|src| view! {
                                <img src=src class="preview-image analyzing" alt="Analyzing..." />
                            })}
                            <StateText kind=StateKind::Loading message="Analyzing your print — this may take 10–30 seconds" />
                        </div>
                    }.into_any(),

                    AnalysisState::Complete(ref response) => {
                        let response = response.clone();
                        let path_for_display = profile_path.get();
                        let recs_for_dialog = response.recommendations.clone();
                        let path_for_dialog = profile_path.get().unwrap_or_default();
                        view! {
                            <div class="analysis-results">
                                <div class="an-hero-row">
                                    <Hero
                                        label="Overall quality"
                                        value=quality_word(&response.defect_report.overall_quality)
                                        detail=analysis_detail(&response.defect_report, &response.material_type)
                                        tone=quality_tone(&response.defect_report.overall_quality)
                                    />
                                    {move || image_preview.get().map(|src| view! {
                                        <img src=src class="preview-image an-photo" alt="Analyzed print" />
                                    })}
                                </div>

                                {move || has_model.get().then(|| view! {
                                    <section class="an-model">
                                        <h2 class="nd-label">"Model"</h2>
                                        <ModelViewport path=Signal::derive(move || model.get()) />
                                    </section>
                                })}

                                // Show apply success/error message
                                {move || apply_message.get().map(|msg| {
                                    let is_error = msg.starts_with("Apply failed:");
                                    view! {
                                        <div class=format!("apply-message {}", if is_error { "error" } else { "success" })>
                                            {msg}
                                        </div>
                                    }
                                })}

                                <DefectReportDisplay
                                    defect_report=response.defect_report.clone()
                                    recommendations=response.recommendations.clone()
                                    conflicts=response.conflicts.clone()
                                    material_type=response.material_type.clone()
                                    profile_path=path_for_display.clone()
                                    on_apply_click=Some(Callback::new(move |_| {
                                        set_show_apply_dialog.set(true);
                                    }))
                                />

                                <div class="action-buttons">
                                    <button class="btn btn-secondary" on:click=on_reset>
                                        "Analyze Another Photo"
                                    </button>
                                </div>

                                // Change preview dialog
                                {move || show_apply_dialog.get().then(|| {
                                    let recs = recs_for_dialog.clone();
                                    let path = path_for_dialog.clone();
                                    view! {
                                        <ChangePreview
                                            recommendations=recs
                                            profile_path=path.clone()
                                            on_apply=Callback::new(move |selected: Vec<String>| {
                                                let session = current_session_id.get();
                                                let p = path.clone();
                                                spawn_local(async move {
                                                    if let Some(sid) = session {
                                                        match commands::apply_recommendations(p.clone(), sid, selected).await {
                                                            Ok(result) => {
                                                                set_apply_message.set(Some(format!(
                                                                    "Applied {} changes. Backup: {}",
                                                                    result.changes_applied.len(),
                                                                    result.backup_path
                                                                )));
                                                                // Refresh history panel
                                                                set_history_key.update(|k| *k += 1);
                                                            }
                                                            Err(e) => {
                                                                set_apply_message.set(Some(format!("Apply failed: {}", e)));
                                                            }
                                                        }
                                                    } else {
                                                        set_apply_message.set(Some("Apply failed: No session ID available".to_string()));
                                                    }
                                                });
                                                set_show_apply_dialog.set(false);
                                            })
                                            on_cancel=Callback::new(move |_| {
                                                set_show_apply_dialog.set(false);
                                            })
                                        />
                                    }
                                })}

                                // History panel - shows past sessions and revert option
                                {path_for_display.clone().map(|path| {
                                    let on_revert = Callback::new(move |session_id: i64| {
                                        spawn_local(async move {
                                            match commands::revert_to_backup(session_id).await {
                                                Ok(msg) => {
                                                    set_revert_message.set(Some(format!("Success: {}", msg)));
                                                    set_history_key.update(|k| *k += 1);
                                                }
                                                Err(e) => {
                                                    set_revert_message.set(Some(format!("Error: {}", e)));
                                                }
                                            }
                                        });
                                    });

                                    view! {
                                        <div class="history-sidebar">
                                            {move || {
                                                let _key = history_key.get();
                                                let p = path.clone();
                                                view! {
                                                    <HistoryPanel
                                                        profile_path=p
                                                        on_revert=on_revert.clone()
                                                    />
                                                }
                                            }}
                                            {move || revert_message.get().map(|msg| {
                                                let is_error = msg.starts_with("Error:");
                                                view! {
                                                    <p class=format!("revert-message {}", if is_error { "error" } else { "success" })>
                                                        {msg}
                                                    </p>
                                                }
                                            })}
                                        </div>
                                    }
                                })}
                            </div>
                        }.into_any()
                    },

                    AnalysisState::Error(ref msg) => {
                        let msg = msg.clone();
                        view! {
                            <div class="error-state">
                                <StateText kind=StateKind::Error message=msg />
                                <button class="btn btn-secondary" on:click=on_reset>
                                    "Try Again"
                                </button>
                            </div>
                        }.into_any()
                    },
                }
            }}
            </Show>
        </div>
    }
}

/// Photo drop zone component with drag-and-drop and browse.
#[component]
fn PhotoDropZone(
    set_state: WriteSignal<AnalysisState>,
    set_image_preview: WriteSignal<Option<String>>,
) -> impl IntoView {
    let drop_zone_el = NodeRef::<Div>::new();
    let (is_over, set_is_over) = signal(false);
    let (is_loading, set_is_loading) = signal(false);
    let file_input_id = "photo-file-input";

    // Callback to process a loaded file
    let handle_file_loaded = move |(mime, base64): (String, String)| {
        set_image_preview.set(Some(format!("data:{};base64,{}", mime, base64)));
        set_state.set(AnalysisState::Ready(base64));
    };

    // Handle drop event
    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        set_is_over.set(false);

        if let Some(dt) = ev.data_transfer() {
            if let Some(files) = dt.files() {
                if let Some(file) = files.get(0) {
                    set_is_loading.set(true);
                    spawn_local(async move {
                        match read_file_as_base64(file).await {
                            Ok(loaded) => {
                                handle_file_loaded(loaded);
                            }
                            Err(e) => {
                                web_sys::console::error_1(
                                    &format!("Failed to read file: {}", e).into(),
                                );
                            }
                        }
                        set_is_loading.set(false);
                    });
                }
            }
        }
    };

    // Handle file input change
    let on_input_change = move |ev: web_sys::Event| {
        let input: web_sys::HtmlInputElement = event_target(&ev);
        if let Some(files) = input.files() {
            if let Some(file) = files.get(0) {
                set_is_loading.set(true);
                spawn_local(async move {
                    match read_file_as_base64(file).await {
                        Ok(loaded) => {
                            handle_file_loaded(loaded);
                        }
                        Err(e) => {
                            web_sys::console::error_1(
                                &format!("Failed to read file: {}", e).into(),
                            );
                        }
                    }
                    set_is_loading.set(false);
                });
            }
        }
    };

    view! {
        <div
            node_ref=drop_zone_el
            class="drop-zone"
            class:drop-zone-active=move || is_over.get()
            class:drop-zone-loading=move || is_loading.get()
            on:dragover=move |ev: web_sys::DragEvent| {
                ev.prevent_default();
                set_is_over.set(true);
            }
            on:dragleave=move |_| set_is_over.set(false)
            on:drop=on_drop
        >
            <Show
                when=move || is_loading.get()
                fallback=move || view! {
                    <div class="drop-zone-content">
                        <div class="drop-icon">"[camera]"</div>
                        <p class="drop-main">"Drop a photo of your print here"</p>
                        <p class="drop-hint">"or"</p>
                        <label for=file_input_id class="btn btn-secondary">
                            "Browse Files"
                        </label>
                        <input
                            type="file"
                            id=file_input_id
                            accept="image/*"
                            style="display: none"
                            on:change=on_input_change.clone()
                        />
                        <p class="drop-formats">"Supports JPEG, PNG, WebP"</p>
                    </div>
                }
            >
                <div class="drop-zone-loading-content">
                    <StateText kind=StateKind::Loading message="Image" />
                </div>
            </Show>
        </div>
    }
}

/// Read a File as base64, together with the image MIME type sniffed from its
/// magic bytes.
///
/// The MIME type matters for the preview `data:` URL. WKWebView (macOS)
/// validates the declared type and silently refuses to render an image whose
/// bytes do not match, whereas Chromium/WebView2 (Windows) content-sniffs and
/// renders it anyway — so hardcoding `image/jpeg` produced a blank preview for
/// PNG/WebP/HEIC screenshots on macOS only.
///
/// Magic bytes are used rather than `File.type()` because files arriving via
/// drag-and-drop frequently report an empty type.
pub(crate) async fn read_file_as_base64(file: web_sys::File) -> Result<(String, String), String> {
    use js_sys::{ArrayBuffer, Uint8Array};
    use wasm_bindgen_futures::JsFuture;

    let array_buffer: ArrayBuffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("Failed to read file: {:?}", e))?
        .dyn_into()
        .map_err(|_| "Failed to convert to ArrayBuffer")?;

    let uint8_array = Uint8Array::new(&array_buffer);
    let bytes = uint8_array.to_vec();

    if bytes.is_empty() {
        return Err("The selected file is empty".to_string());
    }

    // Fall back to the browser-reported type only when sniffing fails.
    let mime = detect_image_mime(&bytes)
        .map(|m| m.to_string())
        .or_else(|| {
            let reported = file.type_();
            if reported.starts_with("image/") {
                Some(reported)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            "Unsupported file type — please choose a JPEG, PNG, WebP, GIF or HEIC image".to_string()
        })?;

    Ok((mime, base64_encode(&bytes)))
}

/// Identify an image format from its leading bytes.
fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    // RIFF....WEBP
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // ISO-BMFF: ....ftyp<brand>. iPhone screenshots and photos are HEIC.
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return match &bytes[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => Some("image/heic"),
            b"avif" | b"avis" => Some("image/avif"),
            _ => None,
        };
    }
    None
}

/// Simple base64 encoder (avoiding extra dependencies in WASM).
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity((bytes.len() + 2) / 3 * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }

    result
}

/// Helper to get event target value from select element
fn event_target_value(ev: &web_sys::Event) -> String {
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok())
        .map(|el| el.value())
        .unwrap_or_default()
}

/// Helper to get event target value from input element
fn event_target_value_input(ev: &web_sys::Event) -> String {
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.value())
        .unwrap_or_default()
}

/// Helper to get event target
fn event_target<T: wasm_bindgen::JsCast>(ev: &web_sys::Event) -> T {
    ev.target().unwrap().dyn_into::<T>().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, path: &str) -> commands::ProfileInfo {
        commands::ProfileInfo {
            name: name.into(),
            filament_type: Some("PLA".into()),
            filament_id: None,
            path: path.into(),
            is_user_profile: true,
        }
    }

    #[test]
    fn the_journal_param_is_an_entry_id() {
        assert_eq!(journal_param(Some("2")), Some(2));
        assert_eq!(journal_param(Some(" 12 ")), Some(12));
        assert_eq!(journal_param(None), None);
        assert_eq!(journal_param(Some("")), None);
        assert_eq!(journal_param(Some("abc")), None);
        assert_eq!(journal_param(Some("0")), None);
        assert_eq!(journal_param(Some("-3")), None);
        assert_eq!(journal_param(Some("2.5")), None);
    }

    #[test]
    fn a_preset_picks_the_one_profile_with_that_name() {
        let list = [
            profile("Bambu PLA Basic", "/a.json"),
            profile("Other", "/b.json"),
        ];
        assert_eq!(
            preset_profile(&list, "Bambu PLA Basic").as_deref(),
            Some("/a.json")
        );
    }

    #[test]
    fn a_preset_with_no_or_several_matches_picks_nothing() {
        let list = [
            profile("Bambu PLA Basic", "/a.json"),
            profile("Bambu PLA Basic", "/b.json"),
            profile("Other", "/c.json"),
        ];
        assert_eq!(preset_profile(&list, "Bambu PLA Basic"), None);
        assert_eq!(preset_profile(&list, "Missing"), None);
        assert_eq!(preset_profile(&[], "Other"), None);
        // Names are compared as they are: no case folding, no trimming.
        assert_eq!(preset_profile(&list, "other"), None);
    }
}
