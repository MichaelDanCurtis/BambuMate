use leptos::prelude::*;

#[component]
pub fn SettingsPage() -> impl IntoView {
    // Signals for API key fields
    let (claude_key, set_claude_key) = signal(String::new());
    let (openai_key, set_openai_key) = signal(String::new());
    let (claude_status, set_claude_status) = signal("Not set".to_string());
    let (openai_status, set_openai_status) = signal("Not set".to_string());
    let (bambu_path, set_bambu_path) = signal(String::new());

    // Placeholder save handlers -- will be wired to backend in Plan 02
    let save_claude_key = move |_| {
        let _key = claude_key.get();
        set_claude_status.set("Save not yet wired".to_string());
    };

    let save_openai_key = move |_| {
        let _key = openai_key.get();
        set_openai_status.set("Save not yet wired".to_string());
    };

    let save_bambu_path = move |_| {
        let _path = bambu_path.get();
    };

    view! {
        <div class="page settings-page">
            <h2>"Settings"</h2>

            <section class="settings-section">
                <h3>"API Keys"</h3>
                <p class="section-description">"API keys are stored securely in your system keychain."</p>

                <div class="form-group">
                    <label for="claude-key">"Claude API Key"</label>
                    <div class="input-row">
                        <input
                            id="claude-key"
                            type="password"
                            placeholder="sk-ant-..."
                            class="input"
                            prop:value=move || claude_key.get()
                            on:input=move |ev| {
                                set_claude_key.set(event_target_value(&ev));
                            }
                        />
                        <button class="btn btn-primary" on:click=save_claude_key>"Save"</button>
                    </div>
                    <span class="status-text">{move || claude_status.get()}</span>
                </div>

                <div class="form-group">
                    <label for="openai-key">"OpenAI API Key"</label>
                    <div class="input-row">
                        <input
                            id="openai-key"
                            type="password"
                            placeholder="sk-..."
                            class="input"
                            prop:value=move || openai_key.get()
                            on:input=move |ev| {
                                set_openai_key.set(event_target_value(&ev));
                            }
                        />
                        <button class="btn btn-primary" on:click=save_openai_key>"Save"</button>
                    </div>
                    <span class="status-text">{move || openai_status.get()}</span>
                </div>
            </section>

            <section class="settings-section">
                <h3>"Preferences"</h3>

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
                        <button class="btn btn-secondary" on:click=save_bambu_path>"Save"</button>
                    </div>
                </div>
            </section>
        </div>
    }
}
