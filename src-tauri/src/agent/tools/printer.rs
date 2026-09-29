//! Read-only printer tools. They never include the printer's IP address, its
//! access code or its pinned certificate fingerprint, and there are no write
//! or control tools.

use serde_json::{json, Value};

use super::{ToolOutput, ToolRegistry, ToolSpec};
use crate::printer::client::ConnectionState;
use crate::printer::service::PrinterView;
use crate::printer::slots::{set_on_printer_steps, SlotStatus};

pub const NO_PRINTER: &str = "No printer configured";
pub const NOT_CONNECTED: &str = "Printer not connected";
pub const WAITING_FOR_REPORT: &str = "Printer connected; waiting for the first status report.";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_printer_status",
            description: "Read-only status of the user's Bambu printer: connection, the current print (state, file, progress, layers, time left), bed and nozzle temperatures, active errors with their text, and model/serial.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolSpec {
            name: "bm_ams_slots",
            description: "Read-only list of every AMS slot and external spool: label (A1-D4, HT1, Ext-L/Ext-R), the filament the printer reports, whether it is a Bambu RFID spool, remaining %, the preset the user assigned in BambuMate, and its status: set (the printer's setting matches the assignment), bambu_spool, needs_setting_on_printer (the printer reports a different filament; the user must still set the preset on the printer, and the steps are included), empty or not_set. preset_has_no_id means BambuMate can't verify the slot because the preset has no filament id.",
            input_schema: json!({"type":"object","properties":{}}),
        },
    ]
}

pub async fn handle(reg: &ToolRegistry, name: &str, _args: &Value) -> Option<ToolOutput> {
    match name {
        "bm_printer_status" => Some(status(&reg.host().printer_view())),
        "bm_ams_slots" => Some(slots(&reg.host().printer_view())),
        _ => None,
    }
}

/// Any state but `Connected` (including the ones that need the user to act:
/// a rejected access code, an untrusted certificate, a wrong serial) reads as
/// "not connected". The states' payloads (fingerprint, presented serial)
/// never reach the agent.
fn unavailable(view: &PrinterView) -> Option<ToolOutput> {
    if !view.configured {
        return Some(ToolOutput::text(NO_PRINTER));
    }
    if view.connection != ConnectionState::Connected {
        return Some(ToolOutput::text(NOT_CONNECTED));
    }
    if view.state.is_none() {
        return Some(ToolOutput::text(WAITING_FOR_REPORT));
    }
    None
}

/// On the H2 series extruder 0 is the right nozzle and 1 the left.
fn nozzle_name(id: u32, dual: bool) -> Value {
    match (dual, id) {
        (true, 0) => json!("right"),
        (true, 1) => json!("left"),
        _ => json!(id),
    }
}

fn status(view: &PrinterView) -> ToolOutput {
    if let Some(out) = unavailable(view) {
        return out;
    }
    let s = view.state.clone().unwrap_or_default();
    let dual = s.nozzles.len() >= 2;
    let printer = view.printer.as_ref();
    ToolOutput::json(&json!({
        "connection": "connected",
        "printer": {
            "model": printer.map(|p| p.model.clone()),
            "serial": printer.map(|p| p.serial.clone()),
            "name": printer.map(|p| p.name.clone()),
            "firmware": printer.and_then(|p| p.firmware.clone()),
        },
        "print": {
            "state": s.gcode_state,
            "file": s.subtask_name,
            "percent": s.mc_percent,
            "layer": s.layer_num,
            "total_layers": s.total_layer_num,
            "remaining_minutes": s.mc_remaining_time,
        },
        "temperatures": {
            "bed": {"current": s.bed_temp, "target": s.bed_target_temp},
            "nozzles": s.nozzles.iter().map(|n| json!({
                "nozzle": nozzle_name(n.id, dual),
                "current": n.temp,
                "target": n.target_temp,
                "diameter": n.diameter,
                "type": n.nozzle_type,
            })).collect::<Vec<_>>(),
            "active_nozzle": s.active_nozzle.map(|id| nozzle_name(id, dual)),
        },
        "errors": view.errors.iter().map(|e| json!({
            "code": e.code,
            "text": e.text,
            "wiki_url": e.wiki_url,
        })).collect::<Vec<_>>(),
    }))
}

