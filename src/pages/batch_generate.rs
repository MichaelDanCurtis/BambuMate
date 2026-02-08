use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, BatchProgress};
use crate::components::specs_editor::PRINTER_OPTIONS;

#[component]
pub fn BatchGeneratePage() -> impl IntoView {
    let (brands, set_brands) = signal::<Vec<String>>(vec![]);
    let (brands_loading, set_brands_loading) = signal(true);
    let (brands_error, set_brands_error) = signal::<Option<String>>(None);

    let (selected_brand, set_selected_brand) = signal(String::new());
    let (selected_printer, set_selected_printer) = signal(String::from("Bambu Lab X1 Carbon 0.4 nozzle"));
    let (install_profiles, set_install_profiles) = signal(true);

    let (is_generating, set_is_generating) = signal(false);
    let (result, set_result) = signal::<Option<BatchProgress>>(None);
    let (gen_error, set_gen_error) = signal::<Option<String>>(None);

    // Load brands on mount
    Effect::new(move |_| {
        spawn_local(async move {
            match commands::list_catalog_brands().await {
                Ok(b) => {
                    set_brands.set(b);
                    set_brands_error.set(None);
                }
                Err(e) => set_brands_error.set(Some(e)),
            }
            set_brands_loading.set(false);
        });
    });

    let do_generate = move |_| {
        let brand = selected_brand.get();
        if brand.is_empty() {
            set_gen_error.set(Some("Please select a brand".to_string()));
            return;
        }

        let printer = selected_printer.get();
        let install = install_profiles.get();

        set_is_generating.set(true);
        set_result.set(None);
        set_gen_error.set(None);

        spawn_local(async move {
            match commands::batch_generate_brand(&brand, Some(printer), install).await {
                Ok(progress) => set_result.set(Some(progress)),
                Err(e) => set_gen_error.set(Some(e)),
            }
            set_is_generating.set(false);
        });
    };

    view! {
        <div class="page batch-generate-page">
            <style>{include_str!("batch_generate.css")}</style>

            <h2>"Batch Profile Generation"</h2>
            <p class="page-description">
                "Generate profiles for all filaments from a brand at once."
            </p>

            // Configuration section
            <div class="batch-config">
                <div class="form-group">
                    <label for="brand-select">"Brand"</label>
                    <Show
                        when=move || !brands_loading.get()
                        fallback=|| view! { <select class="input" disabled=true><option>"Loading brands..."</option></select> }
                    >
                        {move || {
                            if let Some(err) = brands_error.get() {
                                return view! {
                                    <div class="batch-error">{err}</div>
                                }.into_any();
                            }
                            view! {
                                <select
                                    id="brand-select"
                                    class="input"
                                    on:change=move |ev| set_selected_brand.set(event_target_value(&ev))
                                    prop:value=move || selected_brand.get()
                                >
                                    <option value="">"-- Select a brand --"</option>
                                    {move || brands.get().into_iter().map(|b| {
                                        let val = b.clone();
                                        view! { <option value={val}>{b}</option> }
                                    }).collect::<Vec<_>>()}
                                </select>
                            }.into_any()
                        }}
                    </Show>
                </div>

                <div class="form-group">
                    <label for="printer-select">"Target Printer"</label>
                    <select
                        id="printer-select"
                        class="input"
                        on:change=move |ev| set_selected_printer.set(event_target_value(&ev))
                        prop:value=move || selected_printer.get()
                    >
                        {PRINTER_OPTIONS.iter().map(|&p| {
                            view! { <option value={p}>{p}</option> }
                        }).collect::<Vec<_>>()}
                    </select>
                </div>

                <div class="form-group checkbox-group">
                    <label>
                        <input
                            type="checkbox"
                            prop:checked=move || install_profiles.get()
                            on:change=move |ev| {
                                let checked = event_target_checked(&ev);
                                set_install_profiles.set(checked);
                            }
                        />
                        " Install profiles to Bambu Studio"
                    </label>
                </div>

                <button
                    class="btn btn-primary"
                    on:click=do_generate
                    disabled=move || is_generating.get() || selected_brand.get().is_empty()
                >
                    {move || if is_generating.get() { "Generating..." } else { "Generate All" }}
                </button>
            </div>

            // Error
            {move || gen_error.get().map(|e| view! {
                <div class="batch-error">{e}</div>
            })}

            // Generating spinner
            <Show when=move || is_generating.get()>
                <div class="loading-spinner">
                    <div class="spinner"></div>
                    <span>"Generating profiles... This may take a moment."</span>
                </div>
            </Show>

            // Results
            {move || result.get().map(|r| {
                view! {
                    <div class="batch-results">
                        <div class="batch-summary">
                            <span class="batch-stat">{format!("{} total", r.total)}</span>
                            <span class="batch-stat batch-success">{format!("{} succeeded", r.succeeded)}</span>
                            <span class="batch-stat batch-fail">{format!("{} failed", r.failed)}</span>
                        </div>

                        <table class="batch-table">
                            <thead>
                                <tr>
                                    <th>"Status"</th>
                                    <th>"Filament"</th>
                                    <th>"Material"</th>
                                    <th>"Profile Name"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {r.results.iter().map(|entry| {
                                    let status = if entry.success { "OK" } else { "FAIL" };
                                    let class = if entry.success { "row-success" } else { "row-fail" };
                                    let name = entry.profile_name.clone().unwrap_or_default();
                                    let err = entry.error.clone().unwrap_or_default();
                                    view! {
                                        <tr class={class}>
                                            <td class="status-cell">{status}</td>
                                            <td>{entry.filament_name.clone()}</td>
                                            <td>{entry.material.clone()}</td>
                                            <td>
                                                {name}
                                                {(!err.is_empty()).then(|| view! {
                                                    <span class="error-hint" title={err.clone()}>{format!(" ({})", err)}</span>
                                                })}
                                            </td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>
                    </div>
                }
            })}
        </div>
    }
}

fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|e| e.checked())
        .unwrap_or(false)
}
