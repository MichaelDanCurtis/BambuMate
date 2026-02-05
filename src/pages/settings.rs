use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands;
use crate::components::api_key_form::ApiKeyForm;

#[component]
pub fn SettingsPage() -> impl IntoView {
    let (bambu_path, set_bambu_path) = signal(String::new());
    let (path_status, set_path_status) = signal::<Option<String>>(None);

    // Load existing Bambu Studio path preference on mount
    Effect::new(move |_| {
        spawn_local(async move {
            match commands::get_preference("bambu_studio_path").await {
                Ok(Some(path)) => {
                    set_bambu_path.set(path);
                }
                Ok(None) => {}
                Err(e) => {
                    set_path_status.set(Some(format!("Failed to load preference: {}", e)));
                }
            }
        });
    });

    let save_bambu_path = move |_| {
        let path = bambu_path.get();
        spawn_local(async move {
            match commands::set_preference("bambu_studio_path", &path).await {
                Ok(()) => {
                    set_path_status.set(Some("Path saved".to_string()));
                }
                Err(e) => {
                    set_path_status.set(Some(format!("Failed to save: {}", e)));
                }
            }
        });
    };

    view! {
        <div class="page settings-page">
            <h2>"Settings"</h2>

            <section class="settings-section">
                <h3>"API Keys"</h3>
                <p class="section-description">"API keys are stored securely in your macOS Keychain."</p>

                <ApiKeyForm
                    service_name="Claude API Key"
                    service_id="bambumate-claude-api"
                    placeholder="sk-ant-..."
                />
                <ApiKeyForm
                    service_name="OpenAI API Key"
                    service_id="bambumate-openai-api"
                    placeholder="sk-..."
                />
            </section>

            <section class="settings-section">
                <h3>"Application"</h3>
                <p class="section-description">"Configure application paths and preferences."</p>

                <div class="form-group">
                    <label for="bambu-path">"Bambu Studio Path"</label>
                    <div class="input-row">
                        <input
                            id="bambu-path"
                            type="text"
                            placeholder="/Applications/BambuStudio.app"
                            class="input"
                            prop:value=move || bambu_path.get()
                            on:input=move |ev| {
                                set_bambu_path.set(event_target_value(&ev));
                            }
                        />
                        <button class="btn btn-save" on:click=save_bambu_path>"Save"</button>
                    </div>
                    <Show when=move || path_status.get().is_some()>
                        <span class="status-text">{move || path_status.get().unwrap_or_default()}</span>
                    </Show>
                </div>
            </section>
        </div>
    }
}
