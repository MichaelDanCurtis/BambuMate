//! Inline repair panel for the `bambu.preset_sync` Health check.
//!
//! Lists presets Bambu Studio will not upload and lets the user choose which
//! to reset to "new". BambuMate never contacts Bambu Cloud: the repair only
//! rewrites `.info` files, and Bambu Studio uploads the presets on its next
//! sync.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, UnsyncedPreset};

/// Label under a preset that only matches BambuMate's file shape.
pub const SIGNATURE_NOTE: &str =
    "Might already be synced — only tick if it's missing from your printer";

/// Shown while "Repair selected" is disabled because Bambu Studio is open.
pub const CLOSE_STUDIO_NOTE: &str = "Close Bambu Studio first.";

/// Paths ticked when the panel opens: only presets whose `setting_id`
/// BambuMate is known to have made up.
pub fn default_selection(presets: &[UnsyncedPreset]) -> Vec<String> {
    presets
        .iter()
        .filter(|p| p.source == "confirmed")
        .map(|p| p.path.clone())
        .collect()
}

/// Message shown after a repair.
pub fn repaired_message(count: usize) -> String {
    let noun = if count == 1 { "preset" } else { "presets" };
    format!("Repaired {count} {noun}. Open Bambu Studio while signed in to upload them.")
}

#[component]
pub fn PresetSyncPanel() -> impl IntoView {
    let presets = RwSignal::new(Vec::<UnsyncedPreset>::new());
    let selected = RwSignal::new(Vec::<String>::new());
    let studio_running = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let result = RwSignal::new(Option::<String>::None);
    let skipped = RwSignal::new(Vec::<(String, String)>::new());
    let error = RwSignal::new(Option::<String>::None);

    spawn_local(async move {
        match commands::list_unsynced_presets().await {
            Ok(list) => {
                selected.set(default_selection(&list.presets));
                presets.set(list.presets);
                studio_running.set(list.bambu_studio_running);
            }
            Err(e) => error.set(Some(e)),
        }
    });

    let repair = move |_| {
        let paths = selected.get_untracked();
        if paths.is_empty() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match commands::repair_preset_sync(paths).await {
                Ok(r) => {
                    presets.update(|list| list.retain(|p| !r.repaired.contains(&p.path)));
                    selected.update(|list| list.retain(|p| !r.repaired.contains(p)));
                    result.set(Some(repaired_message(r.repaired.len())));
                    skipped.set(r.skipped);
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="preset-sync-panel">
            <ul class="preset-sync-list">
                {move || {
                    presets
                        .get()
                        .into_iter()
                        .map(|preset| view! { <PresetSyncRow preset=preset selected=selected /> })
                        .collect_view()
                }}
            </ul>
            <button
                class="btn btn-primary btn-sm preset-sync-repair"
                on:click=repair
                disabled=move || {
                    studio_running.get() || busy.get() || selected.with(|s| s.is_empty())
                }
            >
                "Repair selected"
            </button>
            <Show when=move || studio_running.get()>
                <p class="preset-sync-hint">{CLOSE_STUDIO_NOTE}</p>
            </Show>
            {move || result.get().map(|m| view! { <p class="preset-sync-result">{m}</p> })}
            {move || {
                skipped
                    .get()
                    .into_iter()
                    .map(|(path, reason)| {
                        view! {
                            <p class="preset-sync-skipped">{format!("Skipped {path}: {reason}")}</p>
                        }
                    })
                    .collect_view()
            }}
            {move || {
                error.get().map(|e| view! {
                    <div class="health-error">
                        <span class="status-text status-error">{e}</span>
                    </div>
                })
            }}
        </div>
    }
}

#[component]
fn PresetSyncRow(preset: UnsyncedPreset, selected: RwSignal<Vec<String>>) -> impl IntoView {
    let path = preset.path.clone();
    let path_for_change = preset.path.clone();
    let is_signature = preset.source != "confirmed";

    view! {
        <li class="preset-sync-row">
            <label>
                <input
                    type="checkbox"
                    prop:checked=move || selected.with(|s| s.contains(&path))
                    on:change=move |ev| {
                        let ticked = event_target_checked(&ev);
                        let path = path_for_change.clone();
                        selected.update(|s| {
                            s.retain(|p| p != &path);
                            if ticked {
                                s.push(path);
                            }
                        });
                    }
                />
                <span class="preset-sync-name">{preset.profile_name}</span>
                <code class="preset-sync-file">{preset.file_name}</code>
            </label>
            {is_signature.then(|| view! { <p class="preset-sync-note">{SIGNATURE_NOTE}</p> })}
        </li>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(path: &str, source: &str) -> UnsyncedPreset {
        UnsyncedPreset {
            path: path.to_string(),
            profile_name: path.to_string(),
            file_name: format!("{path}.json"),
            source: source.to_string(),
        }
    }

    #[test]
    fn only_confirmed_presets_start_ticked() {
        let presets = vec![
            preset("a", "confirmed"),
            preset("b", "signature"),
            preset("c", "confirmed"),
        ];
        assert_eq!(
            default_selection(&presets),
            vec!["a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn repaired_message_matches_the_spec_copy() {
        assert_eq!(
            repaired_message(3),
            "Repaired 3 presets. Open Bambu Studio while signed in to upload them."
        );
        assert_eq!(
            repaired_message(1),
            "Repaired 1 preset. Open Bambu Studio while signed in to upload them."
        );
    }
}
