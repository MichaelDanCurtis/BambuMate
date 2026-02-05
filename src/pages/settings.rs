use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands;
use crate::components::api_key_form::ApiKeyForm;

#[component]
pub fn SettingsPage() -> impl IntoView {
    let (bambu_path, set_bambu_path) = signal(String::new());
    let (path_status, set_path_status) = signal::<Option<String>>(None);
    let (ai_model, set_ai_model) = signal(String::new());
    let (ai_provider, set_ai_provider) = signal(String::from("claude"));
    let (model_status, set_model_status) = signal::<Option<String>>(None);

    // Load existing preferences on mount
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
            match commands::get_preference("ai_model").await {
                Ok(Some(model)) => {
                    set_ai_model.set(model);
                }
                Ok(None) => {}
                Err(_) => {}
            }
            match commands::get_preference("ai_provider").await {
                Ok(Some(provider)) => {
                    set_ai_provider.set(provider);
                }
                Ok(None) => {}
                Err(_) => {}
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

    let save_model_config = move |_| {
        let model = ai_model.get();
        let provider = ai_provider.get();
        spawn_local(async move {
            let model_result = commands::set_preference("ai_model", &model).await;
            let provider_result = commands::set_preference("ai_provider", &provider).await;
            match (model_result, provider_result) {
                (Ok(()), Ok(())) => {
                    set_model_status.set(Some("Model configuration saved".to_string()));
                }
                (Err(e), _) | (_, Err(e)) => {
                    set_model_status.set(Some(format!("Failed to save: {}", e)));
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
                <ApiKeyForm
                    service_name="Kimi K2 API Key"
                    service_id="bambumate-kimi-api"
                    placeholder="sk-..."
                />
                <ApiKeyForm
                    service_name="OpenRouter API Key"
                    service_id="bambumate-openrouter-api"
                    placeholder="sk-or-..."
                />
            </section>

            <section class="settings-section">
                <h3>"Model Configuration"</h3>
                <p class="section-description">"Select which AI provider and model to use for analysis."</p>

                <div class="form-group">
                    <label for="ai-provider">"AI Provider"</label>
                    <select
                        id="ai-provider"
                        class="input"
                        on:change=move |ev| {
                            set_ai_provider.set(event_target_value(&ev));
                        }
                        prop:value=move || ai_provider.get()
                    >
                        <option value="claude" selected=move || ai_provider.get() == "claude">"Claude (Anthropic)"</option>
                        <option value="openai" selected=move || ai_provider.get() == "openai">"OpenAI"</option>
                        <option value="kimi" selected=move || ai_provider.get() == "kimi">"Kimi K2 (Moonshot)"</option>
                        <option value="openrouter" selected=move || ai_provider.get() == "openrouter">"OpenRouter"</option>
                    </select>
                </div>

                <div class="form-group">
                    <label for="ai-model">"Model Name"</label>
                    <div class="input-row">
                        <input
                            id="ai-model"
                            type="text"
                            placeholder="e.g. claude-sonnet-4-20250514, gpt-4o, kimi-k2"
                            class="input"
                            prop:value=move || ai_model.get()
                            on:input=move |ev| {
                                set_ai_model.set(event_target_value(&ev));
                            }
                        />
                        <button class="btn btn-save" on:click=save_model_config>"Save"</button>
                    </div>
                    <Show when=move || model_status.get().is_some()>
                        <span class="status-text">{move || model_status.get().unwrap_or_default()}</span>
                    </Show>
                </div>
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
