use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands;

#[component]
pub fn ApiKeyForm(
    /// Display name, e.g. "Claude API Key"
    #[prop(into)]
    service_name: String,
    /// Keyring service identifier, e.g. "bambumate-claude-api"
    #[prop(into)]
    service_id: String,
    /// Placeholder text for the input field
    #[prop(into)]
    placeholder: String,
) -> impl IntoView {
    let (key_value, set_key_value) = signal(String::new());
    let (is_saved, set_is_saved) = signal(false);
    let (is_loading, set_is_loading) = signal(false);
    let (error_message, set_error_message) = signal::<Option<String>>(None);

    // Check for existing key on mount
    let sid_check = service_id.clone();
    Effect::new(move |_| {
        let sid = sid_check.clone();
        spawn_local(async move {
            match commands::get_api_key(&sid).await {
                Ok(Some(_)) => {
                    set_is_saved.set(true);
                }
                Ok(None) => {
                    set_is_saved.set(false);
                }
                Err(e) => {
                    set_error_message.set(Some(format!("Failed to check key: {}", e)));
                }
            }
        });
    });

    // Save handler
    let sid_save = service_id.clone();
    let save_key = move |_| {
        let sid = sid_save.clone();
        let key = key_value.get();
        if key.is_empty() {
            set_error_message.set(Some("Please enter an API key".to_string()));
            return;
        }
        set_is_loading.set(true);
        set_error_message.set(None);
        spawn_local(async move {
            match commands::set_api_key(&sid, &key).await {
                Ok(()) => {
                    set_is_saved.set(true);
                    set_key_value.set(String::new());
                    set_error_message.set(None);
                }
                Err(e) => {
                    set_error_message.set(Some(format!("Failed to save: {}", e)));
                }
            }
            set_is_loading.set(false);
        });
    };

    // Delete handler
    let sid_delete = service_id.clone();
    let delete_key = move |_| {
        let sid = sid_delete.clone();
        set_is_loading.set(true);
        set_error_message.set(None);
        spawn_local(async move {
            match commands::delete_api_key(&sid).await {
                Ok(()) => {
                    set_is_saved.set(false);
                    set_error_message.set(None);
                }
                Err(e) => {
                    set_error_message.set(Some(format!("Failed to delete: {}", e)));
                }
            }
            set_is_loading.set(false);
        });
    };

    view! {
        <div class="form-group api-key-form">
            <label>{service_name}</label>
            <div class="input-row">
                <input
                    type="password"
                    placeholder=placeholder
                    class="input input-password"
                    prop:value=move || key_value.get()
                    on:input=move |ev| {
                        set_key_value.set(event_target_value(&ev));
                    }
                    disabled=move || is_loading.get()
                />
                <button
                    class="btn btn-save"
                    on:click=save_key
                    disabled=move || is_loading.get()
                >
                    {move || if is_loading.get() { "Saving..." } else { "Save" }}
                </button>
                <button
                    class="btn btn-delete"
                    on:click=delete_key
                    disabled=move || is_loading.get()
                    style:display=move || if is_saved.get() { "inline-block" } else { "none" }
                >
                    "Delete"
                </button>
            </div>
            <div class="key-status-row">
                {move || {
                    if let Some(err) = error_message.get() {
                        view! {
                            <span class="status-text status-error">{err}</span>
                        }.into_any()
                    } else if is_saved.get() {
                        view! {
                            <span class="status-text status-saved">"Saved to Keychain"</span>
                        }.into_any()
                    } else {
                        view! {
                            <span class="status-text status-not-set">"Not configured"</span>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
