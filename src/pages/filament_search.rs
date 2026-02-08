use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{
    self, CatalogEntry, CatalogMatch, CatalogStatus, FilamentSpecs, GenerateResult, InstallResult,
};
use crate::components::filament_card::FilamentCard;
use crate::components::profile_preview::ProfilePreview;
use crate::components::specs_editor::SpecsEditor;

#[component]
pub fn FilamentSearchPage() -> impl IntoView {
    // Catalog state
    let (catalog_status, set_catalog_status) = signal::<Option<CatalogStatus>>(None);
    let (is_refreshing_catalog, set_is_refreshing_catalog) = signal(false);

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
    let (generate_result, set_generate_result) =
        signal::<Option<Result<GenerateResult, String>>>(None);
    let (is_generating, set_is_generating) = signal(false);
    let (current_generate, set_current_generate) = signal::<Option<GenerateResult>>(None);

    // Install state
    let (install_result, set_install_result) =
        signal::<Option<Result<InstallResult, String>>>(None);
    let (is_installing, set_is_installing) = signal(false);

    // Check catalog status on mount
    Effect::new(move |_| {
        spawn_local(async move {
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

    // Debounced autocomplete search
    let search_timeout = StoredValue::new(None::<i32>);

    let do_autocomplete = move |query: String| {
        if let Some(id) = search_timeout.get_value() {
            let _ = web_sys::window().unwrap().clear_timeout_with_handle(id);
        }

        if query.len() < 2 {
            set_suggestions.set(vec![]);
            set_show_suggestions.set(false);
            return;
        }

        let callback = wasm_bindgen::closure::Closure::once(move || {
            spawn_local(async move {
                if let Ok(matches) = commands::search_catalog(&query, Some(6)).await {
                    set_suggestions.set(matches);
                    set_show_suggestions.set(true);
                }
            });
        });

        let id = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                150,
            )
            .unwrap();
        callback.forget();
        search_timeout.set_value(Some(id));
    };

    // Handle catalog suggestion selection
    let select_suggestion = move |entry: CatalogEntry| {
        let display_name = format!("{} {}", entry.brand, entry.name);
        set_search_query.set(display_name);
        set_show_suggestions.set(false);

        // Clear previous results
        set_current_specs.set(None);
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);
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
            set_fetch_error.set(Some("Please enter a more specific filament name (at least 5 characters) for AI search.".to_string()));
            return;
        }

        set_show_suggestions.set(false);
        set_current_specs.set(None);
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);
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
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);
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
            set_fetch_error.set(Some("Please enter a valid URL starting with http:// or https://".to_string()));
            return;
        }

        let filament_name = if name.len() >= 3 { name } else { "Unknown Filament".to_string() };

        set_show_url_input.set(false);
        set_current_specs.set(None);
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);
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

    // Generate handler — now takes edited specs and printer from SpecsEditor
    let do_generate_with_specs = move |(edited_specs, printer): (FilamentSpecs, String)| {
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);

        set_is_generating.set(true);
        spawn_local(async move {
            let result = commands::generate_profile(&edited_specs, Some(printer)).await;
            if let Ok(ref gen) = result {
                set_current_generate.set(Some(gen.clone()));
            }
            set_generate_result.set(Some(result));
            set_is_generating.set(false);
        });
    };

    // Install handler
    let do_install = move || {
        let gen = match current_generate.get() {
            Some(g) => g,
            None => return,
        };

        set_is_installing.set(true);
        let profile_json = gen.profile_json.clone();
        let metadata_info = gen.metadata_info.clone();
        let filename = gen.filename.clone();

        spawn_local(async move {
            let result =
                commands::install_profile(&profile_json, &metadata_info, &filename, true).await;
            set_install_result.set(Some(result));
            set_is_installing.set(false);
        });
    };

    let cancel_generate = move || {
        set_generate_result.set(None);
        set_current_generate.set(None);
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
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);
        set_fetch_error.set(None);
        set_show_editor.set(false);
    };

    view! {
        <div class="page filament-search-page">
            <style>{include_str!("filament_search.css")}</style>

            <h2>"Filament Search"</h2>
            <p class="page-description">
                "Type to search from our catalog, or use AI to find any filament."
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
                            let callback = wasm_bindgen::closure::Closure::once(move || {
                                set_show_suggestions.set(false);
                            });
                            let _ = web_sys::window()
                                .unwrap()
                                .set_timeout_with_callback_and_timeout_and_arguments_0(
                                    callback.as_ref().unchecked_ref(),
                                    200,
                                );
                            callback.forget();
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

                        // AI/Web search options (require >= 5 chars when catalog matches exist)
                        {move || {
                            let query_len = search_query.get().len();
                            let has_catalog_matches = !suggestions.get().is_empty();
                            let show_ai_web = query_len >= 5 || !has_catalog_matches;

                            if show_ai_web {
                                view! {
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
                        "Note: Some e-commerce sites (Shopify, etc.) may not work due to JavaScript rendering."
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
                    if show_editor.get() || generate_result.get().is_some() {
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

            // Specs Editor — shown between FilamentCard and ProfilePreview
            {move || {
                if let Some(specs) = current_specs.get() {
                    if show_editor.get() && generate_result.get().is_none() {
                        return Some(view! {
                            <div class="editor-section">
                                <SpecsEditor
                                    specs=specs
                                    on_generate=move |data: (FilamentSpecs, String)| do_generate_with_specs(data)
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
                    <span>"Generating profile..."</span>
                </div>
            </Show>

            // Generate error
            {move || {
                if let Some(Err(e)) = generate_result.get() {
                    Some(view! {
                        <div class="error-message">
                            <strong>"Generation failed: "</strong>{e}
                        </div>
                    })
                } else {
                    None
                }
            }}

            // Profile preview
            {move || {
                if let Some(Ok(gen)) = generate_result.get() {
                    if install_result.get().is_some_and(|r| r.is_ok()) {
                        return None;
                    }
                    Some(view! {
                        <div class="generate-section">
                            <ProfilePreview
                                result=gen
                                on_install=move |_| do_install()
                                on_cancel=move |_| cancel_generate()
                                installing=is_installing.get()
                            />
                        </div>
                    })
                } else {
                    None
                }
            }}

            // Install error
            {move || {
                if let Some(Err(e)) = install_result.get() {
                    Some(view! {
                        <div class="error-message">
                            <strong>"Installation failed: "</strong>{e}
                        </div>
                    })
                } else {
                    None
                }
            }}

            // Install success
            {move || {
                if let Some(Ok(result)) = install_result.get() {
                    Some(view! {
                        <div class="success-message">
                            <div class="success-header">"✓ Profile Installed"</div>
                            <div class="success-details">
                                <p><strong>"Profile: "</strong>{result.profile_name}</p>
                                <p><strong>"Location: "</strong><code>{result.installed_path}</code></p>
                                {result.bambu_studio_was_running.then(|| view! {
                                    <p class="warning-text">
                                        "Restart Bambu Studio to see the new profile."
                                    </p>
                                })}
                            </div>
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
                } else {
                    None
                }
            }}
        </div>
    }
}
