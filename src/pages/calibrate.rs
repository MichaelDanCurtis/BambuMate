//! Filament → Calibrate (spec 2026-10-25 §2): guided filament calibration.
//!
//! The hub's header is the page header, and the open step's value is the
//! page's one hero. The page never computes a value: the backend applies
//! Bambu Studio's formulas, and the page shows what it answers.
//!
//! Every `spawn_local` reads its signals before its first `.await` and uses
//! only `try_*` accessors after it. A request carries the generation it
//! was asked under, so a slow answer for an earlier filament, setup,
//! reading or photo is dropped. An answer that carries a run is applied
//! only while the page still shows that run, and only when no answer to a
//! newer request has been shown (`adopt`). While one request that changes
//! the run is out, the others (the variant pick, save, undo, record, skip
//! and link) wait: the page sends them one at a time.

use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::calibration::bridge;
use crate::calibration::types::{
    Diagram, LinkCandidates, PreviewView, Reading, ResultView, RunView, SaveAnswer, Setup, StepKey,
    StepStatus, Suggestion, TestKind,
};
use crate::calibration::view::{
    calibratable, candidate_line, context_line, current_line, date_text, diff_line, extra_line,
    first_open, height_reading, hero_label, history_change, history_value, linked_elsewhere,
    patch_reading, preset_href, reading_for_pick, restart_before_fine, result_href, ruler_ticks,
    run_for_result, same_run, save_notes, setup_fields, slot_has_preset, slot_href,
    slot_preset_name, status_key, status_text, step_name, step_of_result, step_result,
    studio_steps, suggestion_detail, suggestion_line, target_line, test_for, typed_label,
    typed_reading, undo_note, undo_target, variant_needed, will_save_line, with_field, AnswerOrder,
    DEVELOPER_MODE, DISCARD_CHANGES, FINE_AFTER_COARSE, NO_PRESET, PA_NOTE, PICK_VARIANT_PROMPT,
    RESTART_BEFORE_FINE, RESULT_GONE,
};
use crate::commands::{self, ProfileInfo};
use crate::components::patterns::{Hero, StateKind, StateText};
use crate::journal::format::entry_href;
use crate::pages::journal_detail::local_offset_minutes;
use crate::pages::print_analysis::read_file_as_base64;
use crate::pages::printer::{matching, more_than_shown, short_preset};
use crate::printer::types::untrusted_text;
use crate::printer::PrinterShared;

/// Most characters shown of a preset or print name.
const NAME_MAX: usize = 96;
const CHANGED_SINCE_LINE: &str = "The preset changed since. Check the new value, then save again.";

fn offset_now() -> i32 {
    local_offset_minutes((js_sys::Date::now() / 1000.0) as i64)
}

/// The ticket for a request whose answer carries a run. Call before the
/// `.await`.
fn ticket(order: StoredValue<AnswerOrder>) -> u64 {
    order.try_update_value(|o| o.issue()).unwrap_or_default()
}

/// Shows `next` only while the page still shows run `asked`, and only when
/// no newer request's answer has been shown: an answer for a run the user
/// has left is dropped, so the next save can't go to its preset, and an
/// older answer that lands late can't put back an older variant. Call
/// after the `.await`; says whether it was applied.
fn adopt(
    run: RwSignal<Option<RunView>>,
    order: StoredValue<AnswerOrder>,
    asked: i64,
    ticket: u64,
    next: RunView,
) -> bool {
    let current = run
        .try_with_untracked(|r| r.as_ref().map(|r| r.id))
        .flatten();
    if !same_run(current, asked, next.id) {
        return false;
    }
    if order.try_update_value(|o| o.accept(ticket)) != Some(true) {
        return false;
    }
    run.try_set(Some(next));
    true
}

/// One more request that changes the run is out.
fn begin(pending: RwSignal<u32>) {
    pending.update(|n| *n += 1);
}

/// Its answer is back. Call after the `.await`.
fn end(pending: RwSignal<u32>) {
    pending.try_update(|n| *n = n.saturating_sub(1));
}

/// After the card re-renders, focuses the first of `selectors` inside it,
/// so a button that turned disabled or went away doesn't drop focus.
fn focus_in(card: NodeRef<html::Article>, selectors: &'static [&'static str]) {
    set_timeout(
        move || {
            let Some(el) = card.try_get_untracked().flatten() else {
                return;
            };
            for sel in selectors {
                if let Some(e) = el
                    .query_selector(sel)
                    .ok()
                    .flatten()
                    .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = e.focus();
                    return;
                }
            }
        },
        Duration::ZERO,
    );
}

