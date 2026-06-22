use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::app::FeatureFlagsContext;
use crate::commands::{self, ModelInfo};
use crate::components::api_key_form::ApiKeyForm;
use crate::theme::ThemeContext;

#[component]
pub fn SettingsPage() -> impl IntoView {
    let (bambu_path, set_bambu_path) = signal(String::new());
    let (path_status, set_path_status) = signal::<Option<String>>(None);
    let (path_valid, set_path_valid) = signal(false);
    let (stl_watch_dir, set_stl_watch_dir) = signal(String::new());
    let (stl_status, set_stl_status) = signal::<Option<String>>(None);
    let (ai_model, set_ai_model) = signal(String::new());
    let (ai_provider, set_ai_provider) = signal(String::from("claude"));
    let (model_status, set_model_status) = signal::<Option<String>>(None);
    let (models, set_models) = signal::<Vec<ModelInfo>>(vec![]);
    let (models_loading, set_models_loading) = signal(false);
    let (models_error, set_models_error) = signal::<Option<String>>(None);
    let (prefs_loaded, set_prefs_loaded) = signal(false);
    let (local_url, set_local_url) = signal("http://localhost:1234".to_string());
    let (local_url_status, set_local_url_status) = signal::<Option<String>>(None);
    let (is_searching_path, set_is_searching_path) = signal(false);

    let theme_ctx = use_context::<ThemeContext>().expect("ThemeContext not provided");
    let ff_ctx = use_context::<FeatureFlagsContext>().expect("FeatureFlagsContext not provided");

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
            if let Ok(Some(dir)) = commands::get_stl_watch_dir().await {
                set_stl_watch_dir.set(dir);
            }
            match commands::get_preference("ai_model").await {
                Ok(Some(model)) => {
                    set_ai_model.set(model);
                }
                Ok(None) => {}
                Err(_) => {}
            }
            match commands::get_preference("local_mcp_url").await {
                Ok(Some(url)) => {
                    set_local_url.set(url);
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

    let save_stl_watch_dir = move |_: leptos::ev::MouseEvent| {
        let dir = stl_watch_dir.get();
        spawn_local(async move {
            match commands::set_stl_watch_dir(&dir).await {
                Ok(()) => set_stl_status.set(Some("STL watch directory saved".to_string())),
                Err(e) => set_stl_status.set(Some(format!("Failed: {}", e))),
            }
        });
    };

    let save_bambu_path = move |_| {
        let path = bambu_path.get();
        spawn_local(async move {
            // Validate first
            if let Ok(validation) = commands::validate_bambu_studio_path(&path).await {
                set_path_valid.set(validation.valid);
                if !validation.valid {
                    set_path_status.set(Some(format!("Warning: {}", validation.message)));
                }
            }
            match commands::set_preference("bambu_studio_path", &path).await {
                Ok(()) => {
                    if path_valid.get_untracked() {
                        set_path_status.set(Some("Path saved and validated".to_string()));
                    } else {
                        // Keep the warning from validation
                        let existing = path_status.get_untracked().unwrap_or_default();
                        if existing.is_empty() {
                            set_path_status.set(Some("Path saved".to_string()));
                        }
                    }
                }
                Err(e) => {
                    set_path_status.set(Some(format!("Failed to save: {}", e)));
                }
            }
        });
    };

    let on_search_path = move |_| {
        set_is_searching_path.set(true);
        set_path_status.set(None);
        spawn_local(async move {
            match commands::search_bambu_studio_config().await {
                Ok(path) => {
                    set_bambu_path.set(path.clone());
                    if let Ok(validation) = commands::validate_bambu_studio_path(&path).await {
                        set_path_valid.set(validation.valid);
                        set_path_status.set(Some(validation.message));
                    }
                }
                Err(e) => {
                    set_path_status.set(Some(e));
                    set_path_valid.set(false);
                }
            }
            set_is_searching_path.set(false);
        });
    };

    let on_browse_path = move |_| {
        spawn_local(async move {
            #[allow(unused_imports)]
            use wasm_bindgen::prelude::*;
            let args = serde_wasm_bindgen::to_value(&serde_json::json!({
                "title": "Select Bambu Studio Configuration Folder",
                "directory": true
            })).unwrap();

            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
                async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
            }

            match invoke("plugin:dialog|open", args).await {
                Ok(result) => {
                    if let Some(path_str) = result.as_string() {
                        set_bambu_path.set(path_str.clone());
                        if let Ok(validation) = commands::validate_bambu_studio_path(&path_str).await {
                            set_path_valid.set(validation.valid);
                            set_path_status.set(Some(validation.message));
                        }
                    }
                }
                Err(_) => {}
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

    let save_local_url = move |_| {
        let url = local_url.get();
        spawn_local(async move {
            match commands::set_preference("local_mcp_url", &url).await {
                Ok(()) => set_local_url_status.set(Some("Local server URL saved".to_string())),
                Err(e) => set_local_url_status.set(Some(format!("Failed: {}", e))),
            }
        });
    };

    let on_toggle_profiles = move |ev: leptos::ev::Event| {
        let checked = event_target_checked(&ev);
        let value = if checked { "true" } else { "false" };
        let mut new_flags = ff_ctx.flags.get();
        new_flags.profiles_enabled = checked;
        ff_ctx.set_flags.set(new_flags);
        spawn_local(async move {
            let _ = commands::set_preference("feature_profiles_enabled", value).await;
        });
    };

    // Open external URL helper
    let open_url_handler = |url: &'static str| {
        move |ev: leptos::ev::MouseEvent| {
            ev.prevent_default();
            let url = url.to_string();
            spawn_local(async move {
                let _ = commands::open_external_url(&url).await;
            });
        }
    };

    view! {
        <div class="page settings-page">
            <h2>"Settings"</h2>

            <section class="settings-section">
                <h3>"Feature Modules"</h3>
                <p class="section-description">"Enable or disable application features."</p>

                <div class="form-group feature-toggle">
                    <label class="toggle-label">
                        <input
                            type="checkbox"
                            class="toggle-input"
                            prop:checked=move || ff_ctx.flags.get().profiles_enabled
                            on:change=on_toggle_profiles
                        />
                        <span class="toggle-text">"Filament Profiles"</span>
                    </label>
                    <p class="toggle-description">"Search filament specs from manufacturers, generate optimized Bambu Studio profiles, and manage installed profiles."</p>
                </div>
            </section>

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
                <p class="section-description">"API keys are stored securely in your system keychain."</p>

                <ApiKeyForm
                    service_name="Claude API Key"
                    service_id="bambumate-claude-api"
                    placeholder="sk-ant-..."
                />
                <span class="status-text">
                    <button class="link-btn" on:click=open_url_handler("https://console.anthropic.com/account/keys")>"Get Claude API key"</button>
                </span>
                <ApiKeyForm
                    service_name="OpenAI API Key"
                    service_id="bambumate-openai-api"
                    placeholder="sk-..."
                />
                <span class="status-text">
                    <button class="link-btn" on:click=open_url_handler("https://platform.openai.com/api-keys")>"Get OpenAI API key"</button>
                </span>
                <ApiKeyForm
                    service_name="Kimi K2 API Key"
                    service_id="bambumate-kimi-api"
                    placeholder="sk-..."
                />
                <span class="status-text">
                    <button class="link-btn" on:click=open_url_handler("https://platform.moonshot.cn/console/api-keys")>"Get Kimi API key"</button>
                </span>
                <ApiKeyForm
                    service_name="OpenRouter API Key"
                    service_id="bambumate-openrouter-api"
                    placeholder="sk-or-..."
                />
                <span class="status-text">
                    <button class="link-btn" on:click=open_url_handler("https://openrouter.ai/keys")>"Get OpenRouter API key"</button>
                </span>
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
                        <option value="local" selected=move || ai_provider.get() == "local">"Local Server"</option>
                    </select>
                </div>

                <Show when=move || ai_provider.get() == "local">
                    <div class="form-group">
                        <label for="local-url">"Local Server URL"</label>
                        <p class="section-description">"URL of your OpenAI-compatible local server (LM Studio, Ollama, llama.cpp, etc.)."</p>
                        <div class="input-row">
                            <input
                                id="local-url"
                                type="text"
                                placeholder="http://localhost:1234"
                                class="input"
                                prop:value=move || local_url.get()
                                on:input=move |ev| {
                                    set_local_url.set(event_target_value(&ev));
                                }
                            />
                            <button class="btn btn-save" on:click=save_local_url>"Save"</button>
                        </div>
                        <Show when=move || local_url_status.get().is_some()>
                            <span class="status-text">{move || local_url_status.get().unwrap_or_default()}</span>
                        </Show>
                    </div>
                </Show>

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
                    <label for="bambu-path">"Bambu Studio Configuration Path"</label>
                    <p class="section-description">"The folder where Bambu Studio stores profiles. On Windows: %APPDATA%\\BambuStudio"</p>
                    <div class="input-with-buttons">
                        <input
                            id="bambu-path"
                            type="text"
                            placeholder="e.g. C:\\Users\\You\\AppData\\Roaming\\BambuStudio"
                            class="input"
                            prop:value=move || bambu_path.get()
                            on:input=move |ev| {
                                set_bambu_path.set(event_target_value(&ev));
                            }
                        />
                        <button
                            class="btn btn-secondary btn-sm"
                            on:click=on_search_path
                            disabled=move || is_searching_path.get()
                        >
                            {move || if is_searching_path.get() { "..." } else { "Search" }}
                        </button>
                        <button class="btn btn-secondary btn-sm" on:click=on_browse_path>"Browse"</button>
                        <button class="btn btn-save" on:click=save_bambu_path>"Save"</button>
                    </div>
                    <Show when=move || path_status.get().is_some()>
                        <span class={move || if path_valid.get() { "status-text status-success" } else { "status-text status-warning" }}>
                            {move || path_status.get().unwrap_or_default()}
                        </span>
                    </Show>
                </div>

                <div class="form-group">
                    <label for="stl-watch-dir">"STL Watch Directory"</label>
                    <p class="section-description">"Watch a folder for incoming .stl files from OpenSCAD Studio."</p>
                    <div class="input-row">
                        <input
                            id="stl-watch-dir"
                            type="text"
                            placeholder="/path/to/stl/output"
                            class="input"
                            prop:value=move || stl_watch_dir.get()
                            on:input=move |ev| {
                                set_stl_watch_dir.set(event_target_value(&ev));
                            }
                        />
                        <button class="btn btn-save" on:click=save_stl_watch_dir>"Save"</button>
                    </div>
                    <Show when=move || stl_status.get().is_some()>
                        <span class="status-text">{move || stl_status.get().unwrap_or_default()}</span>
                    </Show>
                </div>
            </section>
        </div>
    }
}

/// Helper to extract checked state from a checkbox change event.
fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}
