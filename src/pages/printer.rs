//! The Printer page: current print, AMS slots with their status and the
//! steps to set a slot on the printer, and active errors. Read-only.
//!
//! Every string that comes from the printer (file name, printer and filament
//! names, HMS text, a presented serial) is rendered as a plain text node, and
//! the printer-supplied names are cut and cleaned by `untrusted_text` first.
//!
//! A state event arrives about twice a second during a print, so each part of
//! the page reads only what it shows through a `Memo`, and slot cards and
//! errors are keyed: a temperature change updates text in place and never
//! rebuilds a card someone is clicking.

use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, ProfileInfo};
use crate::printer::bridge;
use crate::printer::types::{
    connection_message, format_remaining, format_temp, nozzle_label, row_title, swatch_color,
    untrusted_text, ConnectionState, ErrorView, PrinterState, PrinterSummary, PrinterView,
    SlotStatus, SlotView,
};
use crate::printer::PrinterShared;

/// Picker rows shown at once; the search narrows the rest.
const PICKER_LIMIT: usize = 60;

/// Most characters shown of a printer or filament name the printer reports.
const NAME_MAX: usize = 64;

/// Most characters shown of the file name the printer reports.
const FILE_MAX: usize = 120;

#[component]
pub fn PrinterPage() -> impl IntoView {
    let shared = expect_context::<PrinterShared>();
    let view = shared.view;
    // Picks up assignments' cloud-sync state and anything missed while away.
    shared.refresh();
    let picking = RwSignal::new(Option::<SlotView>::None);

    let header = Memo::new(move |_| view.with(|v| v.printer.as_ref().map(ident)));
    let configured = Memo::new(move |_| view.with(|v| v.configured));
    let notice = Memo::new(move |_| {
        view.with(|v| {
            let ip = v.printer.as_ref().map(|p| p.ip.clone()).unwrap_or_default();
            (v.connection.clone(), ip)
        })
    });
    let waiting = Memo::new(move |_| view.with(waiting_for_report));
    let connected = Memo::new(move |_| view.with(|v| v.connection == ConnectionState::Connected));
    let rows = Memo::new(move |_| view.with(ams_rows));

    view! {
        <div class="page printer-page nd">
            <header class="pr-head">
                <h2>"Printer"</h2>
                {move || header.get().map(|text| view! { <span class="pr-ident nd-mono">{text}</span> })}
            </header>

            <Show
                when=move || configured.get()
                fallback=|| view! {
                    <div class="pr-empty">
                        <p>"No printer is set up yet."</p>
                        <a href="/settings#printer" class="pr-setup-link">"Set one up in Settings → Printer"</a>
                    </div>
                }
            >
                {move || {
                    let (connection, ip) = notice.get();
                    // The backend doesn't retry these, so point at the fix.
                    let needs_user = connection.needs_user();
                    connection_message(&connection, &ip).map(|m| view! {
                        <p class="pr-notice" data-state=connection.dot()>
                            {m}
                            {needs_user.then(|| view! {
                                " "
                                <a href="/settings#printer" class="pr-settings-link">"Open Settings → Printer"</a>
                            })}
                        </p>
                    })
                }}
                <Show
                    when=move || !waiting.get()
                    fallback=|| view! { <p class="pr-waiting">"Waiting for the first status report…"</p> }
                >
                    <div class="pr-body" class:pr-stale=move || !connected.get()>
                        <Hero view=view />
                        <section class="pr-section pr-ams">
                            <p class="nd-label">"AMS"</p>
                            <For
                                each=move || rows.with(|r| r.iter().map(|(id, _)| *id).collect::<Vec<_>>())
                                key=|id| *id
                                children=move |ams_id| view! { <AmsRow view=view rows=rows ams_id=ams_id picking=picking /> }
                            />
                        </section>
                        <Errors view=view />
                    </div>
                </Show>
            </Show>

            {move || picking.get().map(|slot| view! { <SlotPicker item=slot picking=picking /> })}
        </div>
    }
}

