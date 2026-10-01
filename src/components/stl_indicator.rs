use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, StlFile};
use crate::slicer::state::{badge_text, latest_for_source};
use crate::slicer::SlicerShared;

#[component]
pub fn StlIndicator() -> impl IntoView {
    let (stl_files, set_stl_files) = signal::<Vec<StlFile>>(vec![]);
    let (expanded, set_expanded) = signal(false);
    let slicer = expect_context::<SlicerShared>();

    // Poll for STL files every 5 seconds
    Effect::new(move |_| {
        let callback = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
            spawn_local(async move {
                if let Ok(files) = commands::list_received_stls().await {
                    set_stl_files.try_set(files);
                }
            });
        }) as Box<dyn Fn()>);

        // Initial fetch
        let cb_ref = callback.as_ref().unchecked_ref();
        let _ = web_sys::window().unwrap().set_timeout_with_callback(cb_ref);

        let interval_id = web_sys::window()
            .unwrap()
            .set_interval_with_callback_and_timeout_and_arguments_0(cb_ref, 5000)
            .unwrap();
        callback.forget();

        on_cleanup(move || {
            let _ = web_sys::window()
                .unwrap()
                .clear_interval_with_handle(interval_id);
        });
    });

    let open_in_bs = move |stl_path: String| {
        spawn_local(async move {
            let _ = commands::launch_bambu_studio(Some(stl_path), None).await;
        });
    };

    let dismiss = move |path: String| {
        spawn_local(async move {
            let _ = commands::dismiss_stl(&path).await;
            if let Ok(files) = commands::list_received_stls().await {
                set_stl_files.try_set(files);
            }
        });
    };

    view! {
        <Show when=move || !stl_files.get().is_empty()>
            <div class="stl-indicator">
                <div
                    class="stl-badge"
                    on:click=move |_| set_expanded.update(|e| *e = !*e)
                    title="STL files received"
                >
                    <span class="stl-badge-icon">"STL"</span>
                    <span class="stl-badge-count">{move || stl_files.get().len()}</span>
                </div>

                <Show when=move || expanded.get()>
                    <div class="stl-dropdown">
                        <For
                            each=move || stl_files.get()
                            key=|f| f.path.clone()
                            children=move |f: StlFile| {
                                let path_open = f.path.clone();
                                let path_dismiss = f.path.clone();
                                let source = f.path.clone();
                                // Re-evaluated only when this file's newest job changes.
                                let job = Memo::new(move |_| {
                                    slicer.jobs.with(|js| latest_for_source(js, &source).cloned())
                                });
                                let navigate = use_navigate();
                                view! {
                                    <div class="stl-item">
                                        <span class="stl-filename">{f.filename.clone()}</span>
                                        {move || job.get().and_then(|j| {
                                            let text = badge_text(&j)?;
                                            let id = j.id;
                                            let navigate = navigate.clone();
                                            Some(view! {
                                                <button
                                                    class="stl-slice-state"
                                                    on:click=move |_| navigate(&format!("/slice?job={id}"), Default::default())
                                                >
                                                    {text}
                                                </button>
                                            })
                                        })}
                                        <div class="stl-item-actions">
                                            <button
                                                class="btn-small"
                                                on:click=move |_| open_in_bs(path_open.clone())
                                            >
                                                "Open in BS"
                                            </button>
                                            <button
                                                class="btn-small btn-dismiss"
                                                on:click=move |_| dismiss(path_dismiss.clone())
                                            >
                                                "Dismiss"
                                            </button>
                                        </div>
                                    </div>
                                }
                            }
                        />
                    </div>
                </Show>
            </div>
        </Show>
    }
}
