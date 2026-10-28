//! Home: a cockpit that leads with whatever matters now (spec 2026-10-17
//! §3). The printer decides the hero (see `home_state::home_mode`); below
//! it come the AMS slots and a footer with recent work and sync status.
//! Read-only: nothing here sends a command to the printer.

use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::agent::drawer::{AgentRefresh, AgentShared};
use crate::app::FeatureFlagsContext;
use crate::camera::view::CameraThumb;
use crate::commands::{self, GeneratedPreset};
use crate::components::patterns::{Hero, SegmentedBar, StateKind, StateText, Tone};
use crate::journal::format::{rate_href, RATE_LAST};
use crate::journal::types::PrintEntry;
use crate::journal::{bridge as journal_bridge, next_revision, JournalShared};
use crate::pages::home_state::{
    active_slot, ams_line, ask_about, attention_detail, attention_label, attention_word, home_mode,
    idle_label, idle_word, last_print_detail, mode_key, no_printer_label, percent_text,
    presets_unit, printing_detail, printing_label, recent_line, sync_line, HomeMode, HOME_SEGMENTS,
};
use crate::pages::printer::{CalibrateLink, SlotDetection, SlotPicker};
use crate::printer::ams::{ams_match, JobFilament};
use crate::printer::types::{swatch_color, untrusted_text, ConnectionState, SlotView};
use crate::printer::PrinterShared;
use crate::slicer::state::newest_done;
use crate::slicer::SlicerShared;

/// How often the relative times ("3h ago") are brought up to date.
const CLOCK_TICK: Duration = Duration::from_secs(30);

/// Most characters shown of a filament type the printer reports.
const MATERIAL_MAX: usize = 16;

