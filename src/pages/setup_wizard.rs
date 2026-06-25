use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::commands;

/// Provider info for the wizard selection UI.
struct ProviderInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    signup_url: &'static str,
    keychain_service: &'static str,
}

const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "claude",
        name: "Anthropic Claude",
        description: "Excellent at structured extraction and vision analysis. Recommended for best results.",
        signup_url: "https://console.anthropic.com/account/keys",
        keychain_service: "bambumate-claude-api",
    },
    ProviderInfo {
        id: "openai",
        name: "OpenAI",
        description: "Strong general-purpose AI with good structured output support.",
        signup_url: "https://platform.openai.com/api-keys",
        keychain_service: "bambumate-openai-api",
    },
    ProviderInfo {
        id: "openrouter",
        name: "OpenRouter",
        description: "Access multiple AI models through a single API. Pay-per-use pricing.",
        signup_url: "https://openrouter.ai/keys",
        keychain_service: "bambumate-openrouter-api",
    },
    ProviderInfo {
        id: "kimi",
        name: "Kimi (Moonshot)",
        description: "AI provider with large context window support.",
        signup_url: "https://platform.moonshot.cn/console/api-keys",
        keychain_service: "bambumate-kimi-api",
    },
    ProviderInfo {
        id: "local",
        name: "Local Server",
        description: "Use a local OpenAI-compatible server (LM Studio, Ollama, etc.). No API key required.",
        signup_url: "",
        keychain_service: "",
    },
];

