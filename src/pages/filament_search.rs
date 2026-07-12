use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{
    self, BaseProfileMatch, CatalogEntry, CatalogMatch, CatalogStatus, FilamentSpecs,
    GenerateResult, InstallResult,
};
use crate::components::filament_card::FilamentCard;
use crate::components::profile_preview::ProfilePreview;
use crate::components::settings_merge::SettingsMerge;
use crate::components::specs_editor::SpecsEditor;

/// Return the default Bambu Studio base profile name for a given material
/// string, matching the mapping in `generator::base_profile_name` on the
/// backend. Case-insensitive; unknown materials fall back to PLA.
fn default_base_profile_name(material: Option<&str>) -> &'static str {
    let m = material.unwrap_or("").trim().to_ascii_uppercase();
    match m.as_str() {
        "PLA" => "fdm_filament_pla",
        "PETG" | "PET" | "PCTG" => "fdm_filament_pet",
        "ABS" => "fdm_filament_abs",
        "ASA" => "fdm_filament_asa",
        "TPU" => "fdm_filament_tpu",
        "PA" | "NYLON" => "fdm_filament_pa",
        "PC" => "fdm_filament_pc",
        "PVA" => "fdm_filament_pva",
        "HIPS" => "fdm_filament_hips",
        _ => "fdm_filament_pla",
    }
}