#[component]
pub fn HomePage() -> impl IntoView {
    let printer = expect_context::<PrinterShared>();
    let view = printer.view;
    // The newest ended print, from the journal, so it survives a restart.
    // Nothing is shown for it until the first answer arrives.
    let journal_changes = expect_context::<JournalShared>().changes;
    let last_print = RwSignal::new(None::<PrintEntry>);
    // The newest request already applied, so an older answer arriving late
    // can't put an earlier print (or an unrated one) back.
    let applied = StoredValue::new(0_u64);
    Effect::new(move |_| {
        journal_changes.track();
        let revision = next_revision();
        spawn_local(async move {
            if let Ok(entry) = journal_bridge::last().await {
                if applied.try_get_value().is_some_and(|a| a < revision) {
                    applied.try_set_value(revision);
                    last_print.try_set(entry);
                }
            }
        });
    });
    let flags = expect_context::<FeatureFlagsContext>().flags;
    let agent = expect_context::<AgentShared>();
    let refresh = expect_context::<AgentRefresh>().0;
    // Picks up anything missed while away.
    printer.refresh();

    let mode = Memo::new(move |_| view.with(home_mode));
    let connected = Memo::new(move |_| {
        view.with(|v| v.configured && v.connection == ConnectionState::Connected)
    });
    let slot_labels = Memo::new(move |_| {
        view.with(|v| v.slots.iter().map(|s| s.label.clone()).collect::<Vec<_>>())
    });
    let in_use = Memo::new(move |_| view.with(active_slot));
    let picking = RwSignal::new(Option::<SlotView>::None);
    // "3h ago" is worked out against this clock, so the relative times keep
    // up while Home stays open.
    let now = RwSignal::new(js_sys::Date::now());
    if let Ok(clock) = set_interval_with_handle(
        move || {
            now.try_set(js_sys::Date::now());
        },
        CLOCK_TICK,
    ) {
        on_cleanup(move || clock.clear());
    }

    // `None` until each call answers. They run on mount and again when the
    // agent changes profiles; never on a timer.
    let profiles = RwSignal::new(None::<Result<usize, String>>);
    let unsynced = RwSignal::new(None::<Result<usize, String>>);
    let recent = RwSignal::new(None::<Result<Option<GeneratedPreset>, String>>);
    Effect::new(move |_| {
        refresh.track();
        // Home can go away while these are in flight, so each write is a
        // try_set, which does nothing once the signal is disposed.
        spawn_local(async move {
            profiles.try_set(Some(commands::list_profiles().await.map(|l| l.len())));
        });
        spawn_local(async move {
            unsynced.try_set(Some(
                commands::list_unsynced_presets()
                    .await
                    .map(|l| l.presets.len()),
            ));
        });
        spawn_local(async move {
            recent.try_set(Some(commands::latest_generated_preset().await));
        });
    });

    let ask = move |_| {
        agent.prefill.set(Some(view.with_untracked(ask_about)));
        agent.open.set(true);
    };

    // The newest finished slice job, while its filament isn't loaded
    // exactly. Slice jobs last only for the session, so only the newest.
    let slicer_jobs = expect_context::<SlicerShared>().jobs;
    let ams_note = Memo::new(move |_| {
        let (name, filament) = slicer_jobs.with(|jobs| {
            newest_done(jobs).map(|(job, result)| {
                (
                    job.model_name.clone(),
                    JobFilament::main(result, &job.filament),
                )
            })
        })?;
        view.with(|v| ams_line(&name, &filament, &ams_match(&filament, v)))
    });

    let hero = move || match mode.get() {
        HomeMode::Attention => view! {
            <Hero
                label=Signal::derive(move || view.with(attention_label))
                value=Signal::derive(move || view.with(|v| attention_word(v).to_string()))
                detail=Signal::derive(move || view.with(attention_detail))
                tone=Tone::Accent
            />
            <div class="hm-actions">
                <a href="/printer" class="btn btn-primary">"Open printer"</a>
                <button type="button" class="btn btn-secondary hm-ask" on:click=ask>
                    "What does this mean?"
                </button>
            </div>
        }
        .into_any(),
        HomeMode::Printing => view! {
            <Hero
                label=Signal::derive(move || view.with(printing_label))
                value=Signal::derive(move || view.with(percent_text))
                unit="%"
                detail=Signal::derive(move || view.with(printing_detail))
            />
            <SegmentedBar
                label="Print progress"
                percent=Signal::derive(move || {
                    view.with(|v| v.state.as_ref().and_then(|s| s.mc_percent).unwrap_or(0))
                })
                total=HOME_SEGMENTS
            />
            <CameraThumb />
            <div class="hm-actions">
                <a href="/printer" class="btn btn-secondary">"Printer →"</a>
            </div>
        }
        .into_any(),
        HomeMode::Idle => view! {
            <Hero
                label=Signal::derive(move || view.with(idle_label))
                value=Signal::derive(move || view.with(|v| idle_word(v).to_string()))
                detail=Signal::derive(move || {
                    last_print.with(|p| last_print_detail(p.as_ref(), now.get()))
                })
            />
            <div class="hm-actions">
                <a href="/slice" class="btn btn-primary">"Slice a model"</a>
                <a href="/filament" class="btn btn-secondary">"New filament profile"</a>
                <Show when=move || flags.get().analysis_enabled>
                    <a href="/analysis" class="btn btn-secondary">"Analyze last print"</a>
                </Show>
            </div>
            {move || last_print.with(|p| p.as_ref().and_then(rate_href)).map(|href| view! {
                <a href=href class="hm-rate">{RATE_LAST}</a>
            })}
            {move || ams_note.get().map(|line| view! {
                <a href="/slice" class="hm-ams-line">{line}</a>
            })}
        }
        .into_any(),
        HomeMode::NoPrinter { disconnected } => view! {
            <Hero
                label=Signal::derive(move || view.with(no_printer_label))
                value=Signal::derive(move || match profiles.get() {
                    Some(Ok(n)) => n.to_string(),
                    Some(Err(_)) => "—".to_string(),
                    None => "…".to_string(),
                })
                unit=Signal::derive(move || match profiles.get() {
                    Some(Ok(n)) => presets_unit(n).to_string(),
                    _ => " presets".to_string(),
                })
                detail=Signal::derive(move || match unsynced.get() {
                    Some(Ok(n)) => sync_line(n),
                    _ => String::new(),
                })
            />
            {move || match profiles.get() {
                Some(Err(e)) => Some(view! { <StateText kind=StateKind::Error message=e /> }),
                _ => None,
            }}
            <div class="hm-actions">
                <a href="/filament" class="btn btn-primary">"New filament profile"</a>
                <a href="/slice" class="btn btn-secondary">"Slice a model"</a>
            </div>
            <p class="hm-connect-line">
                {if disconnected {
                    "The printer is offline. "
                } else {
                    "Connect your printer to see live status and the AMS here. "
                }}
                <a href="/settings#printer" class="hm-connect">
                    {if disconnected { "Check Settings → Printer →" } else { "Connect →" }}
                </a>
            </p>
        }
        .into_any(),
    };

    view! {
        <div class="page home-page" data-mode=move || mode_key(mode.get())>
            <h1 class="nd-visually-hidden">"Home"</h1>
            <section class="hm-hero">{hero}</section>

            <Show when=move || connected.get() && slot_labels.with(|l| !l.is_empty())>
                <section class="hm-ams" aria-label="AMS">
                    <p class="nd-label">"AMS"</p>
                    <div class="hm-slots">
                        <For
                            each=move || slot_labels.get()
                            key=|label| label.clone()
                            children=move |label| {
                                let item = Memo::new(move |_| {
                                    view.with(|v| {
                                        v.slots.iter().find(|s| s.label == label).cloned().unwrap_or_default()
                                    })
                                });
                                view! { <HomeSlot item=item in_use=in_use mode=mode picking=picking /> }
                            }
                        />
                    </div>
                </section>
            </Show>

            <footer class="hm-foot">
                <div class="hm-recent">
                    {move || match recent.get() {
                        Some(Ok(Some(p))) => recent_line(&p, now.get())
                            .map(|line| view! { <span>{line}</span> }.into_any()),
                        Some(Err(e)) => Some(view! { <StateText kind=StateKind::Error message=e /> }.into_any()),
                        _ => None,
                    }}
                </div>
                <div class="hm-sync">
                    {move || match unsynced.get() {
                        Some(Ok(0)) => view! { <span class="hm-sync-ok">{sync_line(0)}</span> }.into_any(),
                        Some(Ok(n)) => view! {
                            <a href="/settings/health" class="hm-sync-warn">{sync_line(n)}</a>
                        }.into_any(),
                        Some(Err(e)) => view! { <StateText kind=StateKind::Error message=e /> }.into_any(),
                        None => view! { <StateText kind=StateKind::Loading /> }.into_any(),
                    }}
                </div>
            </footer>

            {move || picking.get().map(|slot| view! { <SlotPicker item=slot picking=picking /> })}
        </div>
    }
}

