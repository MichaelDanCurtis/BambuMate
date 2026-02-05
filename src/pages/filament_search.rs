use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, FilamentSpecs, GenerateResult, InstallResult};
use crate::components::filament_card::FilamentCard;
use crate::components::profile_preview::ProfilePreview;

#[component]
pub fn FilamentSearchPage() -> impl IntoView {
    // Search state
    let (search_query, set_search_query) = signal(String::new());
    let (search_result, set_search_result) =
        signal::<Option<Result<FilamentSpecs, String>>>(None);
    let (is_searching, set_is_searching) = signal(false);

    // Generate state
    let (generate_result, set_generate_result) =
        signal::<Option<Result<GenerateResult, String>>>(None);
    let (is_generating, set_is_generating) = signal(false);

    // Install state
    let (install_result, set_install_result) =
        signal::<Option<Result<InstallResult, String>>>(None);
    let (is_installing, set_is_installing) = signal(false);

    // Track current specs for generate flow
    let (current_specs, set_current_specs) = signal::<Option<FilamentSpecs>>(None);
    // Track current generate result for install flow
    let (current_generate, set_current_generate) = signal::<Option<GenerateResult>>(None);

    // Search handler
    let do_search = move || {
        let query = search_query.get();
        if query.len() < 3 {
            return;
        }

        // Clear previous results
        set_search_result.set(None);
        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_specs.set(None);
        set_current_generate.set(None);

        set_is_searching.set(true);
        spawn_local(async move {
            let result = commands::search_filament(&query).await;
            match &result {
                Ok(specs) => {
                    set_current_specs.set(Some(specs.clone()));
                }
                Err(_) => {}
            }
            set_search_result.set(Some(result));
            set_is_searching.set(false);
        });
    };

    // Generate handler
    let do_generate = move || {
        let specs = match current_specs.get() {
            Some(s) => s,
            None => return,
        };

        set_generate_result.set(None);
        set_install_result.set(None);
        set_current_generate.set(None);

        set_is_generating.set(true);
        spawn_local(async move {
            let result = commands::generate_profile(&specs, None).await;
            match &result {
                Ok(gen) => {
                    set_current_generate.set(Some(gen.clone()));
                }
                Err(_) => {}
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

    // Cancel generate preview
    let cancel_generate = move || {
        set_generate_result.set(None);
        set_current_generate.set(None);
    };

    // Handle Enter key in search input
    let on_search_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            do_search();
        }
    };

    view! {
        <div class="page filament-search-page">
            <style>
                {include_str!("filament_search.css")}
            </style>

            <h2>"Filament Search"</h2>
            <p class="page-description">
                "Search for a filament by name to view specifications and generate a Bambu Studio profile."
            </p>

            <div class="search-bar">
                <input
                    type="text"
                    class="input search-input"
                    placeholder="e.g., Polymaker PLA Pro"
                    prop:value=move || search_query.get()
                    on:input=move |ev| {
                        set_search_query.set(event_target_value(&ev));
                    }
                    on:keydown=on_search_keydown
                    disabled=move || is_searching.get()
                />
                <button
                    class="btn btn-primary search-button"
                    on:click=move |_| do_search()
                    disabled=move || search_query.get().len() < 3 || is_searching.get()
                >
                    {move || if is_searching.get() { "Searching..." } else { "Search" }}
                </button>
            </div>

            // Loading indicator
            <Show when=move || is_searching.get()>
                <div class="loading-spinner">
                    <div class="spinner"></div>
                    <span>"Searching for filament specifications..."</span>
                </div>
            </Show>

            // Search error
            {move || {
                if let Some(Err(e)) = search_result.get() {
                    Some(view! {
                        <div class="error-message">
                            <strong>"Search failed: "</strong>{e}
                        </div>
                    })
                } else {
                    None
                }
            }}

            // Search results - FilamentCard
            {move || {
                if let Some(Ok(specs)) = search_result.get() {
                    // Don't show card if we have a generate result (preview takes over)
                    if generate_result.get().is_some() {
                        return None;
                    }
                    Some(view! {
                        <div class="search-results">
                            <FilamentCard
                                specs=specs
                                on_generate=move |_| do_generate()
                                generating=is_generating.get()
                            />
                        </div>
                    })
                } else {
                    None
                }
            }}

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

            // Generate result - ProfilePreview
            {move || {
                if let Some(Ok(gen)) = generate_result.get() {
                    // Don't show preview if install succeeded
                    if let Some(Ok(_)) = install_result.get() {
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
                            <div class="success-header">"Profile Installed Successfully!"</div>
                            <div class="success-details">
                                <p><strong>"Profile: "</strong>{result.profile_name}</p>
                                <p><strong>"Location: "</strong><code>{result.installed_path}</code></p>
                                {if result.bambu_studio_was_running {
                                    Some(view! {
                                        <p class="warning-text">
                                            "Bambu Studio was running during installation. Restart it to see the new profile."
                                        </p>
                                    })
                                } else {
                                    None
                                }}
                            </div>
                            <button
                                class="btn btn-secondary"
                                on:click=move |_| {
                                    // Reset to search another
                                    set_search_query.set(String::new());
                                    set_search_result.set(None);
                                    set_generate_result.set(None);
                                    set_install_result.set(None);
                                    set_current_specs.set(None);
                                    set_current_generate.set(None);
                                }
                            >
                                "Search Another Filament"
                            </button>
                        </div>
                    })
                } else {
                    None
                }
            }}
        </div>
    }
}
