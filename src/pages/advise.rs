//! Filament → Advise (spec 2026-10-23 §2): what to print a part with.
//!
//! The hub's header ("Filament", with this tab current) is the page header.
//! Every `spawn_local` reads its signals before the first `.await` and uses
//! only `try_*` accessors after it: the page can be gone by then. `request`
//! counts presses of Advise, and `model_gen` counts model changes, so a slow
//! answer to an earlier one is dropped. Changing the inputs while an answer
//! is on its way drops that answer too, so it can't overwrite the newer
//! chips or model.
//!
//! The description is read only when it is new (`last_read`): pressing
//! Advise again after editing the chips scores those chips.
//!
//! The results' status line is a polite live region that is always in the
//! page: the loading line, the AI-failed line, `NOTHING_FITS`, and a
//! visually hidden summary of the answer. Focus stays on Advise, which is
//! `aria-disabled` rather than `disabled` while it runs so focus is never
//! dropped to the body: the summary is announced, the panel's headings are
//! there to jump to, and the usual next step, refining a chip and pressing
//! Advise again, is right beside it. Moving focus into the results would cut
//! the announcement short and send that loop back up the page.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use wasm_bindgen_futures::spawn_local;

use crate::advisor::bridge;
use crate::advisor::format::{
    detail_notes, excluded_reasons, geometry_reordered, hero_detail, hero_tone, model_line,
    rank_pill, reason_value, result_summary, ruled_out_heading, should_interpret, status_tag,
    toggle_need, AI_FAILED, AI_OFF, BEFORE_ADVISING, BUY_FAILED, CATALOG_EMPTY, GEOMETRY_ORDER,
    NOTHING_FITS, ORDER_RULE, READING,
};
use crate::advisor::types::{Advice, MaterialFit, Need, Product, ALL_NEEDS};
use crate::advisor::PendingCatalogPick;
use crate::commands;
use crate::components::patterns::{state_text, Hero, StatRow, StateKind, StateText};
use crate::pages::slice::file_name;
use crate::slicer::state::newest_source;
use crate::slicer::SlicerShared;
use crate::viewport::types::load_view;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Interpreting,
    Scoring,
}

