//! Settings → Application: auto-slicing and the slice cache.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::slicer::bridge;
use crate::slicer::types::SlicerSettings;

#[component]
pub fn SlicerSettingsSection() -> impl IntoView {
    let saved = RwSignal::new(None::<SlicerSettings>);
    // While a save is in flight the toggle is disabled, so answers can't
    // arrive out of order.
    let saving = RwSignal::new(false);
    let auto_status = RwSignal::new(None::<String>);
    let cache_status = RwSignal::new(None::<String>);

    spawn_local(async move {
        match bridge::get_settings().await {
            Ok(view) => {
                saved.try_set(Some(view.saved));
            }
            Err(e) => {
                auto_status.try_set(Some(format!("Failed to load: {e}")));
            }
        }
    });

    let on_toggle = move |ev: leptos::ev::Event| {
        let enabled = event_target_checked(&ev);
        let Some(previous) = saved.try_get_untracked().flatten() else {
            return;
        };
        let next = SlicerSettings {
            auto_slice: enabled,
            ..previous.clone()
        };
        // Show the change at once; roll back if the backend refuses it.
        saved.set(Some(next.clone()));
        saving.set(true);
        spawn_local(async move {
            match bridge::set_settings(next).await {
                Ok(view) => {
                    saved.try_set(Some(view.saved));
                    auto_status.try_set(Some(if enabled {
                        "New STLs will be sliced with your default printer, process and filament."
                            .to_string()
                    } else {
                        "Auto-slicing is off.".to_string()
                    }));
                }
                Err(e) => {
                    saved.try_set(Some(previous));
                    auto_status.try_set(Some(format!("Failed to save: {e}")));
                }
            }
            saving.try_set(false);
        });
    };

    let clear = move |_| {
        spawn_local(async move {
            // A refusal (e.g. "Finish or cancel the current slice first.")
            // is shown as the backend words it.
            let text = match bridge::clear_cache().await {
                Ok(bytes) => format!("Cleared {:.1} MB.", bytes as f64 / 1_048_576.0),
                Err(e) => e,
            };
            cache_status.try_set(Some(text));
        });
    };

    view! {
        <div class="form-group slicer-settings">
            <label class="checkbox-label" style="display: inline-flex; gap: 0.4rem;">
                <input
                    id="slice-auto"
                    type="checkbox"
                    disabled=move || saving.get() || saved.with(Option::is_none)
                    prop:checked=move || saved.with(|s| s.as_ref().is_some_and(|s| s.auto_slice))
                    on:change=on_toggle
                />
                "Slice new STLs automatically"
            </label>
            <p class="section-description">
                "Uses the printer, process and filament you last sliced with on the Slice page."
            </p>
            <Show when=move || auto_status.get().is_some()>
                <span class="status-text slice-auto-status">{move || auto_status.get().unwrap_or_default()}</span>
            </Show>
            <div class="input-row">
                <button class="btn btn-secondary btn-sm slice-clear-cache" on:click=clear>
                    "Clear slice cache"
                </button>
            </div>
            <Show when=move || cache_status.get().is_some()>
                <span class="status-text slice-cache-status">{move || cache_status.get().unwrap_or_default()}</span>
            </Show>
        </div>
    }
}