#[component]
pub fn FilamentSearchPage() -> impl IntoView {
    // Catalog state
    let (catalog_status, set_catalog_status) = signal::<Option<CatalogStatus>>(None);
    let (is_refreshing_catalog, set_is_refreshing_catalog) = signal(false);
    let (filament_ai_enabled, set_filament_ai_enabled) = signal(true);

    // Autocomplete state
    let (search_query, set_search_query) = signal(String::new());
    let (suggestions, set_suggestions) = signal::<Vec<CatalogMatch>>(vec![]);
    let (show_suggestions, set_show_suggestions) = signal(false);

    // Fetch state (when user selects a suggestion or uses AI fallback)
    let (is_fetching, set_is_fetching) = signal(false);
    let (fetch_error, set_fetch_error) = signal::<Option<String>>(None);

    // URL input mode (for pasting product URLs directly)
    let (show_url_input, set_show_url_input) = signal(false);
    let (url_input, set_url_input) = signal(String::new());

    // Specs state (after fetching)
    let (current_specs, set_current_specs) = signal::<Option<FilamentSpecs>>(None);

    // Editor state (between FilamentCard and ProfilePreview)
    let (show_editor, set_show_editor) = signal(false);

    // Generate state
    let (generate_results, set_generate_results) =
        signal::<Vec<(String, Result<GenerateResult, String>)>>(vec![]);
    let (is_generating, set_is_generating) = signal(false);
    let (pending_installs, set_pending_installs) = signal::<Vec<GenerateResult>>(vec![]);

    // Install state
    let (install_results, set_install_results) =
        signal::<Vec<(String, Result<InstallResult, String>)>>(vec![]);
    let (is_installing, set_is_installing) = signal(false);

    // Base profile reference state
    let (base_profile_matches, set_base_profile_matches) = signal::<Vec<BaseProfileMatch>>(vec![]);
    let (is_searching_base, set_is_searching_base) = signal(false);
    let (selected_base_profile, set_selected_base_profile) =
        signal::<Option<BaseProfileMatch>>(None);
    let (selected_base_profile_path, set_selected_base_profile_path) =
        signal::<Option<String>>(None);
    let (base_profile_specs, set_base_profile_specs) = signal::<Option<FilamentSpecs>>(None);
    let (show_merge_screen, set_show_merge_screen) = signal(false);
    let (base_profile_search, set_base_profile_search) = signal(String::new());

    // Guards the "reset base picker + re-search" Effect below so it only fires
    // when specs come from a new fetch, not when the user completes the merge
    // step (which also updates `current_specs` but must preserve the already-
    // picked base profile so the backend uses it as the generation base).
    let (specs_from_merge, set_specs_from_merge) = signal(false);

    // Check catalog status on mount and load AI preference
    Effect::new(move |_| {
        spawn_local(async move {
            // Load AI mode preference
            if let Ok(Some(val)) = commands::get_preference("filament_search_use_ai").await {
                set_filament_ai_enabled.set(val != "false");
            }

            match commands::get_catalog_status().await {
                Ok(status) => {
                    set_catalog_status.set(Some(status.clone()));
                    if status.needs_refresh || status.entry_count == 0 {
                        set_is_refreshing_catalog.set(true);
                        if let Ok(new_status) = commands::refresh_catalog().await {
                            set_catalog_status.set(Some(new_status));
                        }
                        set_is_refreshing_catalog.set(false);
                    }
                }
                Err(_) => {
                    set_is_refreshing_catalog.set(true);
                    if let Ok(status) = commands::refresh_catalog().await {
                        set_catalog_status.set(Some(status));
                    }
                    set_is_refreshing_catalog.set(false);
                }
            }
        });
    });

    // Auto-search for base profiles when specs are fetched
    Effect::new(move |_| {
        let specs = current_specs.get();
        if let Some(ref s) = specs {
            // Skip the reset+re-search when the specs update came from the
            // merge step: the user has already picked a base and we must not
            // clear `selected_base_profile_path`, otherwise generation would
            // silently fall back to the default (Generic PLA) base.
            if specs_from_merge.get_untracked() {
                set_specs_from_merge.set(false);
                return;
            }
            let material = s.material.clone();
            set_is_searching_base.set(true);
            set_base_profile_matches.set(vec![]);
            set_selected_base_profile.set(None);
            set_selected_base_profile_path.set(None);
            set_base_profile_specs.set(None);
            set_show_merge_screen.set(false);
            set_base_profile_search.set(String::new());
            spawn_local(async move {
                if let Ok(matches) =
                    commands::search_base_profiles("", Some(material.as_str())).await
                {
                    set_base_profile_matches.set(matches);
                }
                set_is_searching_base.set(false);
            });
        }
    });

    // Debounced autocomplete search.
    //
    // Bug fix: this used to call `callback.forget()` for every keystroke,
    // leaking a Closure into WASM memory on each character typed.
    // We now retain the current closure in a `StoredValue` (LocalStorage
    // since `Closure` isn't Send) so it drops when replaced.
    let search_timeout = StoredValue::new(None::<i32>);
    let search_callback: StoredValue<
        Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
        LocalStorage,
    > = StoredValue::new_local(None);
    let blur_callback: StoredValue<
        Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
        LocalStorage,
    > = StoredValue::new_local(None);

    let do_autocomplete = move |query: String| {
        if let Some(id) = search_timeout.get_value() {
            if let Some(win) = web_sys::window() {
                win.clear_timeout_with_handle(id);
            }
        }
        // Drop the previous closure (if any) before installing a new one.
        search_callback.update_value(|slot| {
            *slot = None;
        });

        if query.len() < 2 {
            set_suggestions.set(vec![]);
            set_show_suggestions.set(false);
            return;
        }

        let callback = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            let query = query.clone();
            spawn_local(async move {
                if let Ok(matches) = commands::search_catalog(&query, Some(6)).await {
                    set_suggestions.set(matches);
                    set_show_suggestions.set(true);
                }
            });
        });

        let Some(win) = web_sys::window() else {
            return;
        };
        let id = match win.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            150,
        ) {
            Ok(id) => id,
            Err(_) => return,
        };
        search_callback.update_value(|slot| *slot = Some(callback));
        search_timeout.set_value(Some(id));
    };

    // Handle catalog suggestion selection
    let select_suggestion = move |entry: CatalogEntry| {
        let display_name = format!("{} {}", entry.brand, entry.name);
        set_search_query.set(display_name);
        set_show_suggestions.set(false);

        // Clear previous results
        set_current_specs.set(None);
        set_generate_results.set(vec![]);
        set_install_results.set(vec![]);
        set_pending_installs.set(vec![]);
        set_fetch_error.set(None);
        set_show_editor.set(false);

        // Fetch full specs from catalog entry
        set_is_fetching.set(true);
        spawn_local(async move {
            match commands::fetch_filament_from_catalog(&entry).await {
                Ok(specs) => {
                    set_current_specs.set(Some(specs));
                    set_fetch_error.set(None);
                }
                Err(e) => {
                    set_fetch_error.set(Some(e));
                }
            }
            set_is_fetching.set(false);
        });
    };

    // Handle AI knowledge generation (asks AI directly without web scraping)
    let do_ai_generate = move || {
        let query = search_query.get();
        if query.len() < 5 {
            set_fetch_error.set(Some(
                "Please enter a more specific filament name (at least 5 characters) for AI search."
                    .to_string(),
            ));
            return;
        }

        set_show_suggestions.set(false);
        set_current_specs.set(None);
        set_generate_results.set(vec![]);
        set_install_results.set(vec![]);
        set_pending_installs.set(vec![]);
        set_fetch_error.set(None);
        set_show_editor.set(false);

        set_is_fetching.set(true);
        spawn_local(async move {
            match commands::generate_specs_from_ai(&query).await {
                Ok(specs) => {
                    set_current_specs.set(Some(specs));
                    set_fetch_error.set(None);
                }
                Err(e) => {
                    set_fetch_error.set(Some(e));
                }
            }
            set_is_fetching.set(false);
        });
    };

    // Handle web search fallback (uses original search_filament with web scraping)
    let do_web_search = move || {
        let query = search_query.get();
        if query.len() < 5 {
            set_fetch_error.set(Some("Please enter a more specific filament name (at least 5 characters) for web search.".to_string()));
            return;
        }

        set_show_suggestions.set(false);
        set_current_specs.set(None);
        set_generate_results.set(vec![]);
        set_install_results.set(vec![]);
        set_pending_installs.set(vec![]);
        set_fetch_error.set(None);
        set_show_editor.set(false);

        set_is_fetching.set(true);
        spawn_local(async move {
            match commands::search_filament(&query).await {
                Ok(specs) => {
                    set_current_specs.set(Some(specs));
                    set_fetch_error.set(None);
                }
                Err(e) => {
                    set_fetch_error.set(Some(e));
                }
            }
            set_is_fetching.set(false);
        });
    };

    // Manual refresh catalog
    let refresh_catalog = move || {
        set_is_refreshing_catalog.set(true);
        spawn_local(async move {
            if let Ok(status) = commands::refresh_catalog().await {
                set_catalog_status.set(Some(status));
            }
            set_is_refreshing_catalog.set(false);
        });
    };

    // Extract from pasted URL
    let do_extract_from_url = move || {
        let url = url_input.get();
        let name = search_query.get();

        if url.is_empty() || !url.starts_with("http") {
            set_fetch_error.set(Some(
                "Please enter a valid URL starting with http:// or https://".to_string(),
            ));
            return;
        }

        let filament_name = if name.len() >= 3 {
            name
        } else {
            "Unknown Filament".to_string()
        };

        set_show_url_input.set(false);
        set_current_specs.set(None);
        set_generate_results.set(vec![]);
        set_install_results.set(vec![]);
        set_pending_installs.set(vec![]);
        set_fetch_error.set(None);
        set_show_editor.set(false);

        set_is_fetching.set(true);
        spawn_local(async move {
            match commands::extract_specs_from_url(&url, &filament_name).await {
                Ok(specs) => {
                    set_current_specs.set(Some(specs));
                    set_fetch_error.set(None);
                }
                Err(e) => {
                    set_fetch_error.set(Some(e));
                }
            }
            set_is_fetching.set(false);
        });
    };

    // Show editor when user clicks "Generate Profile" on FilamentCard
    let show_specs_editor = move || {
        set_show_editor.set(true);
    };

    // Generate handler — takes edited specs and a list of printer labels from SpecsEditor.
    // Generates one profile per selected nozzle type sequentially.
    let do_generate_with_specs = move |(edited_specs, printers): (FilamentSpecs, Vec<String>)| {
        set_generate_results.set(vec![]);
        set_install_results.set(vec![]);
        set_pending_installs.set(vec![]);

        set_is_generating.set(true);
        let base_profile_path = selected_base_profile_path.get();
        spawn_local(async move {
            let mut results: Vec<(String, Result<GenerateResult, String>)> = Vec::new();
            let mut installs: Vec<GenerateResult> = Vec::new();
            // Track the filament_id resolved by the first successful generation in
            // this batch. All subsequent nozzle variants reuse the same ID so that
            // profiles for the same physical filament are grouped correctly in
            // Bambu Studio regardless of nozzle size.
            let mut shared_filament_id: Option<String> = None;
            for printer in printers {
                let result = commands::generate_profile(
                    &edited_specs,
                    Some(printer.clone()),
                    base_profile_path.clone(),
                    shared_filament_id.clone(),
                )
                .await;
                if let Ok(ref gen) = result {
                    // Capture the filament_id from the first successful result
                    if shared_filament_id.is_none() {
                        shared_filament_id = Some(gen.filament_id.clone());
                    }
                    installs.push(gen.clone());
                }
                results.push((printer, result));
            }
            set_pending_installs.set(installs);
            set_generate_results.set(results);
            set_is_generating.set(false);
        });
    };

    // Install handler — installs all successfully generated profiles.
    let do_install = move || {
        let installs = pending_installs.get();
        if installs.is_empty() {
            return;
        }
        set_is_installing.set(true);
        spawn_local(async move {
            let mut results: Vec<(String, Result<InstallResult, String>)> = Vec::new();
            for gen in installs {
                let profile_name = gen.profile_name.clone();
                let result = commands::install_profile(
                    &gen.profile_json,
                    &gen.metadata_info,
                    &gen.filename,
                    true,
                )
                .await;
                results.push((profile_name, result));
            }
            set_install_results.set(results);
            set_is_installing.set(false);
        });
    };

    let cancel_generate = move || {
        set_generate_results.set(vec![]);
        set_pending_installs.set(vec![]);
    };

    let cancel_editor = move || {
        set_show_editor.set(false);
    };

    let reset_search = move || {
        set_search_query.set(String::new());
        set_suggestions.set(vec![]);
        set_show_suggestions.set(false);
        set_show_url_input.set(false);
        set_url_input.set(String::new());
        set_current_specs.set(None);
        set_generate_results.set(vec![]);
        set_install_results.set(vec![]);
        set_pending_installs.set(vec![]);
        set_fetch_error.set(None);
        set_show_editor.set(false);
        set_base_profile_matches.set(vec![]);
        set_selected_base_profile.set(None);
        set_selected_base_profile_path.set(None);
        set_base_profile_specs.set(None);
        set_show_merge_screen.set(false);
        set_base_profile_search.set(String::new());
    };

    // Handler for selecting a base profile to use as reference
    let on_select_base_profile = move |profile: BaseProfileMatch| {
        let path = profile.path.clone();
        set_selected_base_profile.set(Some(profile));
        set_selected_base_profile_path.set(Some(path.clone()));
        spawn_local(async move {
            // Extract specs from the selected base profile
            if let Ok(specs) = commands::extract_specs_from_profile(&path).await {
                set_base_profile_specs.set(Some(specs));
                set_show_merge_screen.set(true);
            }
        });
    };

    let on_use_default_base = move || {
        set_selected_base_profile.set(None);
        set_selected_base_profile_path.set(None);
        set_base_profile_specs.set(None);
        set_show_merge_screen.set(false);
    };

    // Handler for completing the merge (user selected which settings to use).
    // Set the merge flag BEFORE updating current_specs so the auto-search
    // Effect skips its reset pass and preserves the selected base profile.
    let on_merge_complete = move |merged_specs: FilamentSpecs| {
        set_specs_from_merge.set(true);
        set_current_specs.set(Some(merged_specs));
        set_show_merge_screen.set(false);
        set_show_editor.set(true);
    };

    let on_skip_merge = move || {
        set_show_merge_screen.set(false);
        set_show_editor.set(true);
    };

    view! {
        <div class="page filament-search-page">
            <style>{include_str!("filament_search.css")}</style>

            <h2>"Filament Search"</h2>
            <p class="page-description">
                {move || if filament_ai_enabled.get() {
                    "Type to search from our catalog, or use AI to find any filament."
                } else {
                    "🌐 Web-only mode — specs pulled from manufacturer sites. Enable AI in Settings for AI-powered search."
                }}
            </p>

            // Catalog status
            <div class="catalog-status">
                {move || {
                    if is_refreshing_catalog.get() {
                        view! {
                            <span class="status-refreshing">
                                <span class="mini-spinner"></span>
                                " Updating catalog..."
                            </span>
                        }.into_any()
                    } else if let Some(status) = catalog_status.get() {
                        view! {
                            <span>{format!("{} filaments", status.entry_count)}</span>
                            <button class="btn-small" on:click=move |_| refresh_catalog()>
                                "Refresh"
                            </button>
                        }.into_any()
                    } else {
                        view! { <span>"Loading..."</span> }.into_any()
                    }
                }}
            </div>

            // Search input with autocomplete
            <div class="search-container">
                <div class="search-bar">
                    <input
                        type="text"
                        class="search-input"
                        placeholder="Search filaments..."
                        prop:value=move || search_query.get()
                        on:input=move |ev| {
                            let value = event_target_value(&ev);
                            set_search_query.set(value.clone());
                            do_autocomplete(value);
                        }
                        on:focus=move |_| {
                            if !suggestions.get().is_empty() {
                                set_show_suggestions.set(true);
                            }
                        }
                        on:blur=move |_| {
                            // Bug fix: previously called `.forget()` on every blur,
                            // leaking a closure per unfocus. We now retain the
                            // closure in a `StoredValue` (LocalStorage) so it
                            // drops on the next blur.
                            let cb = wasm_bindgen::closure::Closure::once(move || {
                                set_show_suggestions.set(false);
                            });
                            if let Some(win) = web_sys::window() {
                                let _ = win
                                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                                        cb.as_ref().unchecked_ref(),
                                        200,
                                    );
                            }
                            blur_callback.update_value(|slot| *slot = Some(cb));
                        }
                        disabled=move || is_refreshing_catalog.get() || is_fetching.get()
                    />
                </div>

                // Dropdown
                <Show when=move || {
                        show_suggestions.get() && search_query.get().len() > 1
                    }>
                    <div class="suggestions-dropdown">
                        // Catalog results
                        <For
                            each=move || suggestions.get()
                            key=|m| format!("{}-{}", m.entry.brand, m.entry.url_slug)
                            children=move |m| {
                                let entry = m.entry.clone();
                                let entry_click = entry.clone();
                                view! {
                                    <div
                                        class="suggestion-item"
                                        on:mousedown=move |_| select_suggestion(entry_click.clone())
                                    >
                                        <span class="suggestion-icon">"🔍"</span>
                                        <span class="suggestion-text">
                                            <span class="suggestion-brand">{entry.brand.clone()}</span>
                                            <span class="suggestion-name">{entry.name.clone()}</span>
                                        </span>
                                        <span class="suggestion-material">{entry.material.clone()}</span>
                                    </div>
                                }
                            }
                        />

                        // AI/Web search options — only shown when AI is enabled
                        // (both options call AI internally and fail without an API key)
                        {move || {
                            let query_len = search_query.get().len();
                            let has_catalog_matches = !suggestions.get().is_empty();
                            let show_ai_web = query_len >= 5 || !has_catalog_matches;
                            let ai_on = filament_ai_enabled.get();

                            if !ai_on {
                                // Web-only mode: no AI options, just a hint
                                view! {
                                    <div class="specificity-hint">
                                        "🌐 Web-only mode — select from catalog above or paste a URL"
                                    </div>
                                }.into_any()
                            } else if show_ai_web {
                                view! {
                                    <>
                                    <div
                                        class="ai-fallback-item"
                                        on:mousedown=move |_| do_ai_generate()
                                    >
                                        <span class="ai-fallback-icon">"🤖"</span>
                                        <span class="ai-fallback-text">
                                            "Ask AI about \""
                                            {move || search_query.get()}
                                            "\""
                                        </span>
                                        <span class="ai-fallback-hint">"Recommended"</span>
                                    </div>
                                    <div
                                        class="ai-fallback-item"
                                        on:mousedown=move |_| do_web_search()
                                    >
                                        <span class="ai-fallback-icon">"🌐"</span>
                                        <span class="ai-fallback-text">
                                            "Search web for specs"
                                        </span>
                                        <span class="ai-fallback-hint">"Scrape pages"</span>
                                    </div>
                                    </>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="specificity-hint">
                                        "Select from above or type more to search AI/web"
                                    </div>
                                }.into_any()
                            }
                        }}

                        // Paste URL option
                        <div
                            class="ai-fallback-item"
                            on:mousedown=move |_| {
                                set_show_suggestions.set(false);
                                set_show_url_input.set(true);
                            }
                        >
                            <span class="ai-fallback-icon">"🔗"</span>
                            <span class="ai-fallback-text">"Paste a product URL"</span>
                            <span class="ai-fallback-hint">"Direct extraction"</span>
                        </div>
                    </div>
                </Show>
            </div>

            // URL input section
            <Show when=move || show_url_input.get()>
                <div class="url-input-section">
                    <p class="url-input-label">"Paste a product page URL:"</p>
                    <div class="url-input-row">
                        <input
                            type="text"
                            class="search-input"
                            placeholder="https://..."
                            prop:value=move || url_input.get()
                            on:input=move |ev| set_url_input.set(event_target_value(&ev))
                        />
                        <button
                            class="btn btn-primary"
                            on:click=move |_| do_extract_from_url()
                            disabled=move || url_input.get().is_empty()
                        >
                            "Extract"
                        </button>
                        <button
                            class="btn btn-secondary"
                            on:click=move |_| set_show_url_input.set(false)
                        >
                            "Cancel"
                        </button>
                    </div>
                    <p class="url-input-hint">
                        {move || if filament_ai_enabled.get() {
                            "AI-assisted extraction — works on most product pages."
                        } else {
                            "Web-only extraction — reads JSON-LD and spec tables. No API key needed. \
                             Some JS-heavy sites may not work."
                        }}
                    </p>
                </div>
            </Show>

            // Fetching indicator
            <Show when=move || is_fetching.get()>
                <div class="loading-spinner">
                    <div class="spinner"></div>
                    <span>"Fetching specifications..."</span>
                </div>
            </Show>

            // Fetch error (hidden when dropdown is visible)
            {move || {
                if show_suggestions.get() { return None; }
                fetch_error.get().map(|e| view! {
                    <div class="error-message">
                        <strong>"Error: "</strong>{e}
                    </div>
                })
            }}

            // Specs display (FilamentCard) — shown when specs exist and editor is not shown
            {move || {
                if let Some(specs) = current_specs.get() {
                    if show_editor.get() || !generate_results.get().is_empty() || show_merge_screen.get() {
                        return None;
                    }
                    Some(view! {
                        <div class="search-results">
                            <FilamentCard
                                specs=specs
                                on_generate=move |_| show_specs_editor()
                                generating=is_generating.get()
                            />
                        </div>
                    })
                } else {
                    None
                }
            }}

            // Base profile reference matches — shown after specs are fetched
            {move || {
                if current_specs.get().is_none() || show_editor.get() || !generate_results.get().is_empty() || show_merge_screen.get() {
                    return None;
                }
                let matches = base_profile_matches.get();
                let query = base_profile_search.get().to_lowercase();
                let filtered: Vec<_> = matches.iter().filter(|m| {
                    query.is_empty()
                        || m.name.to_lowercase().contains(&query)
                        || m.filament_type.as_deref().unwrap_or("").to_lowercase().contains(&query)
                }).cloned().collect();
                let no_results = filtered.is_empty() && !is_searching_base.get() && !query.is_empty();
                Some(view! {
                    <div class="base-profiles-section">
                        <h4>"Base Profile (Installed in Bambu Studio)"</h4>
                        <p class="section-description">
                            "Choose a base profile to build from, or keep the material default base."
                        </p>
                        <Show when=move || is_searching_base.get()>
                            <span class="mini-spinner"></span>
                            " Searching installed profiles..."
                        </Show>
                        <Show when=move || !is_searching_base.get() && !base_profile_matches.get().is_empty()>
                            <input
                                type="text"
                                class="base-profile-search-input"
                                placeholder="Search profiles..."
                                prop:value=move || base_profile_search.get()
                                on:input=move |e| {
                                    set_base_profile_search.set(event_target_value(&e));
                                }
                            />
                        </Show>
                        <div class="base-profiles-list">
                            <Show when=move || base_profile_search.get().is_empty()>
                                <div
                                    class=move || {
                                        if selected_base_profile_path.get().is_none() {
                                            "base-profile-item selected".to_string()
                                        } else {
                                            "base-profile-item".to_string()
                                        }
                                    }
                                    on:click=move |_| on_use_default_base()
                                >
                                    <span class="base-profile-name">
                                        {move || default_base_profile_name(
                                            current_specs.get().map(|s| s.material).as_deref()
                                        )}
                                    </span>
                                    <span class="base-profile-type">"Material family base"</span>
                                    <span class="base-profile-action">
                                        {move || {
                                            if selected_base_profile_path.get().is_none() {
                                                "Selected"
                                            } else {
                                                "Use default base"
                                            }
                                        }}
                                    </span>
                                </div>
                            </Show>
                            {filtered.iter().map(|m| {
                                let profile_click = m.clone();
                                let name = m.name.clone();
                                let path = m.path.clone();
                                let path_for_class = path.clone();
                                let path_for_label = path.clone();
                                let ftype = m.filament_type.clone().unwrap_or_default();
                                view! {
                                    <div
                                        class=move || {
                                            let selected = selected_base_profile_path
                                                .get()
                                                .is_some_and(|p| p == path_for_class);
                                            if selected {
                                                "base-profile-item selected".to_string()
                                            } else {
                                                "base-profile-item".to_string()
                                            }
                                        }
                                        on:click=move |_| on_select_base_profile(profile_click.clone())
                                    >
                                        <span class="base-profile-name">{name}</span>
                                        <span class="base-profile-type">{ftype}</span>
                                        <span class="base-profile-action">
                                            {move || {
                                                let selected = selected_base_profile_path
                                                    .get()
                                                    .is_some_and(|p| p == path_for_label);
                                                if selected { "Selected" } else { "Use as base" }
                                            }}
                                        </span>
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                            <Show when=move || no_results>
                                <p class="base-profile-no-results">"No profiles match your search."</p>
                            </Show>
                        </div>
                        <Show when=move || matches.is_empty() && !is_searching_base.get()>
                            <p class="section-description">
                                "No close installed matches were found for this filament. You can continue with the default base."
                            </p>
                        </Show>
                    </div>
                })
            }}

            // Settings merge screen — shown when user selects a base profile
            {move || {
                if !show_merge_screen.get() {
                    return None;
                }
                let ai_specs = current_specs.get()?;
                let base_specs = base_profile_specs.get()?;
                let base_name = selected_base_profile.get().map(|p| p.name).unwrap_or_default();

                Some(view! {
                    <div class="merge-screen">
                        <h3>"Compare & Merge Settings"</h3>
                        <p class="section-description">
                            "Choose which settings to use from each source. AI-recommended values are on the left, "
                            "values from \""{base_name.clone()}"\" are on the right."
                        </p>
                        <SettingsMerge
                            ai_specs=ai_specs
                            base_specs=base_specs
                            base_name=base_name
                            on_apply=move |merged| on_merge_complete(merged)
                            on_skip=move |_| on_skip_merge()
                        />
                    </div>
                })
            }}

            // Specs Editor — shown between FilamentCard and ProfilePreview
            {move || {
                if let Some(specs) = current_specs.get() {
                    if show_editor.get() && generate_results.get().is_empty() {
                        return Some(view! {
                            <div class="editor-section">
                                <SpecsEditor
                                    specs=specs
                                    on_generate=move |data: (FilamentSpecs, Vec<String>)| do_generate_with_specs(data)
                                    on_cancel=move |_| cancel_editor()
                                />
                            </div>
                        });
                    }
                }
                None
            }}

            // Generating indicator
            <Show when=move || is_generating.get()>
                <div class="loading-spinner">
                    <div class="spinner"></div>
                    <span>"Generating profile(s)..."</span>
                </div>
            </Show>

            // Generate results: errors, single preview, or multi-profile table
            {move || {
                let results = generate_results.get();
                if results.is_empty() {
                    return None;
                }

                // Hide once all installs are done
                if !install_results.get().is_empty() {
                    return None;
                }

                let gen_errors: Vec<(String, String)> = results
                    .iter()
                    .filter_map(|(label, r)| r.as_ref().err().map(|e| (label.clone(), e.clone())))
                    .collect();
                let successes: Vec<GenerateResult> = results
                    .into_iter()
                    .filter_map(|(_, r)| r.ok())
                    .collect();
                let success_count = successes.len();

                Some(view! {
                    <div class="generate-section">
                        // Per-nozzle generation errors
                        {gen_errors.into_iter().map(|(label, e)| view! {
                            <div class="error-message">
                                <strong>{format!("Generation failed ({}): ", label)}</strong>{e}
                            </div>
                        }).collect::<Vec<_>>()}

                        // Single profile → full ProfilePreview with diff table
                        {if success_count == 1 {
                            let gen = successes.into_iter().next().unwrap();
                            view! {
                                <ProfilePreview
                                    result=gen
                                    on_install=move |_| do_install()
                                    on_cancel=move |_| cancel_generate()
                                    installing=is_installing.get()
                                />
                            }.into_any()
                        } else if success_count > 1 {
                            // Multiple profiles → compact summary table + Install All
                            view! {
                                <div class="multi-profile-results">
                                    <h3 class="multi-profile-title">
                                        {format!("{} Profiles Ready to Install", success_count)}
                                    </h3>
                                    <table class="multi-profile-table">
                                        <thead>
                                            <tr>
                                                <th>"Profile Name"</th>
                                                <th>"Base Profile"</th>
                                                <th>"Fields"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {successes.iter().map(|gen| view! {
                                                <tr>
                                                    <td class="profile-name">{gen.profile_name.clone()}</td>
                                                    <td>{gen.base_profile_used.clone()}</td>
                                                    <td>{gen.field_count}</td>
                                                </tr>
                                            }).collect::<Vec<_>>()}
                                        </tbody>
                                    </table>
                                    <div class="multi-profile-actions">
                                        <button
                                            class="btn btn-secondary"
                                            on:click=move |_| cancel_generate()
                                            disabled=move || is_installing.get()
                                        >
                                            "Cancel"
                                        </button>
                                        <button
                                            class="btn btn-primary"
                                            on:click=move |_| do_install()
                                            disabled=move || is_installing.get()
                                        >
                                            {move || if is_installing.get() {
                                                format!("Installing {} profiles...", success_count)
                                            } else {
                                                format!("Install All {} Profiles", success_count)
                                            }}
                                        </button>
                                    </div>
                                </div>
                            }.into_any()
                        } else {
                            view! { <span></span> }.into_any()
                        }}
                    </div>
                })
            }}

            // Install results: success summary + errors
            {move || {
                let results = install_results.get();
                if results.is_empty() {
                    return None;
                }

                let ok_installs: Vec<InstallResult> = results
                    .iter()
                    .filter_map(|(_, r)| r.as_ref().ok().cloned())
                    .collect();
                let inst_errors: Vec<(String, String)> = results
                    .into_iter()
                    .filter_map(|(name, r)| r.err().map(|e| (name, e)))
                    .collect();
                let ok_count = ok_installs.len();
                let any_bs_running = ok_installs.iter().any(|r| r.bambu_studio_was_running);

                Some(view! {
                    <div class="success-message">
                        {if ok_count > 0 {
                            view! {
                                <div class="success-header">
                                    {format!("✓ {} Profile{} Installed",
                                        ok_count,
                                        if ok_count == 1 { "" } else { "s" })}
                                </div>
                                <div class="success-details">
                                    {ok_installs.iter().map(|r| view! {
                                        <p>
                                            <strong>"Profile: "</strong>{r.profile_name.clone()}
                                        </p>
                                        <p>
                                            <strong>"Location: "</strong>
                                            <code>{r.installed_path.clone()}</code>
                                        </p>
                                    }).collect::<Vec<_>>()}
                                    {any_bs_running.then(|| view! {
                                        <p class="warning-text">
                                            "Restart Bambu Studio to see the new profile(s)."
                                        </p>
                                    })}
                                </div>
                            }.into_any()
                        } else {
                            view! { <span></span> }.into_any()
                        }}

                        {inst_errors.into_iter().map(|(name, e)| view! {
                            <div class="error-message">
                                <strong>{format!("Installation failed ({}): ", name)}</strong>{e}
                            </div>
                        }).collect::<Vec<_>>()}

                        <div class="success-actions">
                            <button
                                class="btn btn-primary"
                                on:click=move |_| {
                                    spawn_local(async move {
                                        let _ = commands::launch_bambu_studio(None, None).await;
                                    });
                                }
                            >
                                "Open Bambu Studio"
                            </button>
                            <button class="btn btn-secondary" on:click=move |_| reset_search()>
                                "Search Another"
                            </button>
                        </div>
                    </div>
                })
            }}
        </div>
    }
}