#[component]
pub fn SetupWizard(
    #[prop(into)] on_complete: Callback<()>,
    #[prop(into)] on_cancel: Callback<()>,
) -> impl IntoView {
    let step = RwSignal::new(0u8);
    // None = not yet chosen, Some(true) = use AI, Some(false) = web-only
    let use_ai_mode: RwSignal<Option<bool>> = RwSignal::new(None);
    let bambu_path = RwSignal::new(String::new());
    let bambu_detected = RwSignal::new(false);
    let path_validation_msg = RwSignal::new(String::new());
    let path_valid = RwSignal::new(false);
    let is_searching_path = RwSignal::new(false);
    let selected_provider = RwSignal::new(String::new());
    let api_key_input = RwSignal::new(String::new());
    let local_url_input = RwSignal::new("http://localhost:1234".to_string());
    let selected_model = RwSignal::new(String::new());
    let available_models = RwSignal::new(Vec::<commands::ModelInfo>::new());
    let is_loading_models = RwSignal::new(false);
    let model_error = RwSignal::new(String::new());
    let saving = RwSignal::new(false);
    let error_msg = RwSignal::new(String::new());
    let clean_install = RwSignal::new(false);
    let clean_install_status = RwSignal::new(String::new());

    // Step 0: Auto-detect Bambu Studio config path on mount
    Effect::new(move || {
        spawn_local(async move {
            is_searching_path.set(true);

            // First, check if there's a previously saved preference
            if let Ok(Some(saved_path)) = commands::get_preference("bambu_studio_path").await {
                if !saved_path.is_empty() {
                    bambu_path.set(saved_path.clone());
                    bambu_detected.set(true);
                    if let Ok(validation) = commands::validate_bambu_studio_path(&saved_path).await {
                        path_valid.set(validation.valid);
                        path_validation_msg.set(validation.message);
                    }
                    is_searching_path.set(false);
                    return;
                }
            }

            // No saved preference — try auto-detect
            match commands::search_bambu_studio_config().await {
                Ok(path) => {
                    bambu_path.set(path.clone());
                    bambu_detected.set(true);
                    // Validate the detected path
                    if let Ok(validation) = commands::validate_bambu_studio_path(&path).await {
                        path_valid.set(validation.valid);
                        path_validation_msg.set(validation.message);
                    }
                }
                Err(_) => {
                    // Fallback: try health check for the installed app path
                    if let Ok(report) = commands::run_health_check().await {
                        if let Some(ref profile_path) = report.profile_dir_path {
                            bambu_path.set(profile_path.clone());
                            bambu_detected.set(true);
                            if let Ok(validation) = commands::validate_bambu_studio_path(profile_path).await {
                                path_valid.set(validation.valid);
                                path_validation_msg.set(validation.message);
                            }
                        }
                    }
                }
            }
            is_searching_path.set(false);
        });
    });

    // Handler for the "Search" button
    let on_search_path = move |_| {
        is_searching_path.set(true);
        error_msg.set(String::new());
        path_validation_msg.set(String::new());
        spawn_local(async move {
            match commands::search_bambu_studio_config().await {
                Ok(path) => {
                    bambu_path.set(path.clone());
                    bambu_detected.set(true);
                    if let Ok(validation) = commands::validate_bambu_studio_path(&path).await {
                        path_valid.set(validation.valid);
                        path_validation_msg.set(validation.message);
                    }
                }
                Err(e) => {
                    path_validation_msg.set(e);
                    path_valid.set(false);
                }
            }
            is_searching_path.set(false);
        });
    };

    // Handler for the "Browse" button using Tauri dialog
    let on_browse_path = move |_| {
        spawn_local(async move {
            // Call the Tauri dialog plugin to open a folder picker
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
                        bambu_path.set(path_str.clone());
                        bambu_detected.set(true);
                        // Validate the selected path
                        if let Ok(validation) = commands::validate_bambu_studio_path(&path_str).await {
                            path_valid.set(validation.valid);
                            path_validation_msg.set(validation.message);
                        }
                    }
                }
                Err(_) => {
                    // User cancelled or error - do nothing
                }
            }
        });
    };

    // Validate path when input changes
    let on_path_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
        let path = target.value();
        bambu_path.set(path.clone());
        if path.is_empty() {
            path_validation_msg.set(String::new());
            path_valid.set(false);
            return;
        }
        spawn_local(async move {
            if let Ok(validation) = commands::validate_bambu_studio_path(&path).await {
                path_valid.set(validation.valid);
                path_validation_msg.set(validation.message);
            }
        });
    };

    // Open external URL handler
    let open_url = |url: &'static str| {
        move |ev: web_sys::MouseEvent| {
            ev.prevent_default();
            ev.stop_propagation();
            let url = url.to_string();
            spawn_local(async move {
                let _ = commands::open_external_url(&url).await;
            });
        }
    };

    // Load models when entering step 3
    let load_models = move || {
        let provider = selected_provider.get();
        if provider.is_empty() {
            return;
        }
        is_loading_models.set(true);
        model_error.set(String::new());
        available_models.set(vec![]);
        selected_model.set(String::new());
        spawn_local(async move {
            match commands::list_models(&provider).await {
                Ok(models) => {
                    available_models.set(models);
                }
                Err(e) => {
                    model_error.set(e);
                }
            }
            is_loading_models.set(false);
        });
    };

    let on_next = move |_| {
        let current = step.get();
        error_msg.set(String::new());

        // Step 1 with web-only mode: save prefs and complete wizard immediately
        if current == 1 && use_ai_mode.get() == Some(false) {
            saving.set(true);
            let path = bambu_path.get();
            let on_complete = on_complete.clone();
            spawn_local(async move {
                if !path.is_empty() {
                    if let Err(e) = commands::set_preference("bambu_studio_path", &path).await {
                        error_msg.set(format!("Failed to save path: {}", e));
                        saving.set(false);
                        return;
                    }
                }
                if let Err(e) = commands::set_preference("filament_search_use_ai", "false").await {
                    error_msg.set(format!("Failed to save setting: {}", e));
                    saving.set(false);
                    return;
                }
                if let Err(e) = commands::set_preference("setup_complete", "true").await {
                    error_msg.set(format!("Failed to complete setup: {}", e));
                    saving.set(false);
                    return;
                }
                saving.set(false);
                on_complete.run(());
            });
            return;
        }

        if current < 4 {
            // Step 3 (API key entry): save key before advancing to model selection
            if current == 3 {
                let provider = selected_provider.get();
                let key = api_key_input.get();
                let local_url = local_url_input.get();
                spawn_local(async move {
                    if provider == "local" {
                        let _ = commands::set_preference("local_mcp_url", &local_url).await;
                    } else {
                        let service = PROVIDERS
                            .iter()
                            .find(|p| p.id == provider)
                            .map(|p| p.keychain_service)
                            .unwrap_or("");
                        if !service.is_empty() && !key.is_empty() {
                            if let Err(e) = commands::set_api_key(service, &key).await {
                                error_msg.set(format!("Failed to save API key: {}", e));
                                return;
                            }
                        }
                    }
                    step.set(current + 1);
                    load_models();
                });
            } else {
                step.set(current + 1);
            }
        }
    };

    let on_back = move |_| {
        let current = step.get();
        if current > 0 {
            error_msg.set(String::new());
            step.set(current - 1);
        }
    };

    let on_finish = move |_| {
        saving.set(true);
        error_msg.set(String::new());

        let provider = selected_provider.get();
        let key = api_key_input.get();
        let path = bambu_path.get();
        let local_url = local_url_input.get();
        let model = selected_model.get();
        let is_clean_install = clean_install.get();
        let on_complete = on_complete.clone();

        spawn_local(async move {
            // If clean install is checked, reset everything first
            if is_clean_install {
                if let Err(e) = commands::reset_to_clean_install().await {
                    error_msg.set(format!("Failed to reset for clean install: {}", e));
                    saving.set(false);
                    return;
                }
            }

            // Save Bambu Studio path (validate first)
            if !path.is_empty() {
                if let Ok(validation) = commands::validate_bambu_studio_path(&path).await {
                    if !validation.valid {
                        error_msg.set(format!("Warning: {}", validation.message));
                    }
                }
                if let Err(e) = commands::set_preference("bambu_studio_path", &path).await {
                    error_msg.set(format!("Failed to save Bambu Studio path: {}", e));
                    saving.set(false);
                    return;
                }
            }

            // Save AI provider preference
            if let Err(e) = commands::set_preference("ai_provider", &provider).await {
                error_msg.set(format!("Failed to save AI provider: {}", e));
                saving.set(false);
                return;
            }

            // Save API key (except for local provider)
            if provider != "local" {
                let service = PROVIDERS
                    .iter()
                    .find(|p| p.id == provider)
                    .map(|p| p.keychain_service)
                    .unwrap_or("");

                if !service.is_empty() && !key.is_empty() {
                    if let Err(e) = commands::set_api_key(service, &key).await {
                        error_msg.set(format!("Failed to save API key: {}", e));
                        saving.set(false);
                        return;
                    }
                }
            } else {
                // Save local server URL
                if let Err(e) = commands::set_preference("local_mcp_url", &local_url).await {
                    error_msg.set(format!("Failed to save local server URL: {}", e));
                    saving.set(false);
                    return;
                }
            }

            // Save selected model
            if !model.is_empty() {
                if let Err(e) = commands::set_preference("ai_model", &model).await {
                    error_msg.set(format!("Failed to save model selection: {}", e));
                    saving.set(false);
                    return;
                }
            }

            // Explicitly record that AI is enabled so feature flags are correct
            if let Err(e) = commands::set_preference("filament_search_use_ai", "true").await {
                error_msg.set(format!("Failed to save AI setting: {}", e));
                saving.set(false);
                return;
            }

            // Mark setup as complete
            if let Err(e) = commands::set_preference("setup_complete", "true").await {
                error_msg.set(format!("Failed to mark setup complete: {}", e));
                saving.set(false);
                return;
            }

            saving.set(false);
            on_complete.run(());
        });
    };

    let can_finish = move || {
        let provider = selected_provider.get();
        if provider.is_empty() {
            return false;
        }
        if selected_model.get().is_empty() {
            return false;
        }
        if provider == "local" {
            return !local_url_input.get().is_empty();
        }
        !api_key_input.get().is_empty()
    };

    view! {
        <div class="wizard-overlay">
            <div class="wizard-container">
                <div class="wizard-header">
                    <h2>"BambuMate Setup"</h2>
                    <div class="wizard-steps-indicator">
                        <span class={move || if step.get() == 0 { "wizard-dot active" } else if step.get() > 0 { "wizard-dot completed" } else { "wizard-dot" }}></span>
                        <span class="wizard-dot-line"></span>
                        <span class={move || if step.get() == 1 { "wizard-dot active" } else if step.get() > 1 { "wizard-dot completed" } else { "wizard-dot" }}></span>
                        <span class="wizard-dot-line"></span>
                        <span class={move || if step.get() == 2 { "wizard-dot active" } else if step.get() > 2 { "wizard-dot completed" } else { "wizard-dot" }}></span>
                        <span class="wizard-dot-line"></span>
                        <span class={move || if step.get() == 3 { "wizard-dot active" } else if step.get() > 3 { "wizard-dot completed" } else { "wizard-dot" }}></span>
                        <span class="wizard-dot-line"></span>
                        <span class={move || if step.get() == 4 { "wizard-dot active" } else { "wizard-dot" }}></span>
                    </div>
                </div>

                <div class="wizard-body">
                    // Step 0: Welcome + Bambu Studio detection
                    <Show when=move || step.get() == 0>
                        <div class="wizard-step">
                            <h3>"Welcome to BambuMate"</h3>
                            <p class="wizard-description">
                                "BambuMate helps you optimize your Bambu Studio filament profiles using AI-powered analysis. "
                                "Let's get you set up in a few quick steps."
                            </p>

                            <div class="wizard-section">
                                <h4>"Bambu Studio Configuration Folder"</h4>
                                <p class="wizard-description">
                                    "This is the folder where Bambu Studio stores its profiles and settings. "
                                    "On Windows this is usually %APPDATA%\\BambuStudio, on macOS ~/Library/Application Support/BambuStudio."
                                </p>
                                <Show when=move || bambu_detected.get() && path_valid.get()>
                                    <div class="wizard-status wizard-status-success">
                                        "Bambu Studio configuration detected and validated"
                                    </div>
                                </Show>
                                <Show when=move || bambu_detected.get() && !path_valid.get()>
                                    <div class="wizard-status wizard-status-warning">
                                        {move || path_validation_msg.get()}
                                    </div>
                                </Show>
                                <Show when=move || !bambu_detected.get() && !is_searching_path.get()>
                                    <div class="wizard-status wizard-status-warning">
                                        "Bambu Studio was not detected automatically. Use Search to find it, or Browse to select manually."
                                    </div>
                                </Show>
                                <Show when=move || is_searching_path.get()>
                                    <div class="wizard-status">
                                        "Searching for Bambu Studio..."
                                    </div>
                                </Show>
                                <div class="form-group">
                                    <label>"Configuration Path"</label>
                                    <div class="input-with-buttons">
                                        <input
                                            type="text"
                                            class="input"
                                            placeholder="e.g. C:\\Users\\You\\AppData\\Roaming\\BambuStudio"
                                            prop:value=move || bambu_path.get()
                                            on:input=on_path_input
                                        />
                                        <button
                                            class="btn btn-secondary btn-sm"
                                            on:click=on_search_path
                                            disabled=move || is_searching_path.get()
                                        >
                                            {move || if is_searching_path.get() { "..." } else { "Search" }}
                                        </button>
                                        <button
                                            class="btn btn-secondary btn-sm"
                                            on:click=on_browse_path
                                        >"Browse"</button>
                                    </div>
                                </div>
                                <Show when=move || !path_validation_msg.get().is_empty() && !bambu_detected.get()>
                                    <span class={move || if path_valid.get() { "status-text status-success" } else { "status-text status-warning" }}>
                                        {move || path_validation_msg.get()}
                                    </span>
                                </Show>
                            </div>

                            <div class="wizard-section">
                                <h4>"Clean Installation"</h4>
                                <div class="form-group feature-toggle">
                                    <label class="toggle-label">
                                        <input
                                            type="checkbox"
                                            class="toggle-input"
                                            prop:checked=move || clean_install.get()
                                            on:change=move |ev| {
                                                use wasm_bindgen::JsCast;
                                                let checked = ev.target()
                                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                                    .map(|el| el.checked())
                                                    .unwrap_or(false);
                                                clean_install.set(checked);
                                                if checked {
                                                    clean_install_status.set("All existing preferences and API keys will be cleared on finish.".to_string());
                                                } else {
                                                    clean_install_status.set(String::new());
                                                }
                                            }
                                        />
                                        <span class="toggle-text">"Reset all settings for a clean installation"</span>
                                    </label>
                                    <p class="toggle-description">
                                        "Check this to clear all existing BambuMate preferences and stored API keys before setting up. "
                                        "Use this if you want to start fresh."
                                    </p>
                                    <Show when=move || !clean_install_status.get().is_empty()>
                                        <span class="status-text status-warning">{move || clean_install_status.get()}</span>
                                    </Show>
                                </div>
                            </div>
                        </div>
                    </Show>

                    // Step 1: AI mode choice
                    <Show when=move || step.get() == 1>
                        <div class="wizard-step">
                            <h3>"How would you like to find filament specs?"</h3>
                            <p class="wizard-description">
                                "BambuMate can use an AI model to intelligently extract filament settings "
                                "and analyze print quality, or it can pull specs directly from manufacturer "
                                "websites without any AI or API key."
                            </p>

                            <div class="wizard-mode-cards">
                                <div
                                    class={move || if use_ai_mode.get() == Some(true) { "wizard-mode-card selected" } else { "wizard-mode-card" }}
                                    on:click=move |_| use_ai_mode.set(Some(true))
                                >
                                    <div class="wizard-mode-icon">"🤖"</div>
                                    <h4>"Use AI (Recommended)"</h4>
                                    <p>
                                        "Use an AI model with your API key to intelligently extract filament "
                                        "specs and detect print defects. Best accuracy, especially for "
                                        "niche or specialty filaments."
                                    </p>
                                    <ul class="wizard-mode-features">
                                        <li>"✓ Filament profiles from AI knowledge"</li>
                                        <li>"✓ Web scraping with AI extraction"</li>
                                        <li>"✓ Print Analysis (defect detection)"</li>
                                    </ul>
                                    <p class="wizard-mode-note">"Requires a paid API key (Claude, OpenAI, etc.)"</p>
                                </div>

                                <div
                                    class={move || if use_ai_mode.get() == Some(false) { "wizard-mode-card selected" } else { "wizard-mode-card" }}
                                    on:click=move |_| use_ai_mode.set(Some(false))
                                >
                                    <div class="wizard-mode-icon">"🌐"</div>
                                    <h4>"Use Manufacturer Specs"</h4>
                                    <p>
                                        "Pull specs directly from manufacturer websites and SpoolScout. "
                                        "No API key required — free to use."
                                    </p>
                                    <ul class="wizard-mode-features">
                                        <li>"✓ Filament profiles from web sources"</li>
                                        <li>"✓ SpoolScout catalog lookup"</li>
                                        <li>"✗ Print Analysis (requires AI)"</li>
                                    </ul>
                                    <p class="wizard-mode-note">"Free — no API key required"</p>
                                </div>
                            </div>

                            <p class="wizard-description wizard-mode-footer">
                                "You can change this at any time in Settings."
                            </p>
                        </div>
                    </Show>

                    // Step 2: AI Provider selection (was step 1)
                    <Show when=move || step.get() == 2>
                        <div class="wizard-step">
                            <h3>"AI Provider"</h3>
                            <p class="wizard-description">
                                "BambuMate uses AI to analyze filament specifications and detect print defects. "
                                "Choose a provider below."
                            </p>

                            <div class="wizard-providers">
                                {PROVIDERS.iter().map(|provider| {
                                    let pid = provider.id;
                                    let pname = provider.name;
                                    let pdesc = provider.description;
                                    let signup = provider.signup_url;
                                    view! {
                                        <div
                                            class={move || {
                                                if selected_provider.get() == pid {
                                                    "wizard-provider-card selected"
                                                } else {
                                                    "wizard-provider-card"
                                                }
                                            }}
                                            on:click=move |_| {
                                                selected_provider.set(pid.to_string());
                                                api_key_input.set(String::new());
                                                error_msg.set(String::new());
                                                selected_model.set(String::new());
                                                available_models.set(vec![]);
                                            }
                                        >
                                            <div class="wizard-provider-header">
                                                <strong>{pname}</strong>
                                                <Show when=move || !signup.is_empty()>
                                                    <button
                                                        class="wizard-signup-link"
                                                        on:click=open_url(signup)
                                                    >"Get API Key"</button>
                                                </Show>
                                            </div>
                                            <p class="wizard-provider-desc">{pdesc}</p>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    </Show>

                    // Step 3: API Key input or Local server URL (was step 2)
                    <Show when=move || step.get() == 3>
                        <div class="wizard-step">
                            <Show when=move || selected_provider.get() == "local">
                                <h3>"Local Server Configuration"</h3>
                                <p class="wizard-description">
                                    "Enter the URL of your local OpenAI-compatible server. "
                                    "This works with LM Studio, Ollama, llama.cpp, and similar tools. "
                                    "Make sure your server is running before using BambuMate."
                                </p>
                                <div class="form-group">
                                    <label>"Server URL"</label>
                                    <input
                                        type="text"
                                        class="input"
                                        placeholder="http://localhost:1234"
                                        prop:value=move || local_url_input.get()
                                        on:input=move |ev| {
                                            use wasm_bindgen::JsCast;
                                            let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                                            local_url_input.set(target.value());
                                        }
                                    />
                                    <span class="status-text">
                                        "Default ports: LM Studio (1234), Ollama (11434)"
                                    </span>
                                </div>
                            </Show>
                            <Show when=move || selected_provider.get() != "local">
                                <h3>"Enter API Key"</h3>
                                <p class="wizard-description">
                                    {move || {
                                        let provider = selected_provider.get();
                                        let info = PROVIDERS.iter().find(|p| p.id == provider);
                                        match info {
                                            Some(p) if !p.signup_url.is_empty() => {
                                                format!(
                                                    "Enter your {} API key below. Click the button to get one if you don't have one.",
                                                    p.name
                                                )
                                            }
                                            _ => "Enter your API key below.".to_string(),
                                        }
                                    }}
                                </p>

                                {move || {
                                    let provider = selected_provider.get();
                                    let info = PROVIDERS.iter().find(|p| p.id == provider);
                                    match info {
                                        Some(p) if !p.signup_url.is_empty() => {
                                            let url = p.signup_url;
                                            let name = p.name;
                                            Some(view! {
                                                <button
                                                    class="wizard-signup-link wizard-signup-link-standalone"
                                                    on:click=open_url(url)
                                                >{format!("Sign up for {} API key", name)}</button>
                                            })
                                        }
                                        _ => None,
                                    }
                                }}

                                <div class="form-group">
                                    <label>"API Key"</label>
                                    <input
                                        type="password"
                                        class="input input-password"
                                        placeholder="sk-..."
                                        prop:value=move || api_key_input.get()
                                        on:input=move |ev| {
                                            use wasm_bindgen::JsCast;
                                            let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                                            api_key_input.set(target.value());
                                        }
                                    />
                                    <span class="status-text">
                                        "Your API key is stored securely in your system keychain."
                                    </span>
                                </div>
                            </Show>
                        </div>
                    </Show>

                    // Step 4: Model Selection (was step 3)
                    <Show when=move || step.get() == 4>
                        <div class="wizard-step">
                            <h3>"Select AI Model"</h3>
                            <p class="wizard-description">
                                "Choose which model to use for filament analysis and profile generation."
                            </p>

                            <Show when=move || is_loading_models.get()>
                                <div class="wizard-status">
                                    "Loading available models..."
                                </div>
                            </Show>

                            <Show when=move || !model_error.get().is_empty()>
                                <div class="wizard-status wizard-status-warning">
                                    {move || model_error.get()}
                                </div>
                                <button
                                    class="btn btn-secondary btn-sm"
                                    on:click=move |_| load_models()
                                >"Retry"</button>
                            </Show>

                            <Show when=move || !is_loading_models.get() && model_error.get().is_empty()>
                                <div class="form-group">
                                    <label>"Model"</label>
                                    <select
                                        class="input"
                                        prop:value=move || selected_model.get()
                                        on:change=move |ev| {
                                            use wasm_bindgen::JsCast;
                                            let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlSelectElement>();
                                            selected_model.set(target.value());
                                        }
                                    >
                                        <option value="">"-- Select a model --"</option>
                                        {move || available_models.get().iter().map(|m| {
                                            let id = m.id.clone();
                                            let name = m.name.clone();
                                            view! {
                                                <option value={id.clone()}>{name}</option>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </Show>
                        </div>
                    </Show>

                    // Error message
                    <Show when=move || !error_msg.get().is_empty()>
                        <div class="wizard-error">
                            {move || error_msg.get()}
                        </div>
                    </Show>
                </div>

                <div class="wizard-footer">
                    <Show when=move || { step.get() > 0 }>
                        <button
                            class="btn btn-secondary"
                            on:click=on_back
                            disabled=move || saving.get()
                        >"Back"</button>
                    </Show>
                    <button
                        class="btn btn-ghost wizard-skip-btn"
                        on:click=move |_| on_cancel.run(())
                        disabled=move || saving.get()
                    >"Skip Setup"</button>
                    <div class="wizard-footer-spacer"></div>
                    <Show when=move || step.get() < 4>
                        <button
                            class="btn btn-primary"
                            on:click=on_next
                            disabled=move || {
                                saving.get() ||
                                (step.get() == 1 && use_ai_mode.get().is_none()) ||
                                (step.get() == 2 && selected_provider.get().is_empty()) ||
                                (step.get() == 3 && selected_provider.get() != "local" && api_key_input.get().is_empty()) ||
                                (step.get() == 3 && selected_provider.get() == "local" && local_url_input.get().is_empty())
                            }
                        >
                            {move || {
                                if saving.get() {
                                    "Saving..."
                                } else if step.get() == 1 && use_ai_mode.get() == Some(false) {
                                    "Finish Setup"
                                } else {
                                    "Next"
                                }
                            }}
                        </button>
                    </Show>
                    <Show when=move || step.get() == 4>
                        <button
                            class="btn btn-primary"
                            on:click=on_finish
                            disabled=move || !can_finish() || saving.get()
                        >
                            {move || if saving.get() { "Saving..." } else { "Finish Setup" }}
                        </button>
                    </Show>
                </div>
            </div>
        </div>
    }
}
