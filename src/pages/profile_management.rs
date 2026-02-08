use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, FilamentSpecs, ProfileDetail, ProfileInfo};
use crate::components::specs_editor::SpecsEditor;

/// Key profile fields to display in the detail panel.
const KEY_FIELDS: &[&str] = &[
    "name",
    "filament_type",
    "filament_id",
    "inherits",
    "nozzle_temperature",
    "nozzle_temperature_initial_layer",
    "bed_temperature",
    "hot_plate_temp",
    "fan_max_speed",
    "fan_min_speed",
    "overhang_fan_speed",
    "slow_down_layer_time",
    "filament_retraction_length",
    "filament_retraction_speed",
    "filament_max_volumetric_speed",
    "filament_flow_ratio",
    "compatible_printers",
];

/// Extract display rows from raw JSON for the key fields.
fn build_display_rows(raw_json: &str) -> Vec<(String, String)> {
    let raw: serde_json::Value = serde_json::from_str(raw_json).unwrap_or_default();
    let obj = match raw.as_object() {
        Some(o) => o,
        None => return vec![],
    };

    KEY_FIELDS
        .iter()
        .filter_map(|&k| {
            obj.get(k).map(|v| {
                let display = match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .map(|item| match item {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    other => other.to_string(),
                };
                (k.to_string(), display)
            })
        })
        .collect()
}

#[component]
pub fn ProfileManagementPage() -> impl IntoView {
    // Profile list state
    let (profiles, set_profiles) = signal::<Vec<ProfileInfo>>(vec![]);
    let (is_loading, set_is_loading) = signal(true);
    let (list_error, set_list_error) = signal::<Option<String>>(None);
    let (filter_query, set_filter_query) = signal(String::new());

    // Selection state
    let (selected_path, set_selected_path) = signal::<Option<String>>(None);
    let (selected_detail, set_selected_detail) = signal::<Option<ProfileDetail>>(None);
    let (detail_loading, set_detail_loading) = signal(false);
    let (detail_error, set_detail_error) = signal::<Option<String>>(None);

    // Action state
    let (action_error, set_action_error) = signal::<Option<String>>(None);
    let (action_success, set_action_success) = signal::<Option<String>>(None);
    let (show_delete_confirm, set_show_delete_confirm) = signal(false);
    let (show_duplicate_input, set_show_duplicate_input) = signal(false);
    let (duplicate_name, set_duplicate_name) = signal(String::new());

    // Inline edit state
    let (editing_field, set_editing_field) = signal::<Option<String>>(None);
    let (edit_value, set_edit_value) = signal(String::new());

    // Specs editor state
    let (show_specs_editor, set_show_specs_editor) = signal(false);
    let (editor_specs, set_editor_specs) = signal::<Option<FilamentSpecs>>(None);
    let (specs_loading, set_specs_loading) = signal(false);

    // Load profiles on mount
    let load_profiles = move || {
        set_is_loading.set(true);
        spawn_local(async move {
            match commands::list_profiles().await {
                Ok(list) => {
                    set_profiles.set(list);
                    set_list_error.set(None);
                }
                Err(e) => set_list_error.set(Some(e)),
            }
            set_is_loading.set(false);
        });
    };

    Effect::new(move |_| {
        load_profiles();
    });

    // Select a profile
    let select_profile = move |path: String| {
        set_selected_path.set(Some(path.clone()));
        set_selected_detail.set(None);
        set_detail_loading.set(true);
        set_detail_error.set(None);
        set_action_error.set(None);
        set_action_success.set(None);
        set_editing_field.set(None);
        set_show_delete_confirm.set(false);
        set_show_duplicate_input.set(false);
        set_show_specs_editor.set(false);
        set_editor_specs.set(None);

        spawn_local(async move {
            match commands::read_profile(&path).await {
                Ok(detail) => {
                    set_selected_detail.set(Some(detail));
                    set_detail_error.set(None);
                }
                Err(e) => set_detail_error.set(Some(e)),
            }
            set_detail_loading.set(false);
        });
    };

    // Delete handler
    let do_delete = move || {
        let path = match selected_path.get() {
            Some(p) => p,
            None => return,
        };
        set_show_delete_confirm.set(false);

        spawn_local(async move {
            match commands::delete_profile(&path).await {
                Ok(()) => {
                    set_action_success.set(Some("Profile deleted".to_string()));
                    set_selected_path.set(None);
                    set_selected_detail.set(None);
                    if let Ok(list) = commands::list_profiles().await {
                        set_profiles.set(list);
                    }
                }
                Err(e) => set_action_error.set(Some(e)),
            }
        });
    };

    // Duplicate handler
    let do_duplicate = move || {
        let path = match selected_path.get() {
            Some(p) => p,
            None => return,
        };
        let name = duplicate_name.get();
        if name.is_empty() {
            set_action_error.set(Some("Please enter a name for the copy".to_string()));
            return;
        }
        set_show_duplicate_input.set(false);

        spawn_local(async move {
            match commands::duplicate_profile(&path, &name).await {
                Ok(detail) => {
                    set_action_success.set(Some(format!(
                        "Duplicated as '{}'",
                        detail.name.as_deref().unwrap_or("(unnamed)")
                    )));
                    if let Ok(list) = commands::list_profiles().await {
                        set_profiles.set(list);
                    }
                }
                Err(e) => set_action_error.set(Some(e)),
            }
        });
    };

    // Save edited field
    let save_field = move || {
        let field = match editing_field.get() {
            Some(f) => f,
            None => return,
        };
        let path = match selected_path.get() {
            Some(p) => p,
            None => return,
        };
        let val = edit_value.get();

        set_editing_field.set(None);

        // Wrap plain strings in quotes for valid JSON
        let json_val = if val.starts_with('[')
            || val.starts_with('{')
            || val.starts_with('"')
            || val == "true"
            || val == "false"
            || val == "null"
            || val.parse::<f64>().is_ok()
        {
            val
        } else {
            format!("\"{}\"", val.replace('\\', "\\\\").replace('"', "\\\""))
        };

        spawn_local(async move {
            match commands::update_profile_field(&path, &field, &json_val).await {
                Ok(detail) => {
                    set_selected_detail.set(Some(detail));
                    set_action_success.set(Some(format!("Updated '{}'", field)));
                }
                Err(e) => set_action_error.set(Some(e)),
            }
        });
    };

    // Open specs editor for current profile
    let open_specs_editor = move || {
        let path = match selected_path.get() {
            Some(p) => p,
            None => return,
        };
        set_specs_loading.set(true);
        set_action_error.set(None);
        set_action_success.set(None);

        spawn_local(async move {
            match commands::extract_specs_from_profile(&path).await {
                Ok(specs) => {
                    set_editor_specs.set(Some(specs));
                    set_show_specs_editor.set(true);
                }
                Err(e) => set_action_error.set(Some(format!("Failed to load specs: {}", e))),
            }
            set_specs_loading.set(false);
        });
    };

    // Save edited specs back to profile
    let save_specs = move |(specs, _printer): (FilamentSpecs, String)| {
        let path = match selected_path.get() {
            Some(p) => p,
            None => return,
        };
        set_action_error.set(None);

        spawn_local(async move {
            match commands::save_profile_specs(&path, &specs).await {
                Ok(detail) => {
                    set_selected_detail.set(Some(detail));
                    set_show_specs_editor.set(false);
                    set_editor_specs.set(None);
                    set_action_success.set(Some("Profile specs saved".to_string()));
                    // Refresh list in case name changed
                    if let Ok(list) = commands::list_profiles().await {
                        set_profiles.set(list);
                    }
                }
                Err(e) => set_action_error.set(Some(format!("Failed to save: {}", e))),
            }
        });
    };

    let cancel_specs_editor = move || {
        set_show_specs_editor.set(false);
        set_editor_specs.set(None);
    };

    // Filtered profiles
    let filtered_profiles = move || {
        let q = filter_query.get().to_lowercase();
        let list = profiles.get();
        if q.is_empty() {
            list
        } else {
            list.into_iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&q)
                        || p.filament_type
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&q)
                })
                .collect()
        }
    };

    view! {
        <div class="page profile-management-page">
            <style>{include_str!("profile_management.css")}</style>

            <h2>"Profile Management"</h2>
            <p class="page-description">
                "Browse, edit, and manage your Bambu Studio filament profiles."
            </p>

            // Action feedback
            {move || action_error.get().map(|e| view! {
                <div class="profile-error">{e}</div>
            })}
            {move || action_success.get().map(|s| view! {
                <div class="profile-success">{s}</div>
            })}

            // Toolbar
            <div class="profile-toolbar">
                <input
                    type="text"
                    class="profile-filter-input"
                    placeholder="Filter profiles..."
                    prop:value=move || filter_query.get()
                    on:input=move |ev| set_filter_query.set(event_target_value(&ev))
                />
                <span class="profile-count">
                    {move || {
                        let filtered = filtered_profiles().len();
                        let total = profiles.get().len();
                        if filtered == total {
                            format!("{} profiles", total)
                        } else {
                            format!("{} / {}", filtered, total)
                        }
                    }}
                </span>
            </div>

            // Loading
            <Show when=move || is_loading.get()>
                <div class="profile-loading">
                    <span>"Loading profiles..."</span>
                </div>
            </Show>

            // List error
            {move || list_error.get().map(|e| view! {
                <div class="profile-error">{e}</div>
            })}

            // Main layout
            <Show when=move || !is_loading.get()>
                {move || {
                    view! {
                        <div class="profile-layout">
                            // Profile list
                            <div class="profile-list-panel">
                                <For
                                    each=move || filtered_profiles()
                                    key=|p| p.path.clone()
                                    children=move |p| {
                                        let path = p.path.clone();
                                        let path_click = path.clone();
                                        let is_selected = move || selected_path.get().as_deref() == Some(&path);
                                        view! {
                                            <div
                                                class="profile-list-item"
                                                class:selected=is_selected
                                                on:click=move |_| select_profile(path_click.clone())
                                            >
                                                <div class="profile-item-info">
                                                    <div class="profile-item-name">{p.name.clone()}</div>
                                                    <div class="profile-item-type">
                                                        {p.filament_type.clone().unwrap_or_default()}
                                                    </div>
                                                </div>
                                                {p.filament_type.clone().map(|t| view! {
                                                    <span class="profile-item-badge">{t}</span>
                                                })}
                                            </div>
                                        }
                                    }
                                />
                                <Show when=move || filtered_profiles().is_empty() && !is_loading.get()>
                                    <div class="profile-detail-empty">
                                        <span>"No profiles found"</span>
                                    </div>
                                </Show>
                            </div>

                            // Detail panel
                            <div class="profile-detail-panel">
                                {move || {
                                    if selected_path.get().is_none() {
                                        return view! {
                                            <div class="profile-detail-empty">
                                                <span>"Select a profile to view details"</span>
                                            </div>
                                        }.into_any();
                                    }

                                    if detail_loading.get() {
                                        return view! {
                                            <div class="profile-loading">
                                                <span>"Loading profile..."</span>
                                            </div>
                                        }.into_any();
                                    }

                                    if let Some(e) = detail_error.get() {
                                        return view! {
                                            <div class="profile-error">{e}</div>
                                        }.into_any();
                                    }

                                    match selected_detail.get() {
                                        None => view! {
                                            <div class="profile-detail-empty">
                                                <span>"Select a profile to view details"</span>
                                            </div>
                                        }.into_any(),
                                        Some(detail) => {
                                            view! {
                                                <div>
                                                    <div class="profile-detail-header">
                                                        <div>
                                                            <h3 class="profile-detail-title">
                                                                {detail.name.clone().unwrap_or_else(|| "(unnamed)".to_string())}
                                                            </h3>
                                                            <div class="profile-detail-subtitle">
                                                                {detail.filament_type.clone().unwrap_or_default()}
                                                                " | "
                                                                {format!("{} fields", detail.field_count)}
                                                            </div>
                                                        </div>
                                                        <div class="profile-detail-actions">
                                                            <button
                                                                class="btn-icon"
                                                                title="Open in Bambu Studio"
                                                                on:click=move |_| {
                                                                    let path = selected_path.get();
                                                                    spawn_local(async move {
                                                                        let profile = path;
                                                                        match commands::launch_bambu_studio(None, profile).await {
                                                                            Ok(_) => set_action_success.set(Some("Bambu Studio launched".to_string())),
                                                                            Err(e) => set_action_error.set(Some(e)),
                                                                        }
                                                                    });
                                                                }
                                                            >
                                                                "Open in BS"
                                                            </button>
                                                            <button
                                                                class="btn-icon btn-edit-specs"
                                                                title="Edit Specs"
                                                                disabled=move || specs_loading.get()
                                                                on:click=move |_| open_specs_editor()
                                                            >
                                                                {move || if specs_loading.get() { "Loading..." } else { "Edit Specs" }}
                                                            </button>
                                                            <button
                                                                class="btn-icon"
                                                                title="Duplicate"
                                                                on:click=move |_| {
                                                                    let name = selected_detail.get()
                                                                        .and_then(|d| d.name.clone())
                                                                        .unwrap_or_default();
                                                                    set_duplicate_name.set(format!("{} (copy)", name));
                                                                    set_show_duplicate_input.set(true);
                                                                    set_action_error.set(None);
                                                                    set_action_success.set(None);
                                                                }
                                                            >
                                                                "Copy"
                                                            </button>
                                                            <button
                                                                class="btn-icon btn-danger"
                                                                title="Delete"
                                                                on:click=move |_| {
                                                                    set_show_delete_confirm.set(true);
                                                                    set_action_error.set(None);
                                                                    set_action_success.set(None);
                                                                }
                                                            >
                                                                "Delete"
                                                            </button>
                                                        </div>
                                                    </div>

                                                    // Duplicate input
                                                    <Show when=move || show_duplicate_input.get()>
                                                        <div class="duplicate-input-row">
                                                            <input
                                                                type="text"
                                                                placeholder="New profile name..."
                                                                prop:value=move || duplicate_name.get()
                                                                on:input=move |ev| set_duplicate_name.set(event_target_value(&ev))
                                                                on:keydown=move |ev| {
                                                                    if ev.key() == "Enter" { do_duplicate(); }
                                                                    if ev.key() == "Escape" { set_show_duplicate_input.set(false); }
                                                                }
                                                            />
                                                            <button class="btn btn-primary" on:click=move |_| do_duplicate()>
                                                                "Duplicate"
                                                            </button>
                                                            <button class="btn btn-secondary" on:click=move |_| set_show_duplicate_input.set(false)>
                                                                "Cancel"
                                                            </button>
                                                        </div>
                                                    </Show>

                                                    // Specs editor or fields table
                                                    {move || {
                                                        if show_specs_editor.get() {
                                                            if let Some(specs) = editor_specs.get() {
                                                                return view! {
                                                                    <SpecsEditor
                                                                        specs=specs
                                                                        on_generate=save_specs
                                                                        on_cancel=move |_| cancel_specs_editor()
                                                                        action_label="Save Changes"
                                                                        cancel_label="Cancel"
                                                                        show_printer=false
                                                                        fill_defaults=false
                                                                    />
                                                                }.into_any();
                                                            }
                                                        }

                                                        let detail = selected_detail.get();
                                                        let rows = detail
                                                            .map(|d| build_display_rows(&d.raw_json))
                                                            .unwrap_or_default();

                                                        view! {
                                                            <table class="profile-fields">
                                                                <tbody>
                                                                    {rows.into_iter().map(|(key, value)| {
                                                                        let key_edit = key.clone();
                                                                        let key_display = key.clone();
                                                                        let value_display = value.clone();
                                                                        let value_for_edit = value.clone();
                                                                        view! {
                                                                            <tr>
                                                                                <td class="field-key">{key_display}</td>
                                                                                <td class="field-value">
                                                                                    {move || {
                                                                                        let is_editing = editing_field.get().as_deref() == Some(&key_edit);
                                                                                        if is_editing {
                                                                                            view! {
                                                                                                <input
                                                                                                    class="field-edit-input"
                                                                                                    prop:value=move || edit_value.get()
                                                                                                    on:input=move |ev| set_edit_value.set(event_target_value(&ev))
                                                                                                    on:blur=move |_| save_field()
                                                                                                    on:keydown=move |ev| {
                                                                                                        if ev.key() == "Enter" { save_field(); }
                                                                                                        if ev.key() == "Escape" { set_editing_field.set(None); }
                                                                                                    }
                                                                                                />
                                                                                            }.into_any()
                                                                                        } else {
                                                                                            let key_click = key_edit.clone();
                                                                                            let val_click = value_for_edit.clone();
                                                                                            view! {
                                                                                                <span
                                                                                                    on:click=move |_| {
                                                                                                        set_editing_field.set(Some(key_click.clone()));
                                                                                                        set_edit_value.set(val_click.clone());
                                                                                                        set_action_error.set(None);
                                                                                                        set_action_success.set(None);
                                                                                                    }
                                                                                                    title="Click to edit"
                                                                                                >
                                                                                                    {value_display.clone()}
                                                                                                </span>
                                                                                            }.into_any()
                                                                                        }
                                                                                    }}
                                                                                </td>
                                                                            </tr>
                                                                        }
                                                                    }).collect::<Vec<_>>()}
                                                                </tbody>
                                                            </table>
                                                        }.into_any()
                                                    }}
                                                </div>
                                            }.into_any()
                                        }
                                    }
                                }}
                            </div>
                        </div>
                    }
                }}
            </Show>

            // Delete confirmation modal
            <Show when=move || show_delete_confirm.get()>
                <div class="modal-overlay" on:click=move |_| set_show_delete_confirm.set(false)>
                    <div class="modal-content" on:click=move |ev| ev.stop_propagation()>
                        <h3>"Delete Profile?"</h3>
                        <p>
                            "This will permanently delete \""
                            {move || selected_detail.get().and_then(|d| d.name).unwrap_or_default()}
                            "\" and its metadata. This cannot be undone."
                        </p>
                        <div class="modal-actions">
                            <button class="btn btn-secondary" on:click=move |_| set_show_delete_confirm.set(false)>
                                "Cancel"
                            </button>
                            <button class="btn btn-primary" on:click=move |_| do_delete()>
                                "Delete"
                            </button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}
