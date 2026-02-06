//! Change preview dialog for applying recommendations.
//!
//! Modal dialog that shows which recommendations will be applied,
//! allows user to select/deselect changes, and confirms before apply.

use leptos::prelude::*;

use crate::pages::print_analysis::RecommendationDisplay;

/// Dialog for previewing and confirming parameter changes.
#[component]
pub fn ChangePreview(
    recommendations: Vec<RecommendationDisplay>,
    profile_path: String,
    on_apply: Callback<Vec<String>>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    // Track which recommendations are selected (all by default)
    let initial_selected: Vec<String> = recommendations.iter().map(|r| r.parameter.clone()).collect();
    let (selected, set_selected) = signal(initial_selected);

    // Clone recommendations for use in closures
    let recs_for_render = recommendations.clone();

    let toggle_param = move |param: String| {
        set_selected.update(|s| {
            if s.contains(&param) {
                s.retain(|p| p != &param);
            } else {
                s.push(param);
            }
        });
    };

    let confirm_apply = move |_| {
        on_apply.run(selected.get());
    };

    let cancel_click = move |_| {
        on_cancel.run(());
    };

    view! {
        <div class="change-preview-overlay">
            <style>{include_str!("change_preview.css")}</style>
            <div class="change-preview-dialog">
                <h3>"Apply Recommended Changes"</h3>
                <p class="dialog-subtitle">
                    "Select which changes to apply. A backup will be created automatically."
                </p>
                <p class="dialog-profile">
                    "Profile: " <code>{profile_path.clone()}</code>
                </p>

                <div class="changes-list">
                    {recs_for_render.into_iter().map(|rec| {
                        let param = rec.parameter.clone();
                        let param_for_toggle = param.clone();
                        let param_for_check = param.clone();
                        let label = rec.parameter_label.clone();
                        let change = rec.change_display.clone();
                        let rationale = rec.rationale.clone();

                        view! {
                            <div class="change-item">
                                <label class="change-checkbox">
                                    <input
                                        type="checkbox"
                                        checked=move || selected.get().contains(&param_for_check)
                                        on:change=move |_| toggle_param(param_for_toggle.clone())
                                    />
                                    <div class="change-details">
                                        <span class="param-label">{label}</span>
                                        <span class="change-arrow">{change}</span>
                                    </div>
                                </label>
                                <p class="change-rationale">{rationale}</p>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>

                <div class="dialog-actions">
                    <button class="btn btn-secondary" on:click=cancel_click>
                        "Cancel"
                    </button>
                    <button
                        class="btn btn-primary"
                        on:click=confirm_apply
                        disabled=move || selected.get().is_empty()
                    >
                        {move || format!("Apply {} Changes", selected.get().len())}
                    </button>
                </div>
            </div>
        </div>
    }
}
