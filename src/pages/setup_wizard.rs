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
) -> impl IntoView {
    let step = RwSignal::new(0u8);
    let bambu_path = RwSignal::new(String::new());
    let bambu_detected = RwSignal::new(false);
    let selected_provider = RwSignal::new(String::new());
    let api_key_input = RwSignal::new(String::new());
    let local_url_input = RwSignal::new("http://localhost:1234".to_string());
    let saving = RwSignal::new(false);
    let error_msg = RwSignal::new(String::new());

    // Step 0: Auto-detect Bambu Studio on mount
    Effect::new(move || {
        spawn_local(async move {
            match commands::run_health_check().await {
                Ok(report) => {
                    if report.bambu_studio_installed {
                        if let Some(path) = report.bambu_studio_path {
                            bambu_path.set(path);
                            bambu_detected.set(true);
                        }
                    }
                }
                Err(_) => {}
            }
        });
    });

    let on_next = move |_| {
        let current = step.get();
        if current < 2 {
            error_msg.set(String::new());
            step.set(current + 1);
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
        let on_complete = on_complete.clone();

        spawn_local(async move {
            // Save Bambu Studio path
            if !path.is_empty() {
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
                        <span class={move || if step.get() == 2 { "wizard-dot active" } else { "wizard-dot" }}></span>
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
                                <h4>"Bambu Studio Location"</h4>
                                <Show when=move || bambu_detected.get()>
                                    <div class="wizard-status wizard-status-success">
                                        "Bambu Studio detected automatically"
                                    </div>
                                </Show>
                                <Show when=move || !bambu_detected.get()>
                                    <div class="wizard-status wizard-status-warning">
                                        "Bambu Studio was not detected in the default location. You can enter the path manually, or skip this step if it is not installed yet."
                                    </div>
                                </Show>
                                <div class="form-group">
                                    <label>"Application Path"</label>
                                    <input
                                        type="text"
                                        class="input"
                                        placeholder="/Applications/BambuStudio.app"
                                        prop:value=move || bambu_path.get()
                                        on:input=move |ev| {
                                            use wasm_bindgen::JsCast;
                                            let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                                            bambu_path.set(target.value());
                                        }
                                    />
                                </div>
                            </div>
                        </div>
                    </Show>

                    // Step 1: AI Provider explanation + selection
                    <Show when=move || step.get() == 1>
                        <div class="wizard-step">
                            <h3>"AI Provider"</h3>
                            <p class="wizard-description">
                                "BambuMate uses AI to analyze filament specifications and detect print defects. "
                                "An API key from one of the following providers is required. "
                                "Choose a provider and enter your API key below."
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
                                            }
                                        >
                                            <div class="wizard-provider-header">
                                                <strong>{pname}</strong>
                                                <Show when=move || !signup.is_empty()>
                                                    <a
                                                        href=signup
                                                        target="_blank"
                                                        rel="noopener noreferrer"
                                                        class="wizard-signup-link"
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                        }
                                                    >"Get API Key"</a>
                                                </Show>
                                            </div>
                                            <p class="wizard-provider-desc">{pdesc}</p>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    </Show>

                    // Step 2: API Key input or Local server URL
                    <Show when=move || step.get() == 2>
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
                                                    "Enter your {} API key below. If you don't have one, visit the signup page to create an account.",
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
                                            Some(view! {
                                                <a
                                                    href=url
                                                    target="_blank"
                                                    rel="noopener noreferrer"
                                                    class="wizard-signup-link wizard-signup-link-standalone"
                                                >{format!("Sign up for {} API key", p.name)}</a>
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
                    <div class="wizard-footer-spacer"></div>
                    <Show when=move || step.get() < 2>
                        <button
                            class="btn btn-primary"
                            on:click=on_next
                            disabled=move || step.get() == 1 && selected_provider.get().is_empty()
                        >"Next"</button>
                    </Show>
                    <Show when=move || step.get() == 2>
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