fn status_name(status: SlotStatus) -> &'static str {
    match status {
        SlotStatus::Matches => "set",
        SlotStatus::Rfid => "bambu_spool",
        SlotStatus::Different => "needs_setting_on_printer",
        SlotStatus::Empty => "empty",
        SlotStatus::Unassigned => "not_set",
    }
}

fn slots(view: &PrinterView) -> ToolOutput {
    if let Some(out) = unavailable(view) {
        return out;
    }
    let list: Vec<Value> = view
        .slots
        .iter()
        .map(|s| {
            let reported = (!s.tray.empty).then(|| {
                json!({
                    "type": s.tray.tray_type,
                    "brand": s.tray.tray_sub_brands,
                    "color": s.tray.tray_color,
                    "filament_id": s.tray.tray_info_idx,
                })
            });
            let steps = match (&s.status, &s.assigned_preset) {
                (SlotStatus::Different, Some(preset)) => {
                    set_on_printer_steps(&s.label, preset, s.needs_cloud_sync)
                }
                _ => Vec::new(),
            };
            json!({
                "slot": s.label,
                "reported": reported,
                "rfid": s.rfid,
                "remaining_percent": if s.rfid { s.tray.remain } else { None },
                "assigned_preset": s.assigned_preset,
                "status": status_name(s.status),
                // BambuMate can't verify the slot: the assigned preset has no
                // filament id, so the user must confirm by eye.
                "preset_has_no_id": s.preset_has_no_id,
                "steps": steps,
            })
        })
        .collect();
    ToolOutput::json(&json!({ "slots": list }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::history::SlotAssignment;
    use crate::printer::hms::ErrorView;
    use crate::printer::service::PrinterSummary;
    use crate::printer::slots::compute_slots;
    use crate::printer::state::fixtures::{state_after, H2D_FULL};
    use std::sync::Arc;

    fn connected_view() -> PrinterView {
        let state = state_after(&[H2D_FULL]);
        let assignments = vec![SlotAssignment {
            serial: "0948AB000000001".into(),
            ams_id: 0,
            tray_id: 2,
            preset_name: "Acme PETG".into(),
            filament_id: Some("P0000001".into()),
            preset_path: Some("/u/Acme PETG.json".into()),
            assigned_at: "2026-09-28 10:00:00".into(),
        }];
        PrinterView {
            configured: true,
            printer: Some(PrinterSummary {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                name: "Workshop".into(),
                model: "H2D".into(),
                firmware: Some("01.01.01.00".into()),
                ca_verified: true,
            }),
            connection: ConnectionState::Connected,
            slots: compute_slots(&state, &assignments, &|_| true),
            state: Some(state),
            errors: vec![ErrorView {
                kind: "hms".into(),
                code: "0300_0100_0001_0007".into(),
                text: Some("The heatbed temperature is abnormal.".into()),
                wiki_url:
                    "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0100_0001_0007"
                        .into(),
            }],
        }
    }

    async fn call(view: PrinterView, tool: &str) -> String {
        let host = Arc::new(FakeHost::new());
        *host.printer.lock().unwrap() = view;
        let (reg, _rx) = registry_with(host);
        let out = reg.call(tool, json!({})).await;
        assert!(out.ok);
        match &out.content[0] {
            crate::agent::tools::ToolContent::Text(t) => t.clone(),
            other => panic!("unexpected content {other:?}"),
        }
    }

    #[tokio::test]
    async fn both_tools_say_when_no_printer_is_configured() {
        for tool in ["bm_printer_status", "bm_ams_slots"] {
            assert_eq!(call(PrinterView::unconfigured(), tool).await, NO_PRINTER);
        }
    }

    #[tokio::test]
    async fn both_tools_say_when_the_printer_is_not_connected() {
        let mut view = connected_view();
        view.connection = ConnectionState::Unreachable;
        for tool in ["bm_printer_status", "bm_ams_slots"] {
            assert_eq!(call(view.clone(), tool).await, NOT_CONNECTED);
        }
    }

    #[tokio::test]
    async fn both_tools_wait_when_connected_but_no_report_has_arrived() {
        let mut view = connected_view();
        view.state = None;
        for tool in ["bm_printer_status", "bm_ams_slots"] {
            assert_eq!(call(view.clone(), tool).await, WAITING_FOR_REPORT);
        }
    }

    #[tokio::test]
    async fn every_state_but_connected_reads_as_not_connected_and_leaks_nothing() {
        let states = [
            ConnectionState::Disconnected,
            ConnectionState::Connecting,
            ConnectionState::AuthFailed,
            ConnectionState::CertUntrusted {
                fingerprint: "AA:BB:CC:PINNED".into(),
            },
            ConnectionState::WrongSerial {
                presented: "OTHER-SERIAL".into(),
            },
            ConnectionState::Unreachable,
        ];
        for connection in states {
            let mut view = connected_view();
            view.connection = connection;
            for tool in ["bm_printer_status", "bm_ams_slots"] {
                let text = call(view.clone(), tool).await;
                assert_eq!(text, NOT_CONNECTED);
            }
        }
    }

    #[tokio::test]
    async fn status_reports_the_print_temperatures_and_errors_without_the_ip() {
        let text = call(connected_view(), "bm_printer_status").await;
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["print"]["state"], "RUNNING");
        assert_eq!(v["print"]["percent"], 6);
        assert_eq!(v["temperatures"]["nozzles"][0]["nozzle"], "right");
        assert_eq!(v["temperatures"]["nozzles"][1]["nozzle"], "left");
        assert_eq!(v["temperatures"]["active_nozzle"], "right");
        assert_eq!(v["errors"][0]["code"], "0300_0100_0001_0007");
        assert_eq!(v["printer"]["model"], "H2D");
        assert!(
            !text.contains("192.168.1.20"),
            "the IP must not reach the agent"
        );
        assert!(v["printer"].get("ip").is_none());
    }

    #[tokio::test]
    async fn slots_list_status_and_steps() {
        let text = call(connected_view(), "bm_ams_slots").await;
        let v: Value = serde_json::from_str(&text).unwrap();
        let slots = v["slots"].as_array().unwrap();
        assert_eq!(slots.len(), 10);
        assert_eq!(slots[1]["slot"], "A2");
        assert_eq!(slots[1]["status"], "bambu_spool");
        assert_eq!(slots[1]["remaining_percent"], 31);
        assert_eq!(slots[2]["status"], "needs_setting_on_printer");
        assert_eq!(slots[2]["assigned_preset"], "Acme PETG");
        assert_eq!(slots[2]["steps"].as_array().unwrap().len(), 3);
        assert!(slots[2]["remaining_percent"].is_null());
        assert_eq!(slots[2]["preset_has_no_id"], false);
        assert_eq!(slots[7]["status"], "empty");
        assert!(slots[7]["reported"].is_null());
        assert_eq!(slots[8]["slot"], "Ext-L");
        assert_eq!(slots[9]["slot"], "Ext-R");
        assert!(!text.contains("192.168.1.20"));
    }

    #[tokio::test]
    async fn a_slot_whose_preset_has_no_filament_id_says_so() {
        let mut view = connected_view();
        let state = view.state.clone().unwrap();
        let assignments = vec![SlotAssignment {
            serial: "0948AB000000001".into(),
            ams_id: 0,
            tray_id: 2,
            preset_name: "No Id PETG".into(),
            filament_id: None,
            preset_path: None,
            assigned_at: "2026-09-28 10:00:00".into(),
        }];
        view.slots = compute_slots(&state, &assignments, &|_| false);
        let text = call(view, "bm_ams_slots").await;
        let v: Value = serde_json::from_str(&text).unwrap();
        let slots = v["slots"].as_array().unwrap();
        assert_eq!(slots[2]["status"], "needs_setting_on_printer");
        assert_eq!(slots[2]["preset_has_no_id"], true);
        assert_eq!(slots[1]["preset_has_no_id"], false);
    }
}
