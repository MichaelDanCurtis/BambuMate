use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, CompareResult, ProfileInfo};
use crate::components::searchable_select::{SearchableSelect, SelectOption};

/// Convert user + system profile lists into SelectOption vec.
fn build_select_options(
    user_profiles: &[ProfileInfo],
    system_profiles: &[ProfileInfo],
) -> Vec<SelectOption> {
    let mut options = Vec::new();

    for p in user_profiles {
        options.push(SelectOption {
            value: p.path.clone(),
            label: format!("{} ({})", p.name, p.filament_type.clone().unwrap_or_default()),
            group: "My Profiles".to_string(),
        });
    }

    for p in system_profiles {
        options.push(SelectOption {
            value: p.path.clone(),
            label: format!("{} ({})", p.name, p.filament_type.clone().unwrap_or_default()),
            group: "Bambu Lab Factory Profiles".to_string(),
        });
    }

    options
}

#[component]
pub fn ProfileDiffPage() -> impl IntoView {
    let (user_profiles, set_user_profiles) = signal::<Vec<ProfileInfo>>(vec![]);
    let (system_profiles, set_system_profiles) = signal::<Vec<ProfileInfo>>(vec![]);
    let (is_loading, set_is_loading) = signal(true);

    let (path_a, set_path_a) = signal(String::new());
    let (path_b, set_path_b) = signal(String::new());
    let (show_identical, set_show_identical) = signal(false);

    let (compare_result, set_compare_result) = signal::<Option<CompareResult>>(None);
    let (is_comparing, set_is_comparing) = signal(false);
    let (compare_error, set_compare_error) = signal::<Option<String>>(None);

    let (collapsed, set_collapsed) = signal::<Vec<String>>(vec![]);

    // Build options signal from both profile lists
    let all_options = Signal::derive(move || {
        build_select_options(&user_profiles.get(), &system_profiles.get())
    });

    // Load both user and system profiles on mount
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(list) = commands::list_profiles().await {
                set_user_profiles.set(list);
            }
            if let Ok(list) = commands::list_system_profiles().await {
                set_system_profiles.set(list);
            }
            set_is_loading.set(false);
        });
    });

    let do_compare = move |_| {
        let a = path_a.get();
        let b = path_b.get();
        if a.is_empty() || b.is_empty() {
            set_compare_error.set(Some("Please select two profiles to compare".to_string()));
            return;
        }
        if a == b {
            set_compare_error.set(Some("Please select two different profiles".to_string()));
            return;
        }

        let identical = show_identical.get();
        set_is_comparing.set(true);
        set_compare_result.set(None);
        set_compare_error.set(None);
        set_collapsed.set(vec![]);

        spawn_local(async move {
            match commands::compare_profiles(&a, &b, identical).await {
                Ok(result) => set_compare_result.set(Some(result)),
                Err(e) => set_compare_error.set(Some(e)),
            }
            set_is_comparing.set(false);
        });
    };

    let toggle_category = move |cat: String| {
        let mut c = collapsed.get();
        if c.contains(&cat) {
            c.retain(|x| x != &cat);
        } else {
            c.push(cat);
        }
        set_collapsed.set(c);
    };

    view! {
        <div class="page profile-diff-page">
            <style>{include_str!("profile_diff.css")}</style>

            <h2>"Profile Comparison"</h2>
            <p class="page-description">
                "Compare two filament profiles side-by-side to see differences. "
                "Includes your custom profiles and Bambu Lab factory profiles."
            </p>

            <div class="diff-config">
                <div class="diff-pickers">
                    <div class="form-group">
                        <label>"Profile A"</label>
                        <SearchableSelect
                            id="profile-a"
                            placeholder="Search profiles..."
                            options=all_options
                            value=path_a
                            on_select=move |v| set_path_a.set(v)
                        />
                    </div>
                    <div class="form-group">
                        <label>"Profile B"</label>
                        <SearchableSelect
                            id="profile-b"
                            placeholder="Search profiles..."
                            options=all_options
                            value=path_b
                            on_select=move |v| set_path_b.set(v)
                        />
                    </div>
                </div>

                <div class="diff-actions">
                    <label class="checkbox-label">
                        <input
                            type="checkbox"
                            prop:checked=move || show_identical.get()
                            on:change=move |ev| {
                                use wasm_bindgen::JsCast;
                                let checked = ev.target()
                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                    .map(|e| e.checked())
                                    .unwrap_or(false);
                                set_show_identical.set(checked);
                            }
                        />
                        " Show identical fields"
                    </label>
                    <Show when=move || is_loading.get()>
                        <span class="status-text">"Loading profiles..."</span>
                    </Show>
                    <button
                        class="btn btn-primary"
                        on:click=do_compare
                        disabled=move || is_comparing.get() || path_a.get().is_empty() || path_b.get().is_empty()
                    >
                        {move || if is_comparing.get() { "Comparing..." } else { "Compare" }}
                    </button>
                </div>
            </div>

            {move || compare_error.get().map(|e| view! {
                <div class="diff-error">{e}</div>
            })}

            {move || compare_result.get().map(|r| {
                let summary = format!(
                    "{} of {} fields differ",
                    r.changed_fields, r.total_fields
                );
                let name_a_display = r.profile_a_name.clone();
                let name_a_tip = r.profile_a_name.clone();
                let name_b_display = r.profile_b_name.clone();
                let name_b_tip = r.profile_b_name.clone();

                let category_bodies = r.categories.into_iter().map(|cat| {
                    let cat_name = cat.category.clone();
                    let cat_name_display = cat_name.clone();
                    let cat_toggle = cat_name.clone();
                    let cat_check_1 = cat_name.clone();
                    let cat_check_2 = cat_name.clone();
                    let diff_count = cat.diffs.iter().filter(|d| d.base_value != d.new_value).count();
                    let count_label = if diff_count == 1 {
                        "1 change".to_string()
                    } else {
                        format!("{} changes", diff_count)
                    };

                    let diff_rows: Vec<_> = cat.diffs.into_iter().map(|d| {
                        let is_changed = d.base_value != d.new_value;
                        let row_class = if is_changed { "diff-row changed" } else { "diff-row identical" };
                        view! {
                            <tr class={row_class}>
                                <td class="diff-key">{d.label}</td>
                                <td class="diff-val diff-val-a">{d.base_value}</td>
                                <td class="diff-val diff-val-b">{d.new_value}</td>
                            </tr>
                        }
                    }).collect();

                    view! {
                        <tbody class="diff-category-group">
                            <tr class="diff-category-row">
                                <td colspan="3">
                                    <div
                                        class="diff-category-header"
                                        on:click=move |_| toggle_category(cat_toggle.clone())
                                    >
                                        <span class={move || {
                                            if collapsed.get().contains(&cat_check_1) {
                                                "diff-category-arrow"
                                            } else {
                                                "diff-category-arrow open"
                                            }
                                        }}>
                                            "\u{25B6}"
                                        </span>
                                        <span class="diff-category-name">{cat_name_display}</span>
                                        <span class="diff-category-count">{count_label}</span>
                                    </div>
                                </td>
                            </tr>
                            <Show when=move || !collapsed.get().contains(&cat_check_2)>
                                {diff_rows.clone()}
                            </Show>
                        </tbody>
                    }
                }).collect::<Vec<_>>();

                view! {
                    <div class="diff-results">
                        <div class="diff-summary">
                            <span class="diff-summary-text">{summary}</span>
                        </div>

                        <table class="diff-table">
                            <thead>
                                <tr>
                                    <th class="diff-col-setting">"Setting"</th>
                                    <th class="diff-col-value" title={name_a_tip}>{name_a_display}</th>
                                    <th class="diff-col-value" title={name_b_tip}>{name_b_display}</th>
                                </tr>
                            </thead>
                            {category_bodies}
                        </table>
                    </div>
                }
            })}
        </div>
    }
}
