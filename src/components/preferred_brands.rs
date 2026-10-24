//! Settings → General: the brands Filament → Advise lists first, after the
//! AMS (spec 2026-10-23 §2). Saved on every toggle; `save_gen` drops a
//! slower answer to an earlier toggle.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::advisor::bridge;
use crate::advisor::format::toggle_brand;
use crate::commands;
use crate::components::patterns::{StateKind, StateText};

#[component]
pub fn PreferredBrands() -> impl IntoView {
    // The catalog's brands; `None` while loading.
    let brands = RwSignal::new(None::<Vec<String>>);
    let chosen = RwSignal::new(Vec::<String>::new());
    let error = RwSignal::new(None::<String>);
    let save_gen = RwSignal::new(0u32);
    // Set once the backend confirms the latest toggle; cleared by the next.
    let just_saved = RwSignal::new(false);

    spawn_local(async move {
        let mine = bridge::get_preferred_brands().await;
        let all = commands::list_catalog_brands().await;
        chosen.try_set(mine.unwrap_or_default());
        match all {
            Ok(list) => {
                brands.try_set(Some(list));
            }
            Err(e) => {
                error.try_set(Some(e));
            }
        }
    });

    let toggle = move |brand: String| {
        let next = toggle_brand(&chosen.get_untracked(), &brand);
        chosen.set(next.clone());
        just_saved.set(false);
        error.set(None);
        save_gen.update(|g| *g = g.wrapping_add(1));
        let id = save_gen.get_untracked();
        spawn_local(async move {
            let saved = bridge::set_preferred_brands(&next).await;
            if save_gen.try_get_untracked() != Some(id) {
                return;
            }
            match saved {
                Ok(list) => {
                    chosen.try_set(list);
                    error.try_set(None);
                    just_saved.try_set(true);
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };

    view! {
        <section class="settings-section preferred-brands">
            <h2 class="nd-label settings-section-title">"Preferred brands"</h2>
            <p class="section-description">
                "Filament → Advise lists these brands' products right after what's loaded in your AMS."
            </p>
            {move || match brands.get() {
                None if error.with(Option::is_none) => view! { <StateText kind=StateKind::Loading /> }.into_any(),
                None => ().into_any(),
                Some(list) if list.is_empty() => view! {
                    <StateText kind=StateKind::Empty message="The filament catalog is empty. Open Filament → Search to fill it." />
                }
                .into_any(),
                Some(list) => view! {
                    <div class="pb-brands" role="group" aria-label="Preferred brands">
                        {list.into_iter().map(|b| {
                            let key = b.clone();
                            let label = b.clone();
                            view! {
                                <button type="button" class="nd-tab pb-brand"
                                    aria-pressed=move || chosen.with(|c| c.iter().any(|x| x.eq_ignore_ascii_case(&key))).to_string()
                                    on:click=move |_| toggle(b.clone())>
                                    {label}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                }
                .into_any(),
            }}
            <Show when=move || just_saved.get()>
                <p class="nd-state pb-saved" role="status">"[SAVED]"</p>
            </Show>
            {move || error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
        </section>
    }
}
