//! The Printer page: current print, AMS slots with their status and the
//! steps to set a slot on the printer, and active errors. Read-only.
//!
//! Every string that comes from the printer (file name, preset and printer
//! names, HMS text, a presented serial) is rendered as a plain text node.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, ProfileInfo};
use crate::printer::bridge;
use crate::printer::types::{
    connection_message, format_remaining, format_temp, nozzle_label, row_title, swatch_color,
    ConnectionState, PrinterView, SlotStatus, SlotView,
};
use crate::printer::PrinterShared;

/// Picker rows shown at once; the search narrows the rest.
const PICKER_LIMIT: usize = 60;

#[component]
pub fn PrinterPage() -> impl IntoView {
    let shared = expect_context::<PrinterShared>();
    let view = shared.view;
    // Picks up assignments' cloud-sync state and anything missed while away.
    shared.refresh();
    let picking = RwSignal::new(Option::<SlotView>::None);

    let connected = move || view.with(|v| v.connection == ConnectionState::Connected);

    view! {
        <div class="page printer-page nd">
            <header class="pr-head">
                <h2>"Printer"</h2>
                {move || view.with(|v| v.printer.clone()).map(|p| view! {
                    <span class="pr-ident nd-mono">
                        {format!("{} · {} · {}", if p.name.is_empty() { p.serial.clone() } else { p.name.clone() }, p.model, p.serial)}
                    </span>
                })}
            </header>

            <Show
                when=move || view.with(|v| v.configured)
                fallback=|| view! {
                    <div class="pr-empty">
                        <p>"No printer is set up yet."</p>
                        <a href="/settings#printer" class="pr-setup-link">"Set one up in Settings → Printer"</a>
                    </div>
                }
            >
                {move || {
                    let (connection, ip) = view.with(|v| {
                        (v.connection.clone(), v.printer.as_ref().map(|p| p.ip.clone()).unwrap_or_default())
                    });
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
                    when=move || !view.with(waiting_for_report)
                    fallback=|| view! { <p class="pr-waiting">"Waiting for the first status report…"</p> }
                >
                    <div class="pr-body" class:pr-stale=move || !connected()>
                        <Hero view=view />
                        <section class="pr-section pr-ams">
                            <p class="nd-label">"AMS"</p>
                            {move || ams_rows(&view.get()).into_iter().map(|(ams_id, slots)| {
                                let meta = view.with(|v| unit_meta(v, ams_id));
                                view! {
                                    <div class="pr-ams-row" data-ams=ams_id>
                                        <div class="pr-ams-head">
                                            <span class="pr-ams-title">{row_title(ams_id)}</span>
                                            <span class="pr-ams-meta nd-mono">{meta}</span>
                                        </div>
                                        <div class="pr-slots">
                                            {slots.into_iter().map(|s| view! { <SlotCard item=s picking=picking /> }).collect::<Vec<_>>()}
                                        </div>
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        </section>
                        <Errors view=view />
                    </div>
                </Show>
            </Show>

            {move || picking.get().map(|slot| view! { <SlotPicker item=slot picking=picking /> })}
        </div>
    }
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

/// The preset name a card shows, and an assignment to mention only as
/// history. An RFID spool's own report beats a stale assignment.
fn card_preset(s: &SlotView) -> (String, Option<String>) {
    match (&s.assigned_preset, s.status) {
        (Some(stale), SlotStatus::Rfid) => {
            let reported = SlotView {
                assigned_preset: None,
                ..s.clone()
            };
            (reported.preset_name(), Some(stale.clone()))
        }
        _ => (s.preset_name(), None),
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

#[component]
fn Hero(view: RwSignal<PrinterView>) -> impl IntoView {
    let state = move || view.with(|v| v.state.clone().unwrap_or_default());
    view! {
        <section class="pr-section pr-hero">
            <p class="nd-label">"Current print"</p>
            <div class="pr-hero-top">
                <span class="pr-hero-state nd-mono">{move || state().gcode_state.unwrap_or_else(|| "—".into())}</span>
                <span class="pr-hero-percent">{move || state().mc_percent.map(|p| format!("{p}%")).unwrap_or_else(|| "—".into())}</span>
            </div>
            <div class="pr-progress">
                <div class="pr-progress-fill" style:width=move || format!("{}%", state().mc_percent.unwrap_or(0).min(100))></div>
            </div>
            <dl class="pr-facts">
                <div><dt>"Layer"</dt><dd class="pr-layer">{move || {
                    let s = state();
                    match (s.layer_num, s.total_layer_num) {
                        (Some(l), Some(t)) if t > 0 => format!("{l} / {t}"),
                        _ => "—".into(),
                    }
                }}</dd></div>
                <div><dt>"Remaining"</dt><dd class="pr-remaining">{move || state().mc_remaining_time.map(format_remaining).unwrap_or_else(|| "—".into())}</dd></div>
                <div class="pr-file"><dt>"File"</dt><dd>{move || state().subtask_name.filter(|f| !f.is_empty()).unwrap_or_else(|| "—".into())}</dd></div>
            </dl>
            <div class="pr-temps">
                {move || {
                    let s = state();
                    let count = s.nozzles.len();
                    s.nozzles.iter().map(|n| {
                        let active = count >= 2 && s.active_nozzle == Some(n.id);
                        view! {
                            <div class="pr-temp" class:pr-temp-active=active>
                                <span class="nd-label">{nozzle_label(n.id, count)}</span>
                                <span class="nd-mono">{format_temp(n.temp, n.target_temp)}</span>
                                <span class="pr-temp-note">{n.diameter.map(|d| format!("{d} mm")).unwrap_or_default()}</span>
                            </div>
                        }
                    }).collect::<Vec<_>>()
                }}
                <div class="pr-temp">
                    <span class="nd-label">"Bed"</span>
                    <span class="nd-mono">{move || { let s = state(); format_temp(s.bed_temp, s.bed_target_temp) }}</span>
                </div>
            </div>
        </section>
    }
}

#[component]
fn SlotCard(item: SlotView, picking: RwSignal<Option<SlotView>>) -> impl IntoView {
    let slot = item;
    let status = slot.status;
    let color = swatch_color(&slot.tray.tray_color);
    let material = if slot.tray.empty {
        "—".to_string()
    } else {
        slot.tray.tray_type.clone()
    };
    let (preset, previous) = card_preset(&slot);
    let remain = slot.rfid.then_some(slot.tray.remain).flatten();
    let guide = match slot_guide(&slot) {
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
    };
    let open = slot.clone();
    view! {
        <div class="pr-slot" data-status=status.key() data-label=slot.label.clone()>
            <button class="pr-slot-main" on:click=move |_| picking.set(Some(open.clone()))
                title="Choose the preset loaded here">
                <span class="pr-swatch" style:background=color></span>
                <span class="pr-slot-label nd-mono">{slot.label.clone()}</span>
                <span class="pr-material">{material}</span>
                <span class="pr-preset">{preset}</span>
                {previous.map(|p| view! { <span class="pr-prev">{format!("Previously assigned: {p}")}</span> })}
                <span class="pr-slot-foot">
                    {remain.map(|r| view! { <span class="pr-remain nd-mono">{format!("{r}%")}</span> })}
                    {slot.rfid.then(|| view! { <span class="pr-rfid nd-label">"RFID"</span> })}
                    <span class="pr-status">{status.badge()}</span>
                </span>
            </button>
            {guide}
        </div>
    }
}

#[component]
fn Errors(view: RwSignal<PrinterView>) -> impl IntoView {
    view! {
        <section class="pr-section pr-errors">
            <p class="nd-label">"Errors"</p>
            <Show
                when=move || view.with(|v| !v.errors.is_empty())
                fallback=|| view! { <p class="pr-quiet">"No active errors."</p> }
            >
                <ul class="pr-error-list">
                    {move || view.get().errors.into_iter().map(|e| view! {
                        <li class="pr-error">
                            <span class="pr-error-code nd-mono">{e.code.clone()}</span>
                            {match e.text.clone() {
                                Some(t) => view! { <span class="pr-error-text">{t}</span> }.into_any(),
                                None => view! {
                                    <a class="pr-error-link" href=e.wiki_url.clone() target="_blank" rel="noopener">
                                        "Look up this code on the Bambu wiki"
                                    </a>
                                }.into_any(),
                            }}
                        </li>
                    }).collect::<Vec<_>>()}
                </ul>
            </Show>
        </section>
    }
}

/// Filters presets by a case-insensitive search.
fn matching(list: &[ProfileInfo], query: &str) -> Vec<ProfileInfo> {
    let q = query.trim().to_lowercase();
    list.iter()
        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
        .take(PICKER_LIMIT)
        .cloned()
        .collect()
}

#[component]
fn SlotPicker(item: SlotView, picking: RwSignal<Option<SlotView>>) -> impl IntoView {
    let slot = item;
    let shared = expect_context::<PrinterShared>();
    let user = RwSignal::new(Vec::<ProfileInfo>::new());
    let system = RwSignal::new(Vec::<ProfileInfo>::new());
    let query = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let busy = RwSignal::new(false);
    // The picker or the whole page can go away while these are in flight
    // (Cancel, or the agent navigating), so every write after an await is a
    // `try_set`, which does nothing once the signal is disposed.
    spawn_local(async move {
        if let Ok(list) = commands::list_profiles().await {
            user.try_set(list);
        }
        if let Ok(list) = commands::list_system_profiles().await {
            system.try_set(list);
        }
    });
    let (ams_id, tray_id) = (slot.ams_id, slot.tray_id);
    let settle = move |result: Result<PrinterView, String>| match result {
        Ok(v) => {
            shared.view.try_set(v);
            picking.try_set(None);
        }
        Err(e) => {
            busy.try_set(false);
            error.try_set(Some(e));
        }
    };
    let start = move || {
        busy.set(true);
        error.set(None);
    };
    let choose = move |path: String| {
        start();
        spawn_local(async move { settle(bridge::assign_slot(ams_id, tray_id, &path).await) });
    };
    let clear = move |_| {
        start();
        spawn_local(async move { settle(bridge::clear_slot(ams_id, tray_id).await) });
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
    let has_assignment = slot.assigned_preset.is_some();
    view! {
        <div class="pr-picker-backdrop" on:click=move |_| picking.set(None)>
            <div class="pr-picker" role="dialog" aria-label="Choose a preset" on:click=|e| e.stop_propagation()>
                <p class="nd-label">{format!("Slot {}", slot.label)}</p>
                <h3>"Which preset is loaded here?"</h3>
                <input class="pr-picker-search" type="search" placeholder="Search presets"
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev)) />
                <div class="pr-picker-results">
                    {move || group("Your presets", matching(&user.get(), &query.get()))}
                    {move || group("Bambu presets", matching(&system.get(), &query.get()))}
                </div>
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
    use crate::printer::types::{AmsUnit, PrinterState, Tray};

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
            ("PLA Basic".to_string(), Some("Acme PLA".to_string()))
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