/// `Name · Model · Serial`, leaving out what's blank so the serial shows once.
fn ident(p: &PrinterSummary) -> String {
    [
        untrusted_text(&p.name, NAME_MAX),
        untrusted_text(&p.model, NAME_MAX),
        p.serial.clone(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Connected, but the printer hasn't sent a status report yet.
fn waiting_for_report(v: &PrinterView) -> bool {
    v.connection == ConnectionState::Connected && v.state.is_none()
}

/// Slots grouped by unit, AMS units first, external spools last.
fn ams_rows(v: &PrinterView) -> Vec<(u32, Vec<SlotView>)> {
    let mut rows: Vec<(u32, Vec<SlotView>)> = Vec::new();
    for s in &v.slots {
        match rows.iter_mut().find(|(id, _)| *id == s.ams_id) {
            Some((_, list)) => list.push(s.clone()),
            None => rows.push((s.ams_id, vec![s.clone()])),
        }
    }
    rows
}

/// `Humidity 21% · 27.0 °C` for an AMS unit.
fn unit_meta(v: &PrinterView, ams_id: u32) -> String {
    let Some(unit) = v
        .state
        .as_ref()
        .and_then(|s| s.ams_units.iter().find(|u| u.id == ams_id))
    else {
        return String::new();
    };
    let humidity = match (unit.humidity_pct, unit.humidity_level) {
        (Some(p), _) => format!("Humidity {p}%"),
        (None, Some(l)) => format!("Humidity level {l}"),
        _ => String::new(),
    };
    let temp = unit.temp.map(|t| format!("{t:.1} °C")).unwrap_or_default();
    [humidity, temp]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The preset line a card shows, and a secondary line for an assignment the
/// printer's report beats (an RFID spool) or that has nothing loaded (an
/// empty slot).
fn card_preset(s: &SlotView) -> (String, Option<String>) {
    let reported = || {
        let name = if s.tray.tray_sub_brands.is_empty() {
            &s.tray.tray_info_idx
        } else {
            &s.tray.tray_sub_brands
        };
        untrusted_text(name, NAME_MAX)
    };
    match (&s.assigned_preset, s.status) {
        (Some(p), SlotStatus::Rfid) => (reported(), Some(format!("Previously assigned: {p}"))),
        (Some(p), SlotStatus::Empty) => (String::new(), Some(format!("Assigned: {p}"))),
        // The assigned name, which `preset_name` prefers.
        (Some(_), _) => (s.preset_name(), None),
        (None, _) => (reported(), None),
    }
}

/// What a card tells the user to do under its badge.
#[derive(Debug, Clone, PartialEq)]
enum SlotGuide {
    None,
    /// Set the assigned preset on the printer.
    Steps {
        label: String,
        preset: String,
        needs_sync: bool,
    },
    /// The assigned preset has no filament id, so no report can confirm it.
    NoId {
        preset: String,
    },
}

fn slot_guide(s: &SlotView) -> SlotGuide {
    match (&s.assigned_preset, s.status) {
        (Some(preset), SlotStatus::Different) if s.preset_has_no_id => SlotGuide::NoId {
            preset: preset.clone(),
        },
        (Some(preset), SlotStatus::Different) => SlotGuide::Steps {
            label: s.label.clone(),
            preset: preset.clone(),
            needs_sync: s.needs_cloud_sync,
        },
        _ => SlotGuide::None,
    }
}

/// A memo over one part of the printer's state; a missing state reads as
/// the default one.
fn state_memo<T>(
    view: RwSignal<PrinterView>,
    f: impl Fn(&PrinterState) -> T + Send + Sync + 'static,
) -> Memo<T>
where
    T: PartialEq + Send + Sync + 'static,
{
    Memo::new(move |_| {
        view.with(|v| match &v.state {
            Some(s) => f(s),
            None => f(&PrinterState::default()),
        })
    })
}

#[component]
fn Hero(view: RwSignal<PrinterView>) -> impl IntoView {
    let gcode = state_memo(view, |s| s.gcode_state.clone());
    let percent = state_memo(view, |s| s.mc_percent);
    let layer = state_memo(view, |s| match (s.layer_num, s.total_layer_num) {
        (Some(l), Some(t)) if t > 0 => format!("{l} / {t}"),
        _ => "—".into(),
    });
    let remaining = state_memo(view, |s| s.mc_remaining_time);
    let file = state_memo(view, |s| {
        s.subtask_name
            .as_deref()
            .filter(|f| !f.is_empty())
            .map(|f| untrusted_text(f, FILE_MAX))
    });
    let nozzle_ids = state_memo(view, |s| s.nozzles.iter().map(|n| n.id).collect::<Vec<_>>());
    let bed = state_memo(view, |s| format_temp(s.bed_temp, s.bed_target_temp));
    view! {
        <section class="pr-section pr-hero">
            <p class="nd-label">"Current print"</p>
            <div class="pr-hero-top">
                <span class="pr-hero-state nd-mono">{move || gcode.get().unwrap_or_else(|| "—".into())}</span>
                <span class="pr-hero-percent">{move || percent.get().map(|p| format!("{p}%")).unwrap_or_else(|| "—".into())}</span>
            </div>
            <div class="pr-progress">
                <div class="pr-progress-fill" style:width=move || format!("{}%", percent.get().unwrap_or(0).min(100))></div>
            </div>
            <dl class="pr-facts">
                <div><dt>"Layer"</dt><dd class="pr-layer">{move || layer.get()}</dd></div>
                <div><dt>"Remaining"</dt><dd class="pr-remaining">{move || remaining.get().map(format_remaining).unwrap_or_else(|| "—".into())}</dd></div>
                <div class="pr-file"><dt>"File"</dt><dd>{move || file.get().unwrap_or_else(|| "—".into())}</dd></div>
            </dl>
            <div class="pr-temps">
                <For
                    each=move || nozzle_ids.get()
                    key=|id| *id
                    children=move |id| view! { <NozzleTemp view=view id=id nozzle_ids=nozzle_ids /> }
                />
                <div class="pr-temp">
                    <span class="nd-label">"Bed"</span>
                    <span class="nd-mono">{move || bed.get()}</span>
                </div>
            </div>
        </section>
    }
}

#[component]
fn NozzleTemp(view: RwSignal<PrinterView>, id: u32, nozzle_ids: Memo<Vec<u32>>) -> impl IntoView {
    let count = move || nozzle_ids.with(Vec::len);
    let active = state_memo(view, move |s| {
        s.nozzles.len() >= 2 && s.active_nozzle == Some(id)
    });
    let nozzle = state_memo(view, move |s| {
        s.nozzles
            .iter()
            .find(|n| n.id == id)
            .map(|n| {
                (
                    format_temp(n.temp, n.target_temp),
                    n.diameter.map(|d| format!("{d} mm")),
                )
            })
            .unwrap_or_default()
    });
    view! {
        <div class="pr-temp" class:pr-temp-active=move || active.get()>
            <span class="nd-label">{move || nozzle_label(id, count())}</span>
            <span class="nd-mono">{move || nozzle.with(|n| n.0.clone())}</span>
            <span class="pr-temp-note">{move || nozzle.with(|n| n.1.clone().unwrap_or_default())}</span>
        </div>
    }
}

#[component]
fn AmsRow(
    view: RwSignal<PrinterView>,
    rows: Memo<Vec<(u32, Vec<SlotView>)>>,
    ams_id: u32,
    picking: RwSignal<Option<SlotView>>,
) -> impl IntoView {
    let meta = Memo::new(move |_| view.with(|v| unit_meta(v, ams_id)));
    let slots_here = move || {
        rows.with(|r| {
            r.iter()
                .find(|(id, _)| *id == ams_id)
                .map(|(_, s)| s.iter().map(|s| s.tray_id).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    };
    view! {
        <div class="pr-ams-row" data-ams=ams_id>
            <div class="pr-ams-head">
                <span class="pr-ams-title">{row_title(ams_id)}</span>
                <span class="pr-ams-meta nd-mono">{move || meta.get()}</span>
            </div>
            <div class="pr-slots">
                <For
                    each=slots_here
                    key=|tray_id| *tray_id
                    children=move |tray_id| {
                        let item = Memo::new(move |_| {
                            rows.with(|r| {
                                r.iter()
                                    .find(|(id, _)| *id == ams_id)
                                    .and_then(|(_, s)| s.iter().find(|s| s.tray_id == tray_id))
                                    .cloned()
                                    .unwrap_or_default()
                            })
                        });
                        view! { <SlotCard item=item picking=picking /> }
                    }
                />
            </div>
        </div>
    }
}

/// One slot. The card and its button stay the same nodes for as long as the
/// slot exists; only their contents follow the slot's memo.
#[component]
fn SlotCard(item: Memo<SlotView>, picking: RwSignal<Option<SlotView>>) -> impl IntoView {
    let body = move || {
        let slot = item.get();
        let color = swatch_color(&slot.tray.tray_color);
        let material = if slot.tray.empty {
            "—".to_string()
        } else {
            untrusted_text(&slot.tray.tray_type, NAME_MAX)
        };
        let (preset, secondary) = card_preset(&slot);
        let remain = slot.rfid.then_some(slot.tray.remain).flatten();
        view! {
            <span class="pr-swatch" style:background=color></span>
            <span class="pr-slot-label nd-mono">{slot.label.clone()}</span>
            <span class="pr-material">{material}</span>
            <span class="pr-preset">{preset}</span>
            {secondary.map(|p| view! { <span class="pr-prev">{p}</span> })}
            <span class="pr-slot-foot">
                {remain.map(|r| view! { <span class="pr-remain nd-mono">{format!("{r}%")}</span> })}
                {slot.rfid.then(|| view! { <span class="pr-rfid nd-label">"RFID"</span> })}
                <span class="pr-status">{slot.status.badge()}</span>
            </span>
        }
    };
    let guide = move || {
        match item.with(slot_guide) {
        SlotGuide::None => None,
        SlotGuide::Steps {
            label,
            preset,
            needs_sync,
        } => Some(
            view! {
                <ol class="pr-steps">
                    <li>"On the printer: Filament → "{label.clone()}" → choose "<em>{preset.clone()}</em>"."</li>
                    <li>"Or in Bambu Studio: Device → AMS → "{label}" → "<em>{preset}</em>"."</li>
                    {needs_sync.then(|| view! {
                        <li class="pr-sync-note">"It must sync to Bambu Cloud first — open Bambu Studio while signed in."</li>
                    })}
                </ol>
            }
            .into_any(),
        ),
        SlotGuide::NoId { preset } => Some(
            view! {
                <p class="pr-no-id">
                    "BambuMate can't check this slot: "<em>{preset}</em>
                    " has no filament id. Set it on the printer and confirm by eye."
                </p>
            }
            .into_any(),
        ),
    }
    };
    view! {
        <div class="pr-slot" data-status=move || item.with(|s| s.status.key())
            data-label=move || item.with(|s| s.label.clone())>
            <button class="pr-slot-main" on:click=move |_| { picking.set(Some(item.get_untracked())); }
                title="Choose the preset loaded here">
                {body}
            </button>
            {guide}
        </div>
    }
}

/// Active errors, one row per code, first occurrence kept.
fn unique_errors(v: &PrinterView) -> Vec<ErrorView> {
    let mut out: Vec<ErrorView> = Vec::new();
    for e in &v.errors {
        if !out.iter().any(|o| o.code == e.code) {
            out.push(e.clone());
        }
    }
    out
}

#[component]
fn Errors(view: RwSignal<PrinterView>) -> impl IntoView {
    let errors = Memo::new(move |_| view.with(unique_errors));
    view! {
        <section class="pr-section pr-errors">
            <p class="nd-label">"Errors"</p>
            <Show
                when=move || errors.with(|e| !e.is_empty())
                fallback=|| view! { <p class="pr-quiet">"No active errors."</p> }
            >
                <ul class="pr-error-list">
                    <For
                        each=move || errors.with(|e| e.iter().map(|e| e.code.clone()).collect::<Vec<_>>())
                        key=|code| code.clone()
                        children=move |code| {
                            let shown = code.clone();
                            let error = Memo::new(move |_| {
                                errors.with(|e| e.iter().find(|e| e.code == code).cloned().unwrap_or_default())
                            });
                            view! {
                                <li class="pr-error">
                                    <span class="pr-error-code nd-mono">{shown}</span>
                                    {move || {
                                        let e = error.get();
                                        match e.text {
                                            Some(t) => view! { <span class="pr-error-text">{t}</span> }.into_any(),
                                            None => view! {
                                                <a class="pr-error-link" href=e.wiki_url target="_blank" rel="noopener">
                                                    "Look up this code on the Bambu wiki"
                                                </a>
                                            }.into_any(),
                                        }
                                    }}
                                </li>
                            }
                        }
                    />
                </ul>
            </Show>
        </section>
    }
}

/// Filters presets by a case-insensitive search, keeping the first
/// `PICKER_LIMIT`.
fn matching(list: &[ProfileInfo], query: &str) -> Vec<ProfileInfo> {
    let q = query.trim().to_lowercase();
    list.iter()
        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
        .take(PICKER_LIMIT)
        .cloned()
        .collect()
}

/// More presets match than `matching` shows.
fn more_than_shown(list: &[ProfileInfo], query: &str) -> bool {
    let q = query.trim().to_lowercase();
    list.iter()
        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
        .nth(PICKER_LIMIT)
        .is_some()
}

/// Puts focus back on a slot card's button once the picker has gone, if that
/// card is still on the page.
fn refocus_card(label: String) {
    set_timeout(
        move || {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Ok(cards) = doc.query_selector_all(".pr-slot") else {
                return;
            };
            for i in 0..cards.length() {
                let Some(card) = cards
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                if card.get_attribute("data-label").as_deref() == Some(label.as_str()) {
                    if let Some(button) = card
                        .query_selector(".pr-slot-main")
                        .ok()
                        .flatten()
                        .and_then(|b| b.dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        let _ = button.focus();
                    }
                    return;
                }
            }
        },
        Duration::ZERO,
    );
}

#[component]
fn SlotPicker(item: SlotView, picking: RwSignal<Option<SlotView>>) -> impl IntoView {
    let slot = item;
    let shared = expect_context::<PrinterShared>();
    let user = RwSignal::new(Vec::<ProfileInfo>::new());
    let system = RwSignal::new(Vec::<ProfileInfo>::new());
    let loading = RwSignal::new(true);
    let query = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let busy = RwSignal::new(false);
    let search = NodeRef::<html::Input>::new();

    // The picker or the whole page can go away while these are in flight
    // (Cancel, or the agent navigating), so every write after an await is a
    // `try_set`, which does nothing once the signal is disposed, and nothing
    // is read after one.
    spawn_local(async move {
        let mut failed = None;
        for (list, into) in [
            (commands::list_profiles().await, user),
            (commands::list_system_profiles().await, system),
        ] {
            match list {
                Ok(list) => {
                    into.try_set(list);
                }
                Err(e) => failed = failed.or(Some(e)),
            }
        }
        if let Some(e) = failed {
            error.try_set(Some(format!("Couldn't load presets: {e}")));
        }
        loading.try_set(false);
    });

    // Focus the search on open; Escape closes from anywhere; closing returns
    // focus to the card that opened the picker.
    Effect::new(move |_| {
        if let Some(input) = search.get() {
            let _ = input.focus();
        }
    });
    let keys = window_event_listener(leptos::ev::keydown, move |e| {
        if e.key() == "Escape" {
            picking.try_set(None);
        }
    });
    let label = slot.label.clone();
    on_cleanup(move || {
        keys.remove();
        refocus_card(label);
    });

    let (ams_id, tray_id) = (slot.ams_id, slot.tray_id);
    let settle = move |ticket: Option<crate::printer::Ticket>,
                       result: Result<PrinterView, String>| {
        match result {
            Ok(v) => {
                if let Some(ticket) = ticket {
                    shared.apply(ticket, v);
                }
                picking.try_set(None);
            }
            Err(e) => {
                busy.try_set(false);
                error.try_set(Some(e));
            }
        }
    };
    let start = move || {
        busy.set(true);
        error.set(None);
        shared.ticket()
    };
    let choose = move |path: String| {
        let ticket = start();
        spawn_local(
            async move { settle(ticket, bridge::assign_slot(ams_id, tray_id, &path).await) },
        );
    };
    let clear = move |_| {
        let ticket = start();
        spawn_local(async move { settle(ticket, bridge::clear_slot(ams_id, tray_id).await) });
    };
    let group = move |title: &'static str, list: Vec<ProfileInfo>| {
        (!list.is_empty()).then(|| {
            view! {
                <p class="nd-label pr-picker-group">{title}</p>
                <ul class="pr-picker-list">
                    {list.into_iter().map(|p| {
                        let path = p.path.clone();
                        view! {
                            <li><button class="pr-picker-item" disabled=move || busy.get()
                                on:click=move |_| choose(path.clone())>{p.name}</button></li>
                        }
                    }).collect::<Vec<_>>()}
                </ul>
            }
        })
    };
    // One line under the results: loading, nothing found, or more to find.
    let status = move || {
        if loading.get() {
            return Some("Loading presets…");
        }
        let q = query.get();
        let none = user.with(|l| matching(l, &q).is_empty())
            && system.with(|l| matching(l, &q).is_empty());
        if none && error.with(Option::is_none) {
            return Some("No presets match.");
        }
        (user.with(|l| more_than_shown(l, &q)) || system.with(|l| more_than_shown(l, &q)))
            .then_some("Refine your search to see more.")
    };
    let has_assignment = slot.assigned_preset.is_some();
    view! {
        <div class="pr-picker-backdrop" on:click=move |_| picking.set(None)>
            <div class="pr-picker" role="dialog" aria-modal="true" aria-label="Choose a preset"
                on:click=|e| e.stop_propagation()>
                <p class="nd-label">{format!("Slot {}", slot.label)}</p>
                <h3>"Which preset is loaded here?"</h3>
                <input class="pr-picker-search" type="search" placeholder="Search presets" node_ref=search
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev)) />
                <div class="pr-picker-results">
                    {move || group("Your presets", user.with(|l| matching(l, &query.get())))}
                    {move || group("Bambu presets", system.with(|l| matching(l, &query.get())))}
                </div>
                {move || status().map(|s| view! { <p class="pr-picker-status">{s}</p> })}
                {move || error.get().map(|e| view! { <p class="pr-picker-error">{e}</p> })}
                <div class="pr-picker-actions">
                    {has_assignment.then(|| view! {
                        <button class="pr-picker-clear" disabled=move || busy.get() on:click=clear>"Clear assignment"</button>
                    })}
                    <button class="pr-picker-cancel" on:click=move |_| picking.set(None)>"Cancel"</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{AmsUnit, PrinterState, PrinterSummary, Tray};

    fn slot(ams_id: u32, tray_id: u32) -> SlotView {
        SlotView {
            ams_id,
            tray_id,
            ..Default::default()
        }
    }

    #[test]
    fn slots_group_into_rows_in_order() {
        let v = PrinterView {
            slots: vec![slot(0, 0), slot(0, 1), slot(1, 0), slot(255, 254)],
            ..Default::default()
        };
        let rows = ams_rows(&v);
        let shape: Vec<(u32, usize)> = rows.iter().map(|(id, s)| (*id, s.len())).collect();
        assert_eq!(shape, vec![(0, 2), (1, 1), (255, 1)]);
    }

    #[test]
    fn unit_meta_prefers_percent_humidity() {
        let v = PrinterView {
            state: Some(PrinterState {
                ams_units: vec![AmsUnit {
                    id: 0,
                    humidity_level: Some(5),
                    humidity_pct: Some(21),
                    temp: Some(27.0),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(unit_meta(&v, 0), "Humidity 21% · 27.0 °C");
        assert_eq!(unit_meta(&v, 9), "");
    }

    #[test]
    fn preset_search_is_case_insensitive() {
        let p = |n: &str| ProfileInfo {
            name: n.into(),
            filament_type: None,
            filament_id: None,
            path: format!("/{n}.json"),
            is_user_profile: true,
        };
        let list = vec![p("Polymaker PolyLite PLA"), p("Bambu PETG HF")];
        assert_eq!(matching(&list, "polylite").len(), 1);
        assert_eq!(matching(&list, "").len(), 2);
    }

    fn assigned(status: SlotStatus) -> SlotView {
        SlotView {
            label: "A3".into(),
            tray: Tray {
                tray_sub_brands: "PLA Basic".into(),
                tray_info_idx: "GFA00".into(),
                ..Default::default()
            },
            assigned_preset: Some("Acme PLA".into()),
            status,
            ..Default::default()
        }
    }

    #[test]
    fn a_different_slot_shows_steps_unless_its_preset_has_no_id() {
        let mut s = assigned(SlotStatus::Different);
        s.needs_cloud_sync = true;
        assert_eq!(
            slot_guide(&s),
            SlotGuide::Steps {
                label: "A3".into(),
                preset: "Acme PLA".into(),
                needs_sync: true,
            }
        );
        s.preset_has_no_id = true;
        assert_eq!(
            slot_guide(&s),
            SlotGuide::NoId {
                preset: "Acme PLA".into()
            }
        );
        assert_eq!(slot_guide(&assigned(SlotStatus::Matches)), SlotGuide::None);
        assert_eq!(slot_guide(&assigned(SlotStatus::Rfid)), SlotGuide::None);
    }

    #[test]
    fn an_rfid_report_beats_a_stale_assignment() {
        // The printer's RFID reading is shown; the assignment only as history.
        assert_eq!(
            card_preset(&assigned(SlotStatus::Rfid)),
            (
                "PLA Basic".to_string(),
                Some("Previously assigned: Acme PLA".to_string())
            )
        );
        assert_eq!(
            card_preset(&assigned(SlotStatus::Matches)),
            ("Acme PLA".to_string(), None)
        );
        let mut plain = assigned(SlotStatus::Rfid);
        plain.assigned_preset = None;
        assert_eq!(card_preset(&plain), ("PLA Basic".to_string(), None));
    }

    #[test]
    fn an_empty_slot_shows_its_assignment_only_as_secondary_text() {
        let mut empty = assigned(SlotStatus::Empty);
        empty.tray = Tray {
            empty: true,
            ..Default::default()
        };
        assert_eq!(
            card_preset(&empty),
            (String::new(), Some("Assigned: Acme PLA".to_string()))
        );
    }

    #[test]
    fn printer_reported_names_are_shortened_and_stripped() {
        let mut s = assigned(SlotStatus::Rfid);
        s.assigned_preset = None;
        s.tray.tray_sub_brands = format!("\u{202E}{}", "S".repeat(100));
        let (preset, _) = card_preset(&s);
        assert_eq!(preset.chars().count(), NAME_MAX);
        assert!(preset.starts_with('\u{FFFD}'));
    }

    #[test]
    fn the_header_names_the_printer_once() {
        let p = |name: &str| PrinterSummary {
            name: name.into(),
            model: "H2D".into(),
            serial: "0948AB000000001".into(),
            ..Default::default()
        };
        assert_eq!(ident(&p("Workshop")), "Workshop · H2D · 0948AB000000001");
        assert_eq!(ident(&p("")), "H2D · 0948AB000000001");
        assert_eq!(
            ident(&p(&"N".repeat(100))),
            format!("{} · H2D · 0948AB000000001", "N".repeat(NAME_MAX))
        );
    }

    #[test]
    fn the_picker_says_when_it_shows_only_the_first_matches() {
        let list: Vec<ProfileInfo> = (0..PICKER_LIMIT + 5)
            .map(|i| ProfileInfo {
                name: format!("Preset {i:03}"),
                filament_type: None,
                filament_id: None,
                path: format!("/{i}.json"),
                is_user_profile: false,
            })
            .collect();
        assert_eq!(matching(&list, "").len(), PICKER_LIMIT);
        assert!(more_than_shown(&list, ""));
        assert!(!more_than_shown(&list, "Preset 00"));
    }

    #[test]
    fn a_connected_printer_without_a_report_is_waiting() {
        let mut v = PrinterView {
            configured: true,
            connection: ConnectionState::Connected,
            ..Default::default()
        };
        assert!(waiting_for_report(&v));
        v.state = Some(PrinterState::default());
        assert!(!waiting_for_report(&v));
        v.state = None;
        v.connection = ConnectionState::Unreachable;
        assert!(!waiting_for_report(&v));
    }
}
