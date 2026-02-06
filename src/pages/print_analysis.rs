//! Print analysis page for AI-powered defect detection.
//!
//! Users can drag-and-drop or browse for a print photo to analyze.

use leptos::prelude::*;
use leptos::html::Div;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::commands;
use crate::components::change_preview::ChangePreview;
use crate::components::defect_report::DefectReportDisplay;

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

/// Print analysis page component.
#[component]
pub fn PrintAnalysisPage() -> impl IntoView {
    let (state, set_state) = signal(AnalysisState::Idle);
    let (image_preview, set_image_preview) = signal::<Option<String>>(None);
    let (material_override, set_material_override) = signal::<Option<String>>(None);
    let (profile_path, set_profile_path) = signal::<Option<String>>(None);
    // Apply flow state
    let (show_apply_dialog, set_show_apply_dialog) = signal(false);
    let (current_session_id, set_current_session_id) = signal::<Option<i64>>(None);
    let (apply_message, set_apply_message) = signal::<Option<String>>(None);
    // History/revert state
    let (revert_message, set_revert_message) = signal::<Option<String>>(None);
    let (history_key, set_history_key) = signal(0u32);

    // Handle analyze button click
    let on_analyze = move |_| {
        let current_state = state.get();
        if let AnalysisState::Ready(base64) = current_state {
            set_state.set(AnalysisState::Analyzing);

            let material = material_override.get();
            let path = profile_path.get();
            spawn_local(async move {
                match commands::analyze_print(base64, path, material).await {
                    Ok(response) => {
                        // Store session ID for apply flow
                        set_current_session_id.set(response.session_id);
                        set_state.set(AnalysisState::Complete(response));
                    }
                    Err(e) => {
                        set_state.set(AnalysisState::Error(e));
                    }
                }
            });
        }
    };

    // Reset to try another image
    let on_reset = move |_| {
        set_state.set(AnalysisState::Idle);
        set_image_preview.set(None);
        set_show_apply_dialog.set(false);
        set_current_session_id.set(None);
        set_apply_message.set(None);
        set_revert_message.set(None);
    };

    view! {
        <div class="page print-analysis-page">
            <style>{include_str!("print_analysis.css")}</style>

            <h2>"Print Analysis"</h2>
            <p class="page-description">
                "Upload a photo of your test print to detect defects and get recommendations."
            </p>

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
                                    <option value="">"Auto-detect"</option>
                                    <option value="PLA">"PLA"</option>
                                    <option value="PETG">"PETG"</option>
                                    <option value="ABS">"ABS"</option>
                                    <option value="ASA">"ASA"</option>
                                    <option value="TPU">"TPU"</option>
                                    <option value="PA">"Nylon (PA)"</option>
                                    <option value="PC">"Polycarbonate (PC)"</option>
                                </select>
                            </div>

                            <div class="profile-path-input">
                                <label>"Profile path (optional):"</label>
                                <input
                                    type="text"
                                    class="input"
                                    placeholder="/path/to/filament/profile.json"
                                    on:input=move |ev| {
                                        let value = event_target_value_input(&ev);
                                        if value.is_empty() {
                                            set_profile_path.set(None);
                                        } else {
                                            set_profile_path.set(Some(value));
                                        }
                                    }
                                />
                                <p class="input-hint">"Enter a Bambu Studio profile path to get current values and enable history tracking."</p>
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
                            <div class="loading-indicator">
                                <div class="spinner"></div>
                                <p>"Analyzing your print..."</p>
                                <p class="hint">"This may take 10-30 seconds"</p>
                            </div>
                        </div>
                    }.into_any(),

                    AnalysisState::Complete(ref response) => {
                        let response = response.clone();
                        let path_for_display = profile_path.get();
                        let recs_for_dialog = response.recommendations.clone();
                        let path_for_dialog = profile_path.get().unwrap_or_default();
                        view! {
                            <div class="analysis-results">
                                {move || image_preview.get().map(|src| view! {
                                    <img src=src class="preview-image small" alt="Analyzed print" />
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
                            </div>
                        }.into_any()
                    },

                    AnalysisState::Error(ref msg) => {
                        let msg = msg.clone();
                        view! {
                            <div class="error-state">
                                <div class="error-message">
                                    <h3>"Analysis Failed"</h3>
                                    <p>{msg}</p>
                                </div>
                                <button class="btn btn-secondary" on:click=on_reset>
                                    "Try Again"
                                </button>
                            </div>
                        }.into_any()
                    },
                }
            }}
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
    let handle_file_loaded = move |base64: String| {
        set_image_preview.set(Some(format!("data:image/jpeg;base64,{}", base64.clone())));
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
                            Ok(base64) => {
                                handle_file_loaded(base64);
                            }
                            Err(e) => {
                                web_sys::console::error_1(&format!("Failed to read file: {}", e).into());
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
                        Ok(base64) => {
                            handle_file_loaded(base64);
                        }
                        Err(e) => {
                            web_sys::console::error_1(&format!("Failed to read file: {}", e).into());
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
                    <div class="spinner"></div>
                    <p>"Loading image..."</p>
                </div>
            </Show>
        </div>
    }
}

/// Read a File as base64 string.
async fn read_file_as_base64(file: web_sys::File) -> Result<String, String> {
    use js_sys::{ArrayBuffer, Uint8Array};
    use wasm_bindgen_futures::JsFuture;

    let array_buffer: ArrayBuffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("Failed to read file: {:?}", e))?
        .dyn_into()
        .map_err(|_| "Failed to convert to ArrayBuffer")?;

    let uint8_array = Uint8Array::new(&array_buffer);
    let bytes = uint8_array.to_vec();

    // Base64 encode
    Ok(base64_encode(&bytes))
}

/// Simple base64 encoder (avoiding extra dependencies in WASM).
fn base64_encode(bytes: &[u8]) -> String {
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
