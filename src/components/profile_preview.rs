use leptos::prelude::*;

use crate::commands::GenerateResult;

#[component]
pub fn ProfilePreview(
    result: GenerateResult,
    #[prop(into)] on_install: Callback<()>,
    #[prop(into)] on_cancel: Callback<()>,
    #[prop(default = false)] installing: bool,
) -> impl IntoView {
    let has_warnings = !result.warnings.is_empty();
    let bs_running = result.bambu_studio_running;
    let warnings = result.warnings.clone();

    let nozzle_temp = result
        .specs_applied
        .nozzle_temp
        .clone()
        .unwrap_or_else(|| "--".to_string());
    let bed_temp = result
        .specs_applied
        .bed_temp
        .clone()
        .unwrap_or_else(|| "--".to_string());
    let fan_speed = result
        .specs_applied
        .fan_speed
        .clone()
        .unwrap_or_else(|| "--".to_string());
    let retraction = result
        .specs_applied
        .retraction
        .clone()
        .unwrap_or_else(|| "--".to_string());

    view! {
        <div class="profile-preview">
            <h3 class="profile-preview-title">"Profile Preview"</h3>

            <Show when=move || bs_running>
                <div class="warning-banner warning-bs-running">
                    "Bambu Studio is running. Restart BS after installation for changes to take effect."
                </div>
            </Show>

            <Show when=move || has_warnings>
                <div class="warning-banner">
                    <ul class="warning-list">
                        {warnings.clone().into_iter().map(|w| {
                            view! { <li>{w}</li> }
                        }).collect::<Vec<_>>()}
                    </ul>
                </div>
            </Show>

            <div class="profile-preview-info">
                <div class="preview-row">
                    <span class="preview-label">"Profile Name"</span>
                    <span class="preview-value profile-name">{result.profile_name.clone()}</span>
                </div>
                <div class="preview-row">
                    <span class="preview-label">"Base Profile"</span>
                    <span class="preview-value">{result.base_profile_used.clone()}</span>
                </div>
                <div class="preview-row">
                    <span class="preview-label">"Field Count"</span>
                    <span class="preview-value">{result.field_count}</span>
                </div>
                <div class="preview-row">
                    <span class="preview-label">"Filename"</span>
                    <span class="preview-value filename">{result.filename.clone()}</span>
                </div>
            </div>

            <div class="profile-preview-specs">
                <h4>"Applied Settings"</h4>
                <div class="preview-specs-grid">
                    <div class="spec-item">
                        <span class="spec-label">"Nozzle Temp"</span>
                        <span class="spec-value">{nozzle_temp}</span>
                    </div>
                    <div class="spec-item">
                        <span class="spec-label">"Bed Temp"</span>
                        <span class="spec-value">{bed_temp}</span>
                    </div>
                    <div class="spec-item">
                        <span class="spec-label">"Fan Speed"</span>
                        <span class="spec-value">{fan_speed}</span>
                    </div>
                    <div class="spec-item">
                        <span class="spec-label">"Retraction"</span>
                        <span class="spec-value">{retraction}</span>
                    </div>
                </div>
            </div>

            <div class="profile-preview-actions">
                <button
                    class="btn btn-secondary"
                    on:click=move |_| on_cancel.run(())
                    disabled=installing
                >
                    "Cancel"
                </button>
                <button
                    class="btn btn-primary"
                    on:click=move |_| on_install.run(())
                    disabled=installing
                >
                    {if installing { "Installing..." } else { "Install Profile" }}
                </button>
            </div>
        </div>
    }
}