#[component]
pub fn CalibratePage() -> impl IntoView {
    let query = use_query_map();
    let run = RwSignal::new(None::<RunView>);
    let error = RwSignal::new(None::<String>);
    let no_preset = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let open = RwSignal::new(StepKey::Temperature);
    let fine = RwSignal::new(false);
    // A coarse save answered `studio_running`: the fine pass says to restart.
    let studio_was_open = RwSignal::new(false);
    let last_run = StoredValue::new(None::<i64>);
    let ask = StoredValue::new(0u64);
    let order = StoredValue::new(AnswerOrder::default());

    // `?slot=A2`, `?preset=<path>` or `?result=<id>` picks the filament.
    Effect::new(move |_| {
        let (slot, preset, result) =
            query.with(|q| (q.get("slot"), q.get("preset"), q.get("result")));
        let Some(generation) = ask.try_update_value(|g| {
            *g += 1;
            *g
        }) else {
            return;
        };
        // The old run goes at once: nothing on it can be saved, undone or
        // linked while the new filament loads.
        run.set(None);
        error.set(None);
        no_preset.set(None);
        if slot.is_none() && preset.is_none() && result.is_none() {
            loading.set(false);
            return;
        }
        loading.set(true);
        let start_ticket = ticket(order);
        spawn_local(async move {
            let result_id = result.as_deref().and_then(|r| r.parse::<i64>().ok());
            let answer = match (result_id, result.is_some()) {
                (Some(id), _) => match bridge::runs().await {
                    Ok(runs) => match run_for_result(&runs, id) {
                        Some(run_id) => bridge::start(None, None, Some(run_id)).await,
                        None => Err(RESULT_GONE.to_string()),
                    },
                    Err(e) => Err(e),
                },
                (None, true) => Err(RESULT_GONE.to_string()),
                (None, false) => bridge::start(slot.as_deref(), preset.as_deref(), None).await,
            };
            if ask.try_get_value() != Some(generation) {
                return;
            }
            loading.try_set(false);
            match answer {
                Ok(r) => {
                    let opened = result_id.and_then(|id| step_of_result(&r, id));
                    let fine_pass = result_id
                        .and_then(|id| r.results.iter().find(|x| x.id == id))
                        .is_some_and(|x| x.test == TestKind::FlowFine);
                    open.try_set(opened.unwrap_or_else(|| first_open(&r)));
                    fine.try_set(fine_pass);
                    if last_run.try_get_value().flatten() != Some(r.id) {
                        studio_was_open.try_set(false);
                    }
                    last_run.try_update_value(|l| *l = Some(r.id));
                    // Older answers for this run, still out, now land late.
                    order.try_update_value(|o| o.accept(start_ticket));
                    run.try_set(Some(r));
                }
                Err(e) if e == NO_PRESET => {
                    no_preset.try_set(Some(slot.unwrap_or_default()));
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    });

    // Only a new run rebuilds the panel: a save updates it in place.
    let run_id = Memo::new(move |_| run.with(|r| r.as_ref().map(|r| r.id)));
    let idle = move || {
        run_id.get().is_none()
            && !loading.get()
            && error.with(Option::is_none)
            && no_preset.with(Option::is_none)
    };

    view! {
        <div class="page calibrate-page">
            <Picker />
            {move || loading.get().then(|| view! { <StateText kind=StateKind::Loading /> })}
            {move || error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            {move || no_preset.get().map(|label| view! {
                <p class="cal-no-preset">
                    {format!("{} has no profile yet. ", untrusted_text(&label, 16))}
                    <a class="cal-text-link" href="/filament">"Create one in Search"</a>
                </p>
            })}
            <Show when=idle>
                <StateText kind=StateKind::Empty message="Pick a slot or a preset to calibrate." />
            </Show>
            {move || run_id.get().map(|_| view! {
                <RunPanel run=run order=order open=open fine=fine studio_was_open=studio_was_open />
            })}
        </div>
    }
}

/// The AMS slots and a preset search, with the printer as context. Every
/// choice is a link, so the URL always names the filament.
#[component]
fn Picker() -> impl IntoView {
    let printer = expect_context::<PrinterShared>();
    let presets = RwSignal::new(Vec::<ProfileInfo>::new());
    let loaded = StoredValue::new(false);
    let load_error = RwSignal::new(None::<String>);
    let search = RwSignal::new(String::new());
    // The system list is long: it is read once, when the search is used,
    // and again only after a failed read.
    let load = move || {
        if loaded.get_value() {
            return;
        }
        loaded.set_value(true);
        load_error.set(None);
        spawn_local(async move {
            let answer = async {
                let mut all = commands::list_profiles().await?;
                all.extend(commands::list_system_profiles().await?);
                Ok::<_, String>(all)
            }
            .await;
            match answer {
                Ok(all) => {
                    presets.try_set(calibratable(all));
                }
                Err(e) => {
                    loaded.try_update_value(|l| *l = false);
                    load_error.try_set(Some(e));
                }
            }
        });
    };
    let slots = Memo::new(move |_| {
        printer.view.with(|v| {
            v.slots
                .iter()
                .filter(|s| !s.tray.empty)
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let context = Memo::new(move |_| printer.view.with(context_line));
    view! {
        <section class="cal-picker" aria-label="Filament to calibrate">
            <p class="nd-label">"Filament"</p>
            <p class="cal-context">{move || context.get()}</p>
            <div class="cal-slots">
                <For
                    each=move || slots.get()
                    key=|s| (s.label.clone(), slot_preset_name(s))
                    children=|s| view! {
                        <a class="cal-slot" href=slot_href(&s.label) data-slot=s.label.clone()
                            data-ready=slot_has_preset(&s).to_string()>
                            <span class="cal-slot-label nd-mono">{s.label.clone()}</span>
                            <span class="cal-slot-preset">{slot_preset_name(&s)}</span>
                        </a>
                    }
                />
            </div>
            <label class="cal-search">
                <span class="nd-label">"Or a preset"</span>
                <input type="search" class="cal-search-input" placeholder="Search presets"
                    prop:value=move || search.get() on:focus=move |_| load()
                    on:input=move |ev| search.set(event_target_value(&ev)) />
            </label>
            {move || load_error.get().map(|e| view! {
                <div class="cal-row cal-search-error">
                    <StateText kind=StateKind::Error message=e />
                    <button type="button" class="btn btn-secondary cal-search-retry"
                        on:click=move |_| load()>"Retry"</button>
                </div>
            })}
            <Show when=move || !search.with(|q| q.trim().is_empty())>
                <ul class="cal-presets">
                    {move || matching(&presets.get(), &search.get()).into_iter().map(|p| view! {
                        <li><a class="cal-preset" href=preset_href(&p.path)>{untrusted_text(&p.name, NAME_MAX)}</a></li>
                    }).collect_view()}
                </ul>
                <Show when=move || more_than_shown(&presets.get(), &search.get())>
                    <p class="cal-quiet">"Type more to narrow the list."</p>
                </Show>
            </Show>
        </section>
    }
}

/// One run: what it saves to, the variant, the strip, the open step's card
/// and the history.
#[component]
fn RunPanel(
    run: RwSignal<Option<RunView>>,
    order: StoredValue<AnswerOrder>,
    open: RwSignal<StepKey>,
    fine: RwSignal<bool>,
    studio_was_open: RwSignal<bool>,
) -> impl IntoView {
    let target = Memo::new(move |_| run.with(|r| r.as_ref().map(target_line).unwrap_or_default()));
    let preset_error =
        Memo::new(move |_| run.with(|r| r.as_ref().and_then(|r| r.preset_error.clone())));
    // Requests that change the run, still out: the variant pick and every
    // card's save, undo, record, skip and link wait for them.
    let pending = RwSignal::new(0u32);
    // A new step, pass or variant gets a fresh card. A save keeps it.
    let card = Memo::new(move |_| {
        (
            open.get(),
            fine.get(),
            run.with(|r| r.as_ref().and_then(|r| r.variant.clone())),
        )
    });
    view! {
        <section class="cal-run" aria-label="Calibration">
            <p class="cal-target">{move || target.get()}</p>
            {move || preset_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            <VariantPicker run=run order=order pending=pending />
            <CalStrip run=run open=open fine=fine />
            {move || {
                let (step, fine_pass, _) = card.get();
                view! {
                    <StepCard run=run order=order pending=pending step=step fine=fine
                        fine_pass=fine_pass studio_was_open=studio_was_open />
                }
            }}
            <History run=run />
        </section>
    }
}

/// Shown when the preset has several nozzle variants: the user's pick,
/// listing the preset's own names. It waits while a card's save, undo,
/// record, skip or link is out: a save is for the variant it previewed.
#[component]
fn VariantPicker(
    run: RwSignal<Option<RunView>>,
    order: StoredValue<AnswerOrder>,
    pending: RwSignal<u32>,
) -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let waiting = move || pending.get() > 0;
    let pick = move |variant: String| {
        if pending.get_untracked() > 0 {
            return;
        }
        let Some(id) = run.with_untracked(|r| r.as_ref().map(|r| r.id)) else {
            return;
        };
        let ticket = ticket(order);
        begin(pending);
        error.set(None);
        spawn_local(async move {
            let answer = bridge::set_variant(id, &variant).await;
            end(pending);
            match answer {
                Ok(r) => {
                    adopt(run, order, id, ticket, r);
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    let shown = Memo::new(move |_| {
        run.with(|r| {
            r.as_ref()
                .map(|r| (r.needs_variant, r.variants.clone(), r.variant.clone()))
                .unwrap_or_default()
        })
    });
    move || {
        let (needed, variants, chosen) = shown.get();
        (variants.len() > 1).then(|| view! {
            <div class="cal-variant" role="group" aria-label="Nozzle variant">
                <span class="nd-label">{if needed { "Pick the nozzle variant" } else { "Nozzle variant" }}</span>
                {variants.into_iter().map(|v| {
                    let pressed = chosen.as_deref() == Some(v.as_str());
                    let value = v.clone();
                    view! {
                        <button type="button" class="nd-tab cal-variant-option"
                            aria-pressed=pressed.to_string() disabled=waiting
                            on:click=move |_| pick(value.clone())>
                            {untrusted_text(&v, NAME_MAX)}
                        </button>
                    }
                }).collect_view()}
                {move || error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            </div>
        })
    }
}

/// The five steps. Any can be opened; the order is a recommendation.
#[component]
fn CalStrip(
    run: RwSignal<Option<RunView>>,
    open: RwSignal<StepKey>,
    fine: RwSignal<bool>,
) -> impl IntoView {
    let offset = offset_now();
    let items = StepKey::ALL
        .iter()
        .enumerate()
        .map(|(i, &step)| {
            let status = Memo::new(move |_| {
                run.with(|r| {
                    r.as_ref()
                        .and_then(|r| r.steps.iter().find(|s| s.step == step))
                        .map(|s| s.status.clone())
                        .unwrap_or(StepStatus::NotRun)
                })
            });
            let state = move || {
                if open.get() == step {
                    "current"
                } else if matches!(status.get(), StepStatus::Saved { .. } | StepStatus::Recorded { .. }) {
                    "done"
                } else {
                    "upcoming"
                }
            };
            view! {
                <li class="nd-step cal-step" data-step=step.key() data-state=state
                    data-status=move || status.with(status_key)>
                    <button type="button" class="nd-step-btn"
                        aria-current=move || (open.get() == step).then_some("step")
                        on:click=move |_| {
                            fine.set(false);
                            open.set(step);
                        }>
                        <span class="nd-step-num">{format!("{:02}", i + 1)}</span>
                        <span class="nd-step-name">{step_name(step)}</span>
                        <span class="cal-step-status">{move || status.with(|s| status_text(s, offset))}</span>
                    </button>
                </li>
            }
        })
        .collect_view();
    view! { <ol class="nd-steps cal-steps" role="list" aria-label="Calibration steps">{items}</ol> }
}

/// Flow's Coarse | Fine switch: fine only after a saved coarse pass.
#[component]
fn FlowPasses(
    run: RwSignal<Option<RunView>>,
    fine: RwSignal<bool>,
    fine_pass: bool,
) -> impl IntoView {
    let ready = Memo::new(move |_| {
        run.with(|r| {
            r.as_ref()
                .and_then(|r| r.steps.iter().find(|s| s.step == StepKey::Flow))
                .is_some_and(|s| s.fine_ready)
        })
    });
    view! {
        <div class="cal-passes" role="group" aria-label="Flow rate pass">
            <button type="button" class="nd-tab cal-pass" data-pass="coarse"
                aria-pressed=(!fine_pass).to_string() on:click=move |_| fine.set(false)>"Coarse"</button>
            <button type="button" class="nd-tab cal-pass" data-pass="fine"
                aria-pressed=fine_pass.to_string() disabled=move || !ready.get()
                on:click=move |_| fine.set(true)>"Fine"</button>
        </div>
        <Show when=move || !ready.get()>
            <p class="cal-quiet cal-fine-note">{FINE_AFTER_COARSE}</p>
        </Show>
    }
}

/// The open step: set up in Bambu Studio, read the print, save.
#[component]
fn StepCard(
    run: RwSignal<Option<RunView>>,
    order: StoredValue<AnswerOrder>,
    pending: RwSignal<u32>,
    step: StepKey,
    fine: RwSignal<bool>,
    fine_pass: bool,
    studio_was_open: RwSignal<bool>,
) -> impl IntoView {
    let test = test_for(step, fine_pass);
    let is_pa = test == TestKind::PressureAdvance;
    let run_id = run
        .with_untracked(|r| r.as_ref().map(|r| r.id))
        .unwrap_or_default();
    // The card is rebuilt when the variant changes, so this holds for its
    // life: with no variant chosen it only asks for one.
    let locked = run.with_untracked(|r| r.as_ref().is_some_and(|r| variant_needed(r, test)));
    let setup = RwSignal::new(None::<Setup>);
    let material = RwSignal::new(None::<String>);
    let current = RwSignal::new(None::<String>);
    let reading = RwSignal::new(None::<Reading>);
    let preview = RwSignal::new(None::<PreviewView>);
    let card_error = RwSignal::new(None::<String>);
    let save_error = RwSignal::new(None::<String>);
    // A typed value that isn't a reading: a slip, said quietly.
    let typed_note = RwSignal::new(None::<String>);
    let changed = RwSignal::new(false);
    let busy = RwSignal::new(false);
    // The setup and reading this card last saved or recorded: Save and
    // Record K wait for a different one.
    let done_for = RwSignal::new(None::<(Setup, Reading)>);
    let notes = RwSignal::new(Vec::<String>::new());
    let photo = RwSignal::new(None::<String>);
    let suggestion = RwSignal::new(None::<Suggestion>);
    let photo_note = RwSignal::new(None::<String>);
    let photo_error = RwSignal::new(None::<String>);
    let ask = StoredValue::new(0u64);
    let photo_ask = StoredValue::new(0u64);
    let file_input = NodeRef::<html::Input>::new();
    let card_ref = NodeRef::<html::Article>::new();

    if !locked {
        spawn_local(async move {
            match bridge::defaults(run_id, test).await {
                Ok(d) => {
                    material.try_set(d.material_label);
                    current.try_set(d.current);
                    setup.try_set(Some(d.setup));
                }
                Err(e) => {
                    card_error.try_set(Some(e));
                }
            }
        });
    }

    // The diagram, and the value once there is a reading.
    Effect::new(move |_| {
        let Some(s) = setup.get() else { return };
        let r = reading.get();
        let Some(generation) = ask.try_update_value(|g| {
            *g += 1;
            *g
        }) else {
            return;
        };
        spawn_local(async move {
            let answer = bridge::preview(run_id, test, &s, r.as_ref()).await;
            if ask.try_get_value() != Some(generation) {
                return;
            }
            match answer {
                Ok(mut p) => {
                    if r.is_none() {
                        p.value = None;
                    }
                    card_error.try_set(None);
                    preview.try_set(Some(p));
                }
                Err(e) => {
                    card_error.try_set(Some(e));
                    preview.try_update(|p| {
                        if let Some(p) = p {
                            p.value = None;
                        }
                    });
                }
            }
        });
    });

    let diagram = Memo::new(move |_| preview.with(|p| p.as_ref().map(|p| p.diagram.clone())));
    let value = Memo::new(move |_| preview.with(|p| p.as_ref().and_then(|p| p.value.clone())));
    let first_result_pending =
        Memo::new(move |_| run.with(|r| r.as_ref().is_some_and(|r| r.results.is_empty())));
    let skipped = Memo::new(move |_| {
        run.with(|r| {
            r.as_ref()
                .and_then(|r| r.steps.iter().find(|s| s.step == step))
                .is_some_and(|s| s.status == StepStatus::Skipped)
        })
    });
    // Undo lasts: the step's newest save, while the backend allows it.
    let undo_offer = Memo::new(move |_| {
        run.with(|r| {
            r.as_ref()
                .and_then(|r| undo_target(r, step))
                .map(|x: &ResultView| (x.id, undo_note(x)))
        })
    });
    // Nothing is read while a save, record or undo is out, so its answer
    // can't land next to a newer reading.
    let reading_locked = Signal::derive(move || locked || busy.get());
    let done_now =
        move || done_for.get().is_some() && done_for.get() == setup.get().zip(reading.get());

    let choose = move |r: Reading| {
        changed.set(false);
        save_error.set(None);
        card_error.set(None);
        typed_note.set(None);
        reading.set(Some(r));
    };
    let set_field = move |key: &'static str, text: String| {
        if let Some(next) = setup
            .get_untracked()
            .and_then(|s| with_field(&s, key, &text))
        {
            reading.set(None);
            setup.set(Some(next));
        }
    };
    let typed = move |text: String| {
        let Some(s) = setup.get_untracked() else {
            return;
        };
        match typed_reading(&s, &text) {
            Some(Ok(r)) => choose(r),
            Some(Err(why)) => typed_note.set(Some(why.into())),
            None => typed_note.set(None),
        }
    };
    let open_studio = move |_| {
        spawn_local(async move {
            if let Err(e) = commands::launch_bambu_studio(None, None).await {
                save_error.try_set(Some(e));
            }
        });
    };
    let save = move |_| {
        let (Some(s), Some(r)) = (setup.get_untracked(), reading.get_untracked()) else {
            return;
        };
        if pending.get_untracked() > 0 {
            return;
        }
        // `old` goes back raw (`"nil"` included) as `expectedOld`, and the
        // variant it was read at as `expectedVariant`.
        let Some((old, variant)) = value.with_untracked(|v| {
            v.as_ref()
                .and_then(|v| v.change.as_ref())
                .map(|c| (c.old.clone(), c.variant.clone()))
        }) else {
            return;
        };
        let photo_name = photo.get_untracked();
        let ticket = ticket(order);
        begin(pending);
        busy.set(true);
        save_error.set(None);
        changed.set(false);
        notes.set(Vec::new());
        spawn_local(async move {
            let answer = bridge::save(
                run_id,
                test,
                &s,
                &r,
                &old,
                variant.as_deref(),
                photo_name.as_deref(),
            )
            .await;
            end(pending);
            busy.try_set(false);
            match answer {
                Ok(SaveAnswer::Saved {
                    run: next,
                    studio_running,
                    assign_error,
                    ..
                }) => {
                    if !adopt(run, order, run_id, ticket, next) {
                        return;
                    }
                    done_for.try_set(Some((s, r)));
                    notes.try_set(save_notes(studio_running, assign_error.as_deref()));
                    studio_was_open
                        .try_update(|w| *w = restart_before_fine(*w, test, studio_running));
                    focus_in(card_ref, &[".cal-undo", ".cal-card-title"]);
                }
                Ok(SaveAnswer::Changed { preview: p }) => {
                    // Newer than any preview still out: it must not be overwritten.
                    ask.try_update_value(|g| *g += 1);
                    changed.try_set(true);
                    preview.try_set(Some(p));
                }
                Err(e) => {
                    save_error.try_set(Some(e));
                }
            }
        });
    };
    let undo = move |_| {
        if pending.get_untracked() > 0 {
            return;
        }
        let Some(id) = undo_offer.with_untracked(|o| o.as_ref().map(|(id, _)| *id)) else {
            return;
        };
        let ticket = ticket(order);
        begin(pending);
        busy.set(true);
        save_error.set(None);
        spawn_local(async move {
            let answer = bridge::undo(id).await;
            end(pending);
            busy.try_set(false);
            match answer {
                Ok(next) => {
                    if !adopt(run, order, run_id, ticket, next) {
                        return;
                    }
                    done_for.try_set(None);
                    notes.try_set(Vec::new());
                    // The preset is back: ask for the diff again.
                    reading.try_update(|_| {});
                    focus_in(
                        card_ref,
                        &[
                            ".cal-undo",
                            ".cal-save-btn:not(:disabled)",
                            ".cal-card-title",
                        ],
                    );
                }
                Err(e) => {
                    save_error.try_set(Some(e));
                }
            }
        });
    };
    let record = move |_| {
        let (Some(s), Some(r)) = (setup.get_untracked(), reading.get_untracked()) else {
            return;
        };
        if pending.get_untracked() > 0 {
            return;
        }
        let photo_name = photo.get_untracked();
        let ticket = ticket(order);
        begin(pending);
        busy.set(true);
        save_error.set(None);
        spawn_local(async move {
            let answer = bridge::record(run_id, &s, &r, photo_name.as_deref()).await;
            end(pending);
            busy.try_set(false);
            match answer {
                Ok(next) => {
                    if !adopt(run, order, run_id, ticket, next) {
                        return;
                    }
                    done_for.try_set(Some((s, r)));
                    notes.try_set(vec!["Recorded. BambuMate never writes K anywhere.".into()]);
                    focus_in(card_ref, &[".cal-card-title"]);
                }
                Err(e) => {
                    save_error.try_set(Some(e));
                }
            }
        });
    };
    let toggle_skip = move |_| {
        if busy.get_untracked() || pending.get_untracked() > 0 {
            return;
        }
        let next = !skipped.get_untracked();
        let ticket = ticket(order);
        begin(pending);
        busy.set(true);
        spawn_local(async move {
            let answer = bridge::skip(run_id, step, next).await;
            end(pending);
            busy.try_set(false);
            match answer {
                Ok(r) => {
                    adopt(run, order, run_id, ticket, r);
                }
                Err(e) => {
                    save_error.try_set(Some(e));
                }
            }
        });
    };
    let pick_photo = move |_| {
        if let Some(input) = file_input.get_untracked() {
            input.click();
        }
    };
    let photo_chosen = move |ev: leptos::ev::Event| {
        let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        let Some(file) = input.files().and_then(|f| f.get(0)) else {
            return;
        };
        input.set_value("");
        let Some(s) = setup.get_untracked() else {
            return;
        };
        let Some(generation) = photo_ask.try_update_value(|g| {
            *g += 1;
            *g
        }) else {
            return;
        };
        suggestion.set(None);
        photo_error.set(None);
        photo_note.set(Some("Reading the photo…".into()));
        spawn_local(async move {
            let answer = match read_file_as_base64(file).await {
                Ok((_, data)) => bridge::suggest(run_id, test, &s, data).await,
                Err(e) => Err(e),
            };
            // A newer photo's answer wins, whichever lands first.
            if photo_ask.try_get_value() != Some(generation) {
                return;
            }
            match answer {
                Ok(a) => {
                    photo.try_set(Some(a.photo));
                    suggestion.try_set(a.suggestion);
                    photo_note.try_set(a.note);
                }
                Err(e) => {
                    photo_note.try_set(None);
                    photo_error.try_set(Some(e));
                }
            }
        });
    };
    // The AI's pick is a reading only once the user taps Use this.
    let accept = move |_| {
        let (Some(s), Some(sg), Some(d)) = (
            setup.get_untracked(),
            suggestion.get_untracked(),
            diagram.get_untracked(),
        ) else {
            return;
        };
        if let Some(r) = reading_for_pick(&s, &d, &sg.pick) {
            choose(r);
        }
    };
    let can_save = move || {
        !busy.get()
            && pending.get() == 0
            && !locked
            && !done_now()
            && value.with(|v| {
                v.as_ref()
                    .is_some_and(|v| v.change.is_some() && v.change_error.is_none())
            })
    };
    // A save, undo, record, skip, link or variant pick is out.
    let waiting = move || busy.get() || pending.get() > 0;
    let can_record =
        move || !busy.get() && pending.get() == 0 && reading.with(Option::is_some) && !done_now();
    let fields = move || setup.with(|s| s.map(|s| setup_fields(&s)).unwrap_or_default());
    let (typed_text, typed_unit) = typed_label(test);

    view! {
        <article class="cal-card" data-test=test.key() data-locked=locked.to_string() node_ref=card_ref>
            <header class="cal-card-head">
                <h2 class="cal-card-title" tabindex="-1">{step_name(step)}</h2>
                {(step == StepKey::Flow).then(|| view! { <FlowPasses run=run fine=fine fine_pass=fine_pass /> })}
            </header>
            {locked.then(|| view! { <p class="cal-pick-variant">{PICK_VARIANT_PROMPT}</p> })}
            {move || card_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            {(test == TestKind::FlowFine).then(|| view! {
                <p class="cal-note cal-restart" role="status">
                    {move || if studio_was_open.get() { RESTART_BEFORE_FINE } else { "" }}
                </p>
            })}

            <section class="cal-part cal-setup">
                <h3 class="nd-label">"Set up in Bambu Studio"</h3>
                <ul class="cal-studio">
                    {move || {
                        setup
                            .get()
                            .map(|s| studio_steps(&s, material.get().as_deref()))
                            .unwrap_or_default()
                            .into_iter()
                            .map(|line| view! { <li>{line}</li> })
                            .collect_view()
                    }}
                </ul>
                {move || current_line(test, current.get().as_deref()).map(|c| view! { <p class="cal-quiet cal-current">{c}</p> })}
                <div class="cal-fields">
                    <For each=fields key=|f| f.key children=move |f| {
                        let key = f.key;
                        view! {
                            <label class="cal-field">
                                <span class="nd-label">{f.label}</span>
                                <input type="number" step="any" class="cal-input" data-field=key
                                    prop:value=f.value disabled=move || busy.get()
                                    on:change=move |ev| set_field(key, event_target_value(&ev)) />
                                <span class="cal-unit">{f.unit}</span>
                            </label>
                        }
                    } />
                </div>
                <div class="cal-row">
                    <button type="button" class="btn btn-secondary cal-open-studio" on:click=open_studio>
                        "Open Bambu Studio"
                    </button>
                </div>
                <Show when=move || !is_pa && first_result_pending.get()>
                    <p class="cal-quiet cal-dev-mode">{DEVELOPER_MODE}</p>
                </Show>
                {(!is_pa).then(|| view! { <p class="cal-quiet">{DISCARD_CHANGES}</p> })}
            </section>

            <section class="cal-part cal-read">
                <h3 class="nd-label">{if is_pa { "Record K" } else { "Read the print" }}</h3>
                {move || diagram.get().map(|d| view! {
                    <DiagramView diagram=d test=test reading=reading disabled=reading_locked
                        choose=Callback::new(choose) />
                })}
                <label class="cal-field cal-typed">
                    <span class="nd-label">{typed_text}</span>
                    <input type="number" step="any" class="cal-input cal-typed-input"
                        disabled=move || reading_locked.get()
                        on:change=move |ev| typed(event_target_value(&ev)) />
                    <span class="cal-unit">{typed_unit}</span>
                </label>
                <p class="cal-quiet cal-typed-note" role="status">
                    {move || typed_note.get().unwrap_or_default()}
                </p>
                <div class="cal-row">
                    <button type="button" class="btn btn-secondary cal-add-photo"
                        disabled=move || reading_locked.get() on:click=pick_photo>"Add a photo"</button>
                    <input type="file" accept="image/*" class="cal-photo-input" hidden node_ref=file_input
                        on:change=photo_chosen />
                    {move || photo.get().is_some().then(|| view! { <span class="cal-quiet">"Photo kept"</span> })}
                </div>
                <p class="cal-quiet cal-photo-note" role="status">
                    {move || photo_note.get().unwrap_or_default()}
                </p>
                {move || photo_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
                {move || suggestion.get().map(|sg| view! {
                    <div class="cal-suggestion">
                        <p class="cal-suggestion-line">
                            <span>{suggestion_line(test, &sg.pick)}</span>
                            <button type="button" class="btn btn-secondary cal-accept"
                                disabled=move || reading_locked.get() on:click=accept>"Use this"</button>
                        </p>
                        <p class="cal-quiet cal-reason">{suggestion_detail(&sg)}</p>
                    </div>
                })}
            </section>

            <section class="cal-part cal-save">
                {move || value.get().map(|v| {
                    let detail = v
                        .change
                        .as_ref()
                        .map(diff_line)
                        .or_else(|| v.change_error.clone())
                        .unwrap_or_default();
                    let extra = v.change.as_ref().and_then(extra_line);
                    view! {
                        <Hero label=hero_label(test).to_string() value=v.text.clone()
                            unit=(!test.unit().is_empty()).then(|| test.unit().to_string()) detail=detail />
                        {extra.map(|e| view! { <p class="cal-quiet cal-extra">{e}</p> })}
                    }
                })}
                {move || {
                    (!is_pa)
                        .then(|| run.with(|r| r.as_ref().and_then(will_save_line)))
                        .flatten()
                        .map(|line| view! { <p class="cal-will-save">{line}</p> })
                }}
                <div class="cal-actions">
                    {if is_pa {
                        view! {
                            <button type="button" class="btn btn-primary cal-record"
                                disabled=move || !can_record() on:click=record>"Record K"</button>
                        }
                        .into_any()
                    } else {
                        view! {
                            <button type="button" class="btn btn-primary cal-save-btn"
                                disabled=move || !can_save() on:click=save>"Save to preset"</button>
                            {move || undo_offer.get().map(|(id, _)| view! {
                                <button type="button" class="btn btn-secondary cal-undo"
                                    data-result=id.to_string()
                                    disabled=waiting on:click=undo>"Undo"</button>
                            })}
                        }
                        .into_any()
                    }}
                    <button type="button" class="btn btn-secondary cal-skip"
                        disabled=waiting on:click=toggle_skip>
                        {move || if skipped.get() { "Unskip" } else { "Skip" }}
                    </button>
                </div>
                {move || undo_offer.get().map(|(_, note)| view! { <p class="cal-quiet cal-undo-note">{note}</p> })}
                {is_pa.then(|| view! { <p class="cal-quiet">{PA_NOTE}</p> })}
                <p class="cal-changed" role="status">
                    {move || if changed.get() { CHANGED_SINCE_LINE } else { "" }}
                </p>
                {move || save_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
                <div class="cal-notes" role="status">
                    {move || notes.get().into_iter().map(|n| view! { <p class="cal-quiet cal-note">{n}</p> }).collect_view()}
                </div>
                <LinkPrint run=run order=order pending=pending step=step />
            </section>
        </article>
    }
}

/// Bands as a column (hottest at the bottom), patches as a row, heights as
/// a ruler. The chosen one is `aria-pressed` and takes the signal colour.
#[component]
fn DiagramView(
    diagram: Diagram,
    test: TestKind,
    reading: RwSignal<Option<Reading>>,
    disabled: Signal<bool>,
    choose: Callback<Reading>,
) -> impl IntoView {
    let pressed = move |r: Option<Reading>| move || (r.is_some() && reading.get() == r).to_string();
    let off = move || disabled.get();
    match diagram {
        Diagram::Bands { bands } => view! {
            <div class="cal-bands" role="group" aria-label="Bands, hottest at the bottom">
                {bands.into_iter().rev().map(|b| {
                    let r = Some(Reading::Temperature { band: b.index });
                    view! {
                        <button type="button" class="cal-band" data-index=b.index.to_string()
                            aria-pressed=pressed(r) disabled=off
                            on:click=move |_| if let Some(r) = r { choose.run(r) }>{b.label}</button>
                    }
                }).collect_view()}
            </div>
        }
        .into_any(),
        Diagram::Patches { patches } => view! {
            <div class="cal-patches" role="group" aria-label="Patches">
                {patches.into_iter().map(|p| {
                    let r = patch_reading(test, p.modifier);
                    view! {
                        <button type="button" class="cal-patch" data-modifier=p.modifier.to_string()
                            aria-pressed=pressed(r) disabled=off
                            on:click=move |_| if let Some(r) = r { choose.run(r) }>{p.label}</button>
                    }
                }).collect_view()}
            </div>
        }
        .into_any(),
        Diagram::Ruler { max_mm } => view! {
            <div class="cal-ruler" role="group" aria-label="Height from the bed, in mm">
                {ruler_ticks(max_mm).into_iter().rev().map(|mm| {
                    let r = height_reading(test, f64::from(mm));
                    view! {
                        <button type="button" class="cal-tick" data-mm=mm.to_string()
                            aria-pressed=pressed(r) disabled=off
                            on:click=move |_| if let Some(r) = r { choose.run(r) }>{format!("{mm} mm")}</button>
                    }
                }).collect_view()}
            </div>
        }
        .into_any(),
        Diagram::None {} => ().into_any(),
    }
}

/// Once the step has a result: the newest print that ended after the step
/// was opened first, then the four before it.
#[component]
fn LinkPrint(
    run: RwSignal<Option<RunView>>,
    order: StoredValue<AnswerOrder>,
    pending: RwSignal<u32>,
    step: StepKey,
) -> impl IntoView {
    let offset = offset_now();
    let run_id = run
        .with_untracked(|r| r.as_ref().map(|r| r.id))
        .unwrap_or_default();
    let result_id = Memo::new(move |_| run.with(|r| r.as_ref().and_then(|r| step_result(r, step))));
    let linked = Memo::new(move |_| {
        run.with(|r| {
            let r = r.as_ref()?;
            let id = step_result(r, step)?;
            r.results.iter().find(|x| x.id == id)?.journal_entry_id
        })
    });
    let candidates = RwSignal::new(None::<LinkCandidates>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let waiting = move || busy.get() || pending.get() > 0;
    let open_list = move |_| {
        let Some(id) = result_id.get_untracked() else {
            return;
        };
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let answer = bridge::link_candidates(id).await;
            busy.try_set(false);
            match answer {
                Ok(c) => {
                    candidates.try_set(Some(c));
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    let link = move |entry: i64| {
        if pending.get_untracked() > 0 {
            return;
        }
        let Some(id) = result_id.get_untracked() else {
            return;
        };
        let ticket = ticket(order);
        begin(pending);
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let answer = bridge::link_print(id, entry).await;
            end(pending);
            busy.try_set(false);
            match answer {
                Ok(r) => {
                    candidates.try_set(None);
                    adopt(run, order, run_id, ticket, r);
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    move || {
        result_id.get().map(|_| view! {
            <div class="cal-link">
                {move || linked.get().map(|entry| view! {
                    <p class="cal-linked">
                        "Linked to a print · "
                        <a class="cal-text-link" href=entry_href(entry)>"Open in the journal"</a>
                    </p>
                })}
                <button type="button" class="btn btn-secondary cal-link-print"
                    disabled=move || busy.get() on:click=open_list>"Link the print"</button>
                {move || candidates.get().map(|c| {
                    let suggested = c.suggested;
                    view! {
                        <ul class="cal-candidates">
                            {c.entries.is_empty().then(|| view! {
                                <li><StateText kind=StateKind::Empty message="No finished prints in the journal yet." /></li>
                            })}
                            {c.entries.into_iter().map(|e| {
                                let id = e.id;
                                let taken = result_id.get_untracked().and_then(|r| linked_elsewhere(&e, r));
                                let is_taken = taken.is_some().to_string();
                                view! {
                                    <li>
                                        <button type="button" class="cal-candidate" data-entry=id.to_string()
                                            data-taken=is_taken
                                            disabled=waiting
                                            on:click=move |_| link(id)>
                                            {(Some(id) == suggested).then(|| view! { <span class="nd-label cal-suggested">"Suggested"</span> })}
                                            <span class="cal-candidate-text">
                                                <span>{candidate_line(&e, offset)}</span>
                                                {taken.map(|t| view! { <span class="cal-quiet cal-taken">{t}</span> })}
                                            </span>
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }
                })}
                {move || error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            </div>
        })
    }
}

/// A quiet table of every result in the run, newest first. Rows are keyed
/// by result, so a run update re-renders (and re-asks a thumbnail for)
/// only a row that changed.
#[component]
fn History(run: RwSignal<Option<RunView>>) -> impl IntoView {
    let offset = offset_now();
    let rows =
        Memo::new(move |_| run.with(|r| r.as_ref().map(|r| r.results.clone()).unwrap_or_default()));
    let printer = Memo::new(move |_| {
        run.with(|r| {
            r.as_ref()
                .map(|r| {
                    if r.nozzle.is_empty() {
                        r.printer_model.clone()
                    } else {
                        format!("{} · {} mm", r.printer_model, r.nozzle)
                    }
                })
                .unwrap_or_default()
        })
    });
    view! {
        <Show when=move || rows.with(|r| !r.is_empty())>
            <section class="cal-history" aria-labelledby="cal-history-title">
                <h3 class="nd-label" id="cal-history-title">"History"</h3>
                <table class="cal-history-table">
                    <thead>
                        <tr>
                            <th scope="col">"Date"</th>
                            <th scope="col">"Test"</th>
                            <th scope="col">"Value"</th>
                            <th scope="col">"Preset"</th>
                            <th scope="col">"Printer"</th>
                            <th scope="col">"Photo"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=move || rows.get()
                            key=|x| (x.id, x.undone, x.journal_entry_id)
                            children=move |x| {
                                let printer = untrusted_text(&printer.get_untracked(), 48);
                                view! {
                                    <tr data-result=x.id.to_string() data-undone=x.undone.to_string()>
                                        <td>{date_text(x.created_at, offset)}</td>
                                        <td><a class="cal-text-link" href=result_href(x.id)>{x.test.title()}</a></td>
                                        <td>
                                            {history_value(&x)}
                                            {history_change(&x).map(|c| view! { <span class="cal-history-change">{c}</span> })}
                                        </td>
                                        <td>{untrusted_text(&short_preset(&x.target_name), NAME_MAX)}</td>
                                        <td>{printer}</td>
                                        <td>{x.photo.clone().map(|file| view! { <Thumb file=file /> })}</td>
                                    </tr>
                                }
                            }
                        />
                    </tbody>
                </table>
            </section>
        </Show>
    }
}

/// A kept photo, asked for when its row renders.
#[component]
fn Thumb(file: String) -> impl IntoView {
    let url = RwSignal::new(None::<String>);
    spawn_local(async move {
        if let Ok(u) = bridge::image(&file).await {
            url.try_set(Some(u));
        }
    });
    move || {
        url.get()
            .map(|u| view! { <img class="cal-thumb" src=u alt="Photo of the print" /> })
    }
}
