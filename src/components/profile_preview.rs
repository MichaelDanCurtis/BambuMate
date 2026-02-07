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
    let diffs = result.diffs.clone();
    let diff_count = diffs.len();
    let base_name = result.base_profile_used.clone();

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

            // Profile diff table
            <div class="profile-diff-section">
                <h4>
                    "Changes from Base"
                    <span class="diff-count">
                        {format!("{} settings changed from {}", diff_count, base_name)}
                    </span>
                </h4>
                {if diffs.is_empty() {
                    view! {
                        <p class="no-diffs">"No differences from base profile."</p>
                    }.into_any()
                } else {
                    view! {
                        <table class="diff-table">
                            <thead>
                                <tr>
                                    <th>"Setting"</th>
                                    <th>"Base Value"</th>
                                    <th>"New Value"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {diffs.into_iter().map(|d| {
                                    view! {
                                        <tr>
                                            <td class="diff-label">{d.label}</td>
                                            <td class="diff-base">{d.base_value}</td>
                                            <td class="diff-new">{d.new_value}</td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>
                    }.into_any()
                }}
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
