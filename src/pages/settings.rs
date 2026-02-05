use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, ModelInfo};
use crate::components::api_key_form::ApiKeyForm;
use crate::theme::ThemeContext;

#[component]
pub fn SettingsPage() -> impl IntoView {
    let (bambu_path, set_bambu_path) = signal(String::new());
    let (path_status, set_path_status) = signal::<Option<String>>(None);
    let (ai_model, set_ai_model) = signal(String::new());
    let (ai_provider, set_ai_provider) = signal(String::from("claude"));
    let (model_status, set_model_status) = signal::<Option<String>>(None);
    let (models, set_models) = signal::<Vec<ModelInfo>>(vec![]);
    let (models_loading, set_models_loading) = signal(false);
    let (models_error, set_models_error) = signal::<Option<String>>(None);
    let (prefs_loaded, set_prefs_loaded) = signal(false);

    let theme_ctx = use_context::<ThemeContext>().expect("ThemeContext not provided");

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
            match commands::get_preference("ai_provider").await {
                Ok(Some(provider)) => {
                    set_ai_provider.set(provider);
                }
                Ok(None) => {}
                Err(_) => {}
            }
            match commands::get_preference("ai_model").await {
                Ok(Some(model)) => {
                    set_ai_model.set(model);
                }
                Ok(None) => {}
                Err(_) => {}
            }
            set_prefs_loaded.set(true);
        });
    });

    // Fetch models whenever provider changes (after prefs are loaded)
    Effect::new(move |_| {
        let provider = ai_provider.get();
        if !prefs_loaded.get() {
            return;
        }
        set_models_loading.set(true);
        set_models_error.set(None);
        set_models.set(vec![]);
        spawn_local(async move {
            match commands::list_models(&provider).await {
                Ok(model_list) => {
                    set_models.set(model_list);
                    set_models_loading.set(false);
                }
                Err(e) => {
                    set_models_error.set(Some(e));
                    set_models_loading.set(false);
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

    let refresh_models = move |_| {
        let provider = ai_provider.get();
        set_models_loading.set(true);
        set_models_error.set(None);
        spawn_local(async move {
            match commands::list_models(&provider).await {
                Ok(model_list) => {
                    set_models.set(model_list);
                    set_models_loading.set(false);
                }
                Err(e) => {
                    set_models_error.set(Some(e));
                    set_models_loading.set(false);
                }
            }
        });
    };

    let on_theme_change = move |ev: leptos::ev::Event| {
        let new_theme = event_target_value(&ev);
        theme_ctx.set_theme.set(new_theme.clone());
        spawn_local(async move {
            let _ = commands::set_preference("theme", &new_theme).await;
        });
    };

    view! {
        <div class="page settings-page">
            <h2>"Settings"</h2>

            <section class="settings-section">
                <h3>"Appearance"</h3>
                <p class="section-description">"Choose how BambuMate looks."</p>

                <div class="form-group">
                    <label for="theme-select">"Theme"</label>
                    <div class="theme-picker">
                        <select
                            id="theme-select"
                            class="input"
                            on:change=on_theme_change
                            prop:value=move || theme_ctx.theme.get()
                        >
                            <option value="system" selected=move || theme_ctx.theme.get() == "system">"System"</option>
                            <option value="light" selected=move || theme_ctx.theme.get() == "light">"Light"</option>
                            <option value="dark" selected=move || theme_ctx.theme.get() == "dark">"Dark"</option>
                        </select>
                    </div>
                </div>
            </section>

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
                    <label for="ai-model">"Model"</label>
                    <div class="input-row">
                        <Show
                            when=move || !models_loading.get() && models_error.get().is_none() && !models.get().is_empty()
                            fallback=move || {
                                if models_loading.get() {
                                    view! {
                                        <select class="input" disabled=true>
                                            <option>"Loading models..."</option>
                                        </select>
                                    }.into_any()
                                } else if let Some(err) = models_error.get() {
                                    view! {
                                        <select class="input" disabled=true>
                                            <option>{err}</option>
                                        </select>
                                    }.into_any()
                                } else {
                                    view! {
                                        <select class="input" disabled=true>
                                            <option>"No models available"</option>
                                        </select>
                                    }.into_any()
                                }
                            }
                        >
                            <select
                                id="ai-model"
                                class="input"
                                on:change=move |ev| {
                                    set_ai_model.set(event_target_value(&ev));
                                }
                                prop:value=move || ai_model.get()
                            >
                                <option value="">"-- Select a model --"</option>
                                {move || {
                                    models.get().into_iter().map(|m| {
                                        let id = m.id.clone();
                                        let display = if m.name != m.id {
                                            format!("{} ({})", m.name, m.id)
                                        } else {
                                            m.id.clone()
                                        };
                                        let is_selected = ai_model.get() == id;
                                        view! {
                                            <option value={id} selected=is_selected>{display}</option>
                                        }
                                    }).collect::<Vec<_>>()
                                }}
                            </select>
                        </Show>
                        <button class="btn btn-secondary" on:click=refresh_models title="Refresh model list">
                            "Refresh"
                        </button>
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