/// One AMS slot. Clicking it opens the same preset picker as the Printer
/// page. The slot feeding the print is outlined: signal while printing,
/// accent when the printer needs attention.
#[component]
fn HomeSlot(
    item: Memo<SlotView>,
    in_use: Memo<Option<(u32, u32)>>,
    mode: Memo<HomeMode>,
    picking: RwSignal<Option<SlotView>>,
) -> impl IntoView {
    let outline = move || {
        let here = item.with(|s| Some((s.ams_id, s.tray_id))) == in_use.get();
        match (here, mode.get()) {
            (true, HomeMode::Printing) => Some("signal"),
            (true, HomeMode::Attention) => Some("accent"),
            _ => None,
        }
    };
    view! {
        <div class="hm-slot"
            data-slot-label=move || item.with(|s| s.label.clone())
            data-status=move || item.with(|s| s.status.key())
            data-in-use=outline>
            <button type="button" class="hm-slot-main" title="Choose the preset loaded here"
                on:click=move |_| picking.set(Some(item.get_untracked()))>
                <span class="hm-swatch" style:background=move || item.with(|s| swatch_color(&s.tray.tray_color))></span>
                <span class="hm-slot-label nd-mono">{move || item.with(|s| s.label.clone())}</span>
                <span class="hm-material">
                    {move || item.with(|s| {
                        if s.tray.empty { "—".to_string() } else { untrusted_text(&s.tray.tray_type, MATERIAL_MAX) }
                    })}
                </span>
                <span class="hm-slot-status">{move || item.with(|s| s.status.badge())}</span>
                <Show when=move || outline().is_some()>
                    <span class="hm-in-use nd-label">"In use"</span>
                </Show>
            </button>
            <SlotDetection item=item />
            <CalibrateLink item=item />
        </div>
    }
}
