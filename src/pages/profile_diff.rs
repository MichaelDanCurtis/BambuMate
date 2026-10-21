use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, CompareResult, ProfileInfo};
use crate::components::patterns::{Hero, StateKind, StateText};
use crate::components::searchable_select::{SearchableSelect, SelectOption};
use crate::pages::diff_view::{is_changed, visible_categories};

/// Convert user + system profile lists into SelectOption vec.
fn build_select_options(
    user_profiles: &[ProfileInfo],
    system_profiles: &[ProfileInfo],
) -> Vec<SelectOption> {
    let mut options = Vec::new();

    for p in user_profiles {
        options.push(SelectOption {
            value: p.path.clone(),
            label: format!(
                "{} ({})",
                p.name,
                p.filament_type.clone().unwrap_or_default()
            ),
            group: "My Profiles".to_string(),
        });
    }

    for p in system_profiles {
        options.push(SelectOption {
            value: p.path.clone(),
            label: format!(
                "{} ({})",
                p.name,
                p.filament_type.clone().unwrap_or_default()
            ),
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
    // On by default; filters on the client, so toggling never re-compares.
    let (changed_only, set_changed_only) = signal(true);

    let (compare_result, set_compare_result) = signal::<Option<CompareResult>>(None);
    let (is_comparing, set_is_comparing) = signal(false);
    let (compare_error, set_compare_error) = signal::<Option<String>>(None);

    let (collapsed, set_collapsed) = signal::<Vec<String>>(vec![]);

    // Build options signal from both profile lists
    let all_options =
        Signal::derive(move || build_select_options(&user_profiles.get(), &system_profiles.get()));

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

        set_is_comparing.set(true);
        set_compare_result.set(None);
        set_compare_error.set(None);
        set_collapsed.set(vec![]);

        spawn_local(async move {
            match commands::compare_profiles(&a, &b, true).await {
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

            <div class="diff-config">
                <div class="diff-pickers">
                    <div class="form-group">
                        <label class="nd-label">"Profile A"</label>
                        <SearchableSelect
                            id="profile-a"
                            placeholder="Search profiles..."
                            options=all_options
                            value=path_a
                            on_select=move |v| set_path_a.set(v)
                        />
                    </div>
                    <div class="form-group">
                        <label class="nd-label">"Profile B"</label>
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
                            class="diff-changed-only"
                            prop:checked=move || changed_only.get()
                            on:change=move |ev| {
                                use wasm_bindgen::JsCast;
                                let checked = ev.target()
                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                    .map(|e| e.checked())
                                    .unwrap_or(true);
                                set_changed_only.set(checked);
                            }
                        />
                        " Changed only"
                    </label>
                    <Show when=move || is_loading.get()>
                        <StateText kind=StateKind::Loading />
                    </Show>
                    <button
                        class="btn btn-primary"
                        on:click=do_compare
                        disabled=move || is_comparing.get() || path_a.get().is_empty() || path_b.get().is_empty()
                    >
                        {move || if is_comparing.get() { "Comparing…" } else { "Compare" }}
                    </button>
                </div>
            </div>

            {move || compare_error.get().map(|e| view! {
                <StateText kind=StateKind::Error message=e />
            })}

            <Show when=move || compare_result.with(|r| r.is_none()) && compare_error.with(|e| e.is_none()) && !is_comparing.get()>
                <StateText kind=StateKind::Empty message="Pick two profiles to compare" />
            </Show>

            {move || compare_result.get().map(|r| {
                let label = format!("{} vs {}", r.profile_a_name, r.profile_b_name);
                let detail = format!("of {} settings", r.total_fields);
                let name_a = r.profile_a_name.clone();
                let name_b = r.profile_b_name.clone();
                let categories = r.categories.clone();
                view! {
                    <div class="diff-results">
                        <Hero
                            label=label
                            value=r.changed_fields.to_string()
                            unit=" differences"
                            detail=detail
                        />
                        <table class="nd-table diff-table">
                            <thead>
                                <tr>
                                    <th class="diff-col-setting">"Setting"</th>
                                    <th class="diff-col-value" title=name_a.clone()>{name_a.clone()}</th>
                                    <th class="diff-col-value" title=name_b.clone()>{name_b.clone()}</th>
                                </tr>
                            </thead>
                            {move || visible_categories(&categories, changed_only.get()).into_iter().map(|cat| {
                                let name = cat.category.clone();
                                let toggle = name.clone();
                                let open_check = name.clone();
                                let rows_check = name.clone();
                                let changes = cat.diffs.iter().filter(|d| is_changed(d)).count();
                                let count_label = if changes == 1 { "1 change".to_string() } else { format!("{changes} changes") };
                                let rows: Vec<_> = cat.diffs.into_iter().map(|d| {
                                    let class = if is_changed(&d) { "diff-row changed" } else { "diff-row identical" };
                                    view! {
                                        <tr class=class>
                                            <td class="diff-key">{d.label}</td>
                                            <td class="diff-val diff-val-a nd-num">{d.base_value}</td>
                                            <td class="diff-val diff-val-b nd-num">{d.new_value}</td>
                                        </tr>
                                    }
                                }).collect();
                                view! {
                                    <tbody class="diff-category-group">
                                        <tr class="diff-category-row">
                                            <td colspan="3">
                                                <button
                                                    type="button"
                                                    class="diff-category-header"
                                                    aria-expanded=move || (!collapsed.get().contains(&open_check)).to_string()
                                                    on:click=move |_| toggle_category(toggle.clone())
                                                >
                                                    <span class="diff-category-name nd-label">{name}</span>
                                                    <span class="diff-category-count">{count_label}</span>
                                                </button>
                                            </td>
                                        </tr>
                                        <Show when=move || !collapsed.get().contains(&rows_check)>
                                            {rows.clone()}
                                        </Show>
                                    </tbody>
                                }
                            }).collect::<Vec<_>>()}
                        </table>
                    </div>
                }
            })}
        </div>
    }
}