#[component]
pub fn AdvisePage() -> impl IntoView {
    let query = use_query_map();
    let pick = expect_context::<PendingCatalogPick>();
    let jobs = expect_context::<SlicerShared>().jobs;
    let last_model = Memo::new(move |_| jobs.with(|js| newest_source(js).map(str::to_string)));

    // `None` until get_ai_capabilities answers.
    let ai_ready = RwSignal::new(None::<bool>);
    let description = RwSignal::new(String::new());
    // The description as last read into chips; read again only once it changes.
    let last_read = RwSignal::new(None::<String>);
    let needs = RwSignal::new(Vec::<Need>::new());
    let model = RwSignal::new(
        query
            .with_untracked(|q| q.get("model"))
            .filter(|p| !p.trim().is_empty()),
    );
    let model_summary = RwSignal::new(None::<String>);
    let model_error = RwSignal::new(None::<String>);
    let model_gen = RwSignal::new(0u32);
    let phase = RwSignal::new(Phase::Idle);
    let advice = RwSignal::new(None::<Advice>);
    let selected = RwSignal::new(0usize);
    let ai_failed = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let request = RwSignal::new(0u32);
    let pick_button = NodeRef::<leptos::html::Button>::new();

    spawn_local(async move {
        let text = commands::get_ai_capabilities()
            .await
            .map(|c| c.text)
            .unwrap_or(false);
        ai_ready.try_set(Some(text));
    });

    // An input changed: an answer still on its way is for the old inputs.
    // `try_*` throughout, as a model picked in a dialog lands after an await.
    let inputs_changed = move || {
        if phase.try_get_untracked().is_some_and(|p| p != Phase::Idle) {
            request.try_update(|r| *r = r.wrapping_add(1));
            phase.try_set(Phase::Idle);
        }
    };
    let set_model = move |path: Option<String>| {
        inputs_changed();
        model.try_set(path);
    };

    // The model row's footprint-and-risks line, read again when the model changes.
    Effect::new(move |_| {
        let path = model.get();
        model_gen.update(|g| *g = g.wrapping_add(1));
        let generation = model_gen.get_untracked();
        model_summary.set(None);
        model_error.set(None);
        let Some(path) = path else { return };
        spawn_local(async move {
            let result = load_view(&path).await;
            if model_gen.try_get_untracked() != Some(generation) {
                return;
            }
            match result {
                Ok(view) => {
                    model_summary.try_set(Some(model_line(&view)));
                }
                Err(e) => {
                    model_error.try_set(Some(e));
                }
            }
        });
    });

    // A model changed while the dialog was open (last sliced, or Clear) wins.
    let choose_model = move |_| {
        let started = model_gen.get_untracked();
        spawn_local(async move {
            if let Ok(Some(p)) = crate::slicer::bridge::pick_model().await {
                if model_gen.try_get_untracked() == Some(started) {
                    set_model(Some(p));
                }
            }
        });
    };

    // With a new description (and AI), read it first and fill the chips;
    // then score. Otherwise score the chips as they are.
    let run = move |_| {
        // `aria-disabled` does not stop clicks.
        if phase.get_untracked() != Phase::Idle {
            return;
        }
        let text = description.get_untracked().trim().to_string();
        let use_ai = last_read.with_untracked(|l| {
            should_interpret(ai_ready.get_untracked() == Some(true), &text, l.as_deref())
        });
        let chosen = needs.get_untracked();
        let path = model.get_untracked();
        request.update(|r| *r = r.wrapping_add(1));
        let id = request.get_untracked();
        ai_failed.set(false);
        error.set(None);
        phase.set(if use_ai {
            Phase::Interpreting
        } else {
            Phase::Scoring
        });
        spawn_local(async move {
            let current = move || request.try_get_untracked() == Some(id);
            let mut chosen = chosen;
            if use_ai {
                let read = bridge::advise_interpret(&text).await;
                if !current() {
                    return;
                }
                match read {
                    Ok(found) => {
                        chosen = found.clone();
                        needs.try_set(found);
                        last_read.try_set(Some(text));
                    }
                    Err(_) => {
                        ai_failed.try_set(true);
                    }
                }
                phase.try_set(Phase::Scoring);
            }
            let answer = bridge::advise(&chosen, path.as_deref()).await;
            if !current() {
                return;
            }
            match answer {
                Ok(a) => {
                    needs.try_set(a.needs.clone());
                    if let Some(e) = a.model_error.clone() {
                        model_error.try_set(Some(e));
                    }
                    selected.try_set(0);
                    advice.try_set(Some(a));
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            phase.try_set(Phase::Idle);
        });
    };

    let chip = move |need: Need| {
        view! {
            <button
                type="button"
                class="nd-tab adv-chip"
                aria-pressed=move || needs.with(|n| n.contains(&need)).to_string()
                on:click=move |_| {
                    inputs_changed();
                    needs.update(|n| *n = toggle_need(n, need));
                }
            >
                {need.label()}
            </button>
        }
    };

    view! {
        <div class="page advise-page">
            <section class="adv-input" aria-label="The part">
                {move || match ai_ready.get() {
                    Some(true) => view! {
                        <label class="adv-field" for="adv-description">
                            <span class="nd-label">"What is the part for?"</span>
                            <textarea
                                id="adv-description"
                                class="adv-description"
                                rows="2"
                                prop:value=move || description.get()
                                on:input=move |ev| {
                                    inputs_changed();
                                    description.set(event_target_value(&ev));
                                }
                            ></textarea>
                        </label>
                    }
                    .into_any(),
                    Some(false) => view! { <p class="adv-ai-off">{AI_OFF}</p> }.into_any(),
                    None => ().into_any(),
                }}

                <div class="adv-chips" role="group" aria-label="What the part needs">
                    {ALL_NEEDS.iter().map(|n| chip(*n)).collect_view()}
                </div>

                <div class="adv-model">
                    <p class="nd-label">"Model (optional)"</p>
                    <div class="adv-model-actions">
                        {move || last_model.get().map(|path| view! {
                            <button
                                type="button"
                                class="btn btn-secondary adv-last-model"
                                on:click=move |_| set_model(Some(path.clone()))
                            >
                                "Use last sliced model"
                            </button>
                        })}
                        <button
                            type="button"
                            class="btn btn-secondary adv-pick-model"
                            node_ref=pick_button
                            on:click=choose_model
                        >
                            "Choose model…"
                        </button>
                        <Show when=move || model.with(Option::is_some)>
                            <button
                                type="button"
                                class="btn btn-ghost btn-sm adv-model-clear"
                                on:click=move |_| {
                                    set_model(None);
                                    // Clear goes with the model: keep focus in the row.
                                    if let Some(b) = pick_button.get_untracked() {
                                        let _ = b.focus();
                                    }
                                }
                            >
                                "Clear"
                            </button>
                        </Show>
                    </div>
                    {move || model.get().map(|path| view! {
                        <p class="adv-model-name" title=path.clone()>{file_name(&path)}</p>
                    })}
                    {move || model_summary.get().map(|line| view! { <p class="adv-model-line">{line}</p> })}
                    {move || model_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
                </div>

                <div class="adv-actions">
                    <button
                        type="button"
                        class="btn btn-primary adv-run"
                        aria-disabled=move || (phase.get() != Phase::Idle).to_string()
                        on:click=run
                    >
                        "Advise"
                    </button>
                </div>
            </section>

            {move || error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            {move || {
                advice
                    .with(|a| a.as_ref().and_then(|a| a.printer_line.clone()))
                    .map(|l| view! { <p class="adv-printer-line">{l}</p> })
            }}

            // Always in the page, so screen readers hear what changes in it.
            // Plain `nd-state` lines, as a `StateText` would nest a second
            // live region inside this one.
            <div class="adv-status" role="status" aria-live="polite">
                {move || match phase.get() {
                    Phase::Interpreting => Some(state_text(StateKind::Loading, Some(READING))),
                    Phase::Scoring => Some(state_text(StateKind::Loading, None)),
                    Phase::Idle => None,
                }
                .map(|t| view! { <p class="nd-state" data-kind="loading">{t}</p> })}
                {move || ai_failed.get().then(|| view! { <p class="adv-ai-error">{AI_FAILED}</p> })}
                {move || (phase.get() == Phase::Idle).then(|| advice.with(|a| a.as_ref().map(|a| {
                    if a.fits.is_empty() {
                        view! {
                            <p class="nd-state" data-kind="empty">
                                {state_text(StateKind::Empty, Some(NOTHING_FITS))}
                            </p>
                        }
                        .into_any()
                    } else {
                        view! { <p class="nd-visually-hidden">{result_summary(a)}</p> }.into_any()
                    }
                })))}
            </div>

            // Its own closure, so a phase change never rebuilds the results.
            {move || advice.get().map(|a| view! { <Results advice=a selected=selected pick=pick /> })}
            {move || (advice.with(Option::is_none) && phase.get() == Phase::Idle)
                .then(|| view! { <StateText kind=StateKind::Empty message=BEFORE_ADVISING /> })}
        </div>
    }
}

/// The hero, the rank pills, the selected material's reasons and products,
/// and "Ruled out". With nothing that fits, the status line above says
/// `NOTHING_FITS` where the hero would be, and "Ruled out" opens, as it is
/// then the whole answer.
#[component]
fn Results(advice: Advice, selected: RwSignal<usize>, pick: PendingCatalogPick) -> impl IntoView {
    let Advice {
        fits,
        ruled_out,
        catalog_empty,
        ..
    } = advice;
    let nothing_fits = fits.is_empty();
    let reordered = geometry_reordered(&fits);
    let ranks: Vec<(usize, String)> = fits
        .iter()
        .enumerate()
        .map(|(i, f)| (i, rank_pill(i, &f.name)))
        .collect();
    let top = fits.first().cloned();
    let fits = StoredValue::new(fits);
    let current = move || {
        fits.with_value(|f| {
            f.get(selected.get())
                .cloned()
                .or_else(|| f.first().cloned())
        })
    };

    let answer = top.map(|top| {
        view! {
            <Hero label="Best fit" value=top.name.clone() detail=hero_detail(&top) tone=hero_tone(&top) />
            <div class="adv-ranks" role="group" aria-label="Show reasons for">
                {ranks.into_iter().map(|(i, text)| view! {
                    <button
                        type="button"
                        class="nd-tab adv-rank"
                        aria-pressed=move || (selected.get() == i).to_string()
                        on:click=move |_| selected.set(i)
                    >
                        {text}
                    </button>
                }).collect_view()}
            </div>
            {reordered.then(|| view! { <p class="adv-order-note">{GEOMETRY_ORDER}</p> })}
            {move || current().map(|fit| view! { <FitDetail fit=fit pick=pick catalog_empty=catalog_empty /> })}
        }
    });

    view! {
        <div class="adv-results">
            {answer}
            {(!ruled_out.is_empty()).then(|| view! {
                <details class="adv-excluded" open=nothing_fits>
                    <summary>{ruled_out_heading(ruled_out.len())}</summary>
                    <ul class="adv-excluded-list">
                        {ruled_out.iter().map(|e| view! {
                            <li>
                                <span class="adv-excluded-name">{e.name.clone()}</span>
                                " "
                                <span class="adv-excluded-reason">{excluded_reasons(e)}</span>
                            </li>
                        }).collect_view()}
                    </ul>
                </details>
            })}
        </div>
    }
}

/// One material's panel. Every need gets a row, ✗ as plainly as ✓, so a
/// low fit reads as low.
#[component]
fn FitDetail(fit: MaterialFit, pick: PendingCatalogPick, catalog_empty: bool) -> impl IntoView {
    let notes = detail_notes(&fit);
    let reasons = fit
        .reasons
        .iter()
        .map(|r| view! { <StatRow label=r.need.label() value=reason_value(r) /> })
        .collect_view();
    let table = products_table(fit.products.clone(), &fit.name, pick, catalog_empty);
    view! {
        <section class="adv-detail" data-material=fit.material.clone()>
            <h2 class="nd-label">{format!("Why {}", fit.name)}</h2>
            <div class="adv-reasons">
                <StatRow label="Fit" value=format!("{}%", fit.fit_pct) />
                {reasons}
            </div>
            {(!notes.is_empty()).then(|| view! {
                <ul class="adv-notes">{notes.into_iter().map(|n| view! { <li>{n}</li> }).collect_view()}</ul>
            })}
            <h2 class="nd-label">"Products"</h2>
            <p class="adv-order-rule">{ORDER_RULE}</p>
            {table}
        </section>
    }
}

fn products_table(
    products: Vec<Product>,
    material: &str,
    pick: PendingCatalogPick,
    catalog_empty: bool,
) -> AnyView {
    if products.is_empty() {
        let message = if catalog_empty {
            CATALOG_EMPTY.to_string()
        } else {
            format!("No {material} filaments in the catalog")
        };
        return view! { <StateText kind=StateKind::Empty message=message /> }.into_any();
    }
    view! {
        // The roles are explicit: at phone width the rows are laid out as
        // flex boxes, which drops the implicit table roles in WebKit.
        <table class="adv-products" role="table" aria-label=format!("{material} products")>
            <thead role="rowgroup">
                <tr role="row">
                    <th scope="col" role="columnheader">"Product"</th>
                    <th scope="col" role="columnheader">"Brand"</th>
                    <th scope="col" role="columnheader">"Status"</th>
                    <th scope="col" role="columnheader"><span class="nd-visually-hidden">"Actions"</span></th>
                </tr>
            </thead>
            <tbody role="rowgroup">{products.into_iter().map(|p| product_row(p, pick)).collect_view()}</tbody>
        </table>
    }
    .into_any()
}

/// "Generate profile" is a link to Search: the router follows it after this
/// click has handed over the entry. Nothing is written until Search's own
/// Install. A click the router leaves to the browser (a modifier or another
/// button) hands nothing over, so no pick is left waiting. A spool with no
/// catalog entry, or no shop, offers neither.
fn product_row(p: Product, pick: PendingCatalogPick) -> impl IntoView {
    let tag = status_tag(&p);
    let label = format!("{} {}", p.brand, p.name);
    let generate_label = format!("Generate profile for {label}");
    let buy_label = format!("Where to buy {label}");
    let buy_failed = RwSignal::new(false);
    let Product {
        name,
        brand,
        status,
        entry,
        buy_url,
        ..
    } = p;
    view! {
        <tr class="adv-product" data-status=status.key()>
            <th scope="row" role="rowheader" class="adv-product-name">{name}</th>
            <td role="cell" class="adv-product-brand">{brand}</td>
            <td role="cell" class="adv-product-status">{tag.map(|t| view! { <span class="adv-tag">{t}</span> })}</td>
            <td role="cell">
                <div class="adv-product-actions">
                    {entry.map(|e| view! {
                        <a
                            class="btn btn-secondary btn-sm adv-generate"
                            href="/filament"
                            aria-label=generate_label.clone()
                            on:click=move |ev| {
                                let plain = ev.button() == 0
                                    && !(ev.meta_key() || ev.ctrl_key() || ev.shift_key() || ev.alt_key());
                                if plain {
                                    pick.0.set(Some(e.clone()));
                                }
                            }
                        >
                            "Generate profile"
                        </a>
                    })}
                    {buy_url.map(|url| view! {
                        <button
                            type="button"
                            class="btn btn-ghost btn-sm adv-buy"
                            aria-label=buy_label.clone()
                            on:click=move |_| {
                                let url = url.clone();
                                buy_failed.set(false);
                                spawn_local(async move {
                                    if let Err(e) = commands::open_external_url(&url).await {
                                        web_sys::console::warn_1(&format!("open_external_url: {e}").into());
                                        buy_failed.try_set(true);
                                    }
                                });
                            }
                        >
                            "Where to buy"
                        </button>
                    })}
                </div>
                {move || buy_failed.get().then(|| view! { <p class="adv-buy-error" role="alert">{BUY_FAILED}</p> })}
            </td>
        </tr>
    }
}
