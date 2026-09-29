//! Printer state built from `device/<serial>/report` messages.
//!
//! Pushes are merged as raw JSON first (`ReportMerger`), then read into the
//! typed `PrinterState`. Every field is optional and unknown fields are
//! ignored, so models other than the H2 series parse without tuning.
//!
//! Field names follow OpenBambuAPI `mqtt.md` and ha-bambulab `pybambu/models.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A push without a `msg` marker counts as full when it has at least this
/// many top-level fields. Real full pushes have 60-100; deltas a handful.
const FULL_PUSH_MIN_FIELDS: usize = 20;

/// One message from the report topic.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    /// `print.push_status`: a full push replaces the state, a delta merges.
    Status {
        print: Map<String, Value>,
        full: bool,
    },
    /// `info.get_version`: the model name and firmware version.
    Version {
        model: Option<String>,
        firmware: Option<String>,
    },
    /// Any other message (command acknowledgements and the like).
    Other,
}

/// Parses one report payload. An error means the message is malformed; the
/// caller logs it at debug level and skips it.
pub fn parse_report(payload: &[u8]) -> Result<Report, String> {
    let value: Value = serde_json::from_slice(payload).map_err(|e| format!("not JSON: {e}"))?;
    let Value::Object(top) = value else {
        return Err("report is not a JSON object".into());
    };
    if let Some(print) = top.get("print") {
        let Value::Object(print) = print else {
            return Err("`print` is not an object".into());
        };
        let command = print.get("command").and_then(Value::as_str);
        if command.is_some() && command != Some("push_status") {
            return Ok(Report::Other);
        }
        let full = match print.get("msg").and_then(num_u64) {
            Some(0) => true,
            Some(_) => false,
            None => print.len() >= FULL_PUSH_MIN_FIELDS,
        };
        return Ok(Report::Status {
            print: print.clone(),
            full,
        });
    }
    if let Some(Value::Object(info)) = top.get("info") {
        if info.get("command").and_then(Value::as_str) == Some("get_version") {
            let modules = info
                .get("module")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let printer = modules.iter().find(|m| {
                m.get("product_name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.starts_with("Bambu Lab "))
            });
            let ota = modules
                .iter()
                .find(|m| m.get("name").and_then(Value::as_str) == Some("ota"));
            let model = printer
                .and_then(|m| m.get("product_name"))
                .and_then(Value::as_str)
                .map(|n| n.trim_start_matches("Bambu Lab ").to_string());
            let firmware = ota
                .and_then(|m| m.get("sw_ver"))
                .and_then(Value::as_str)
                .map(str::to_string);
            return Ok(Report::Version { model, firmware });
        }
    }
    Ok(Report::Other)
}

/// Keeps the merged raw `print` object and derives `PrinterState` from it.
#[derive(Debug, Default, Clone)]
pub struct ReportMerger {
    raw: Map<String, Value>,
    has_full: bool,
}

impl ReportMerger {
    /// Applies one push and returns the resulting state.
    pub fn apply(&mut self, print: &Map<String, Value>, full: bool) -> PrinterState {
        if full {
            self.raw = print.clone();
            self.has_full = true;
        } else {
            merge_into(&mut self.raw, print);
        }
        PrinterState::from_print(&self.raw)
    }

    /// True once a full push has arrived.
    pub fn has_full(&self) -> bool {
        self.has_full
    }
}

/// Merges `delta` into `base` field by field. Objects merge recursively.
/// Arrays of objects that all carry an `id` merge element by element on
/// that id; any other array replaces. An element with only `id` (and
/// `state`) replaces its old element: that is how an emptied tray is sent.
pub fn merge_into(base: &mut Map<String, Value>, delta: &Map<String, Value>) {
    for (key, value) in delta {
        merge_value(base.entry(key.clone()).or_insert(Value::Null), value);
    }
}

fn merge_value(base: &mut Value, delta: &Value) {
    match (base, delta) {
        (Value::Object(b), Value::Object(d)) => merge_into(b, d),
        (Value::Array(b), Value::Array(d)) if is_id_list(b) && is_id_list(d) => {
            for item in d {
                let id = id_key(item);
                match b.iter_mut().find(|old| id_key(old) == id) {
                    Some(old) if is_id_only(item) => *old = item.clone(),
                    Some(old) => merge_value(old, item),
                    None => b.push(item.clone()),
                }
            }
        }
        (b, d) => *b = d.clone(),
    }
}

fn is_id_list(items: &[Value]) -> bool {
    !items.is_empty() && items.iter().all(|v| v.get("id").is_some())
}

fn id_key(item: &Value) -> Option<String> {
    match item.get("id")? {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn is_id_only(item: &Value) -> bool {
    item.as_object()
        .is_some_and(|o| o.keys().all(|k| k == "id" || k == "state"))
}

/// The live printer state shown on the Printer page and to the agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrinterState {
    /// `gcode_state`: IDLE, PREPARE, RUNNING, PAUSE, FINISH, FAILED, ...
    pub gcode_state: Option<String>,
    /// `subtask_name`: the file being printed.
    pub subtask_name: Option<String>,
    pub mc_percent: Option<u32>,
    /// `mc_remaining_time`, in minutes.
    pub mc_remaining_time: Option<u32>,
    pub layer_num: Option<u32>,
    pub total_layer_num: Option<u32>,
    pub bed_temp: Option<f64>,
    pub bed_target_temp: Option<f64>,
    /// One entry per extruder: two on the H2D/H2C, one elsewhere.
    pub nozzles: Vec<Nozzle>,
    /// Index of the active extruder (H2 dual-nozzle only).
    pub active_nozzle: Option<u32>,
    pub ams_units: Vec<AmsUnit>,
    /// External spool holders: `vir_slot` (ids 254/255) on the H2 series,
    /// else the single `vt_tray`.
    pub external_spools: Vec<Tray>,
    /// `ams.tray_now` (255 = nothing loaded is `None`).
    pub tray_now: Option<u32>,
    pub hms: Vec<HmsCode>,
    /// `print_error`, when non-zero.
    pub print_error: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Nozzle {
    /// Extruder id. On the H2 series 0 is the right nozzle, 1 the left.
    pub id: u32,
    pub temp: Option<f64>,
    pub target_temp: Option<f64>,
    pub diameter: Option<f64>,
    /// `HS01` style on the H2 series, `stainless_steel` style elsewhere.
    pub nozzle_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AmsUnit {
    /// 0-3 for AMS / AMS 2 Pro, 128+ for AMS HT.
    pub id: u32,
    /// `humidity`: Bambu's 1-5 level.
    pub humidity_level: Option<u32>,
    /// `humidity_raw`: percent, on newer firmware.
    pub humidity_pct: Option<u32>,
    pub temp: Option<f64>,
    pub trays: Vec<Tray>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tray {
    pub id: u32,
    /// No filament reported in this slot.
    pub empty: bool,
    pub tray_type: String,
    /// `RRGGBBAA`.
    pub tray_color: String,
    /// The filament id of the preset set on this slot (`GFA00`, `P1234567`).
    pub tray_info_idx: String,
    pub tray_sub_brands: String,
    pub nozzle_temp_min: Option<u32>,
    pub nozzle_temp_max: Option<u32>,
    /// Remaining filament in percent. Only meaningful for RFID spools.
    pub remain: Option<u32>,
    pub tag_uid: String,
    pub tray_uuid: String,
}

impl Tray {
    /// A Bambu RFID spool: a tag uid or tray uuid that is not all zeros.
    pub fn has_rfid(&self) -> bool {
        is_set_id(&self.tag_uid) || is_set_id(&self.tray_uuid)
    }
}

fn is_set_id(s: &str) -> bool {
    s.chars().any(|c| c != '0')
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HmsCode {
    pub attr: u32,
    pub code: u32,
}

impl HmsCode {
    /// `0300_0100_0001_0007`, the form Bambu's wiki and apps show.
    pub fn display(&self) -> String {
        format!(
            "{:04X}_{:04X}_{:04X}_{:04X}",
            self.attr >> 16,
            self.attr & 0xFFFF,
            self.code >> 16,
            self.code & 0xFFFF
        )
    }
}

impl PrinterState {
    /// Reads the typed state out of a merged `print` object.
    pub fn from_print(p: &Map<String, Value>) -> Self {
        let device = p.get("device");
        let (bed_temp, bed_target_temp) = match device
            .and_then(|d| d.pointer("/bed/info/temp"))
            .and_then(num_u64)
        {
            Some(packed) => unpack_temp(packed),
            None => (
                p.get("bed_temper").and_then(num_f64),
                p.get("bed_target_temper").and_then(num_f64),
            ),
        };
        let extruder = device.and_then(|d| d.get("extruder"));
        let active_nozzle = extruder
            .and_then(|e| e.get("state"))
            .and_then(num_u64)
            .map(|s| ((s >> 4) & 0xF) as u32);
        let ams = p.get("ams");
        PrinterState {
            gcode_state: str_field(p, "gcode_state"),
            subtask_name: str_field(p, "subtask_name"),
            mc_percent: p.get("mc_percent").and_then(num_u32),
            mc_remaining_time: p.get("mc_remaining_time").and_then(num_u32),
            layer_num: p.get("layer_num").and_then(num_u32),
            total_layer_num: p.get("total_layer_num").and_then(num_u32),
            bed_temp,
            bed_target_temp,
            nozzles: nozzles(p, device),
            active_nozzle,
            ams_units: ams
                .and_then(|a| a.get("ams"))
                .and_then(Value::as_array)
                .map(|units| units.iter().filter_map(ams_unit).collect::<Vec<_>>())
                .map(sorted_units)
                .unwrap_or_default(),
            external_spools: external_spools(p),
            tray_now: ams
                .and_then(|a| a.get("tray_now"))
                .and_then(num_u32)
                .filter(|n| *n != 255),
            hms: p
                .get("hms")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|h| {
                            Some(HmsCode {
                                attr: h.get("attr").and_then(num_u32)?,
                                code: h.get("code").and_then(num_u32)?,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            print_error: p.get("print_error").and_then(num_u32).filter(|c| *c != 0),
        }
    }
}

fn nozzles(p: &Map<String, Value>, device: Option<&Value>) -> Vec<Nozzle> {
    let nozzle_info = device
        .and_then(|d| d.pointer("/nozzle/info"))
        .and_then(Value::as_array);
    let extruders = device
        .and_then(|d| d.pointer("/extruder/info"))
        .and_then(Value::as_array);
    if let Some(extruders) = extruders {
        let mut out: Vec<Nozzle> = extruders
            .iter()
            .filter_map(|e| {
                let id = e.get("id").and_then(num_u32)?;
                let (temp, target_temp) = e
                    .get("temp")
                    .and_then(num_u64)
                    .map(unpack_temp)
                    .unwrap_or((None, None));
                let info = nozzle_info.and_then(|list| {
                    list.iter()
                        .find(|n| n.get("id").and_then(num_u32) == Some(id))
                });
                Some(Nozzle {
                    id,
                    temp,
                    target_temp,
                    diameter: info.and_then(|n| n.get("diameter")).and_then(num_f64),
                    nozzle_type: info
                        .and_then(|n| n.get("type"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect();
        out.sort_by_key(|n| n.id);
        return out;
    }
    let any = ["nozzle_temper", "nozzle_target_temper", "nozzle_diameter"]
        .iter()
        .any(|k| p.contains_key(*k));
    if !any {
        return Vec::new();
    }
    vec![Nozzle {
        id: 0,
        temp: p.get("nozzle_temper").and_then(num_f64),
        target_temp: p.get("nozzle_target_temper").and_then(num_f64),
        diameter: p.get("nozzle_diameter").and_then(num_f64),
        nozzle_type: str_field(p, "nozzle_type"),
    }]
}

fn ams_unit(v: &Value) -> Option<AmsUnit> {
    let id = v.get("id").and_then(num_u32)?;
    let mut trays: Vec<Tray> = v
        .get("tray")
        .and_then(Value::as_array)
        .map(|t| t.iter().filter_map(tray).collect())
        .unwrap_or_default();
    trays.sort_by_key(|t| t.id);
    Some(AmsUnit {
        id,
        humidity_level: v.get("humidity").and_then(num_u32),
        humidity_pct: v.get("humidity_raw").and_then(num_u32),
        temp: v.get("temp").and_then(num_f64),
        trays,
    })
}

fn sorted_units(mut units: Vec<AmsUnit>) -> Vec<AmsUnit> {
    units.sort_by_key(|u| u.id);
    units
}

fn tray(v: &Value) -> Option<Tray> {
    let id = v.get("id").and_then(num_u32)?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let tray_type = s("tray_type");
    let tray_info_idx = s("tray_info_idx");
    Some(Tray {
        id,
        empty: tray_type.is_empty() && tray_info_idx.is_empty(),
        tray_color: s("tray_color"),
        tray_sub_brands: s("tray_sub_brands"),
        nozzle_temp_min: v
            .get("nozzle_temp_min")
            .and_then(num_u32)
            .filter(|t| *t > 0),
        nozzle_temp_max: v
            .get("nozzle_temp_max")
            .and_then(num_u32)
            .filter(|t| *t > 0),
        remain: v
            .get("remain")
            .and_then(num_f64)
            .filter(|r| *r >= 0.0)
            .map(|r| r as u32),
        tag_uid: s("tag_uid"),
        tray_uuid: s("tray_uuid"),
        tray_type,
        tray_info_idx,
    })
}

fn external_spools(p: &Map<String, Value>) -> Vec<Tray> {
    if let Some(slots) = p.get("vir_slot").and_then(Value::as_array) {
        let mut out: Vec<Tray> = slots.iter().filter_map(tray).collect();
        out.sort_by_key(|t| t.id);
        return out;
    }
    p.get("vt_tray").and_then(tray).into_iter().collect()
}

/// H2 packs temperatures as `target << 16 | current`.
fn unpack_temp(packed: u64) -> (Option<f64>, Option<f64>) {
    (
        Some((packed & 0xFFFF) as f64),
        Some(((packed >> 16) & 0xFFFF) as f64),
    )
}

fn str_field(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Numbers arrive as JSON numbers or as strings ("27.0", "190").
fn num_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite())
}

fn num_u64(v: &Value) -> Option<u64> {
    num_f64(v).filter(|f| *f >= 0.0).map(|f| f as u64)
}

fn num_u32(v: &Value) -> Option<u32> {
    num_u64(v).and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Report fixtures. `h2d_full` is trimmed from ha-bambulab
    //! `pybambu/mock_data/MOCK-H2D.json` (commit 0e027ff, 2026-09-21), with
    //! one slot emptied, one non-RFID spool and one loaded external spool
    //! added. `p1p_full` follows the OpenBambuAPI `mqtt.md` pushall example.
    //! Replace with real captures from the user's H2 during acceptance.
    pub const H2D_FULL: &str = include_str!("testdata/h2d_full.json");
    pub const H2D_DELTA_PROGRESS: &str = include_str!("testdata/h2d_delta_progress.json");
    pub const H2D_DELTA_AMS: &str = include_str!("testdata/h2d_delta_ams.json");
    pub const P1P_FULL: &str = include_str!("testdata/p1p_full.json");
    pub const P1P_DELTA: &str = include_str!("testdata/p1p_delta.json");
    pub const GET_VERSION_H2D: &str = include_str!("testdata/get_version_h2d.json");

    use super::{parse_report, PrinterState, Report, ReportMerger};

    /// Applies each fixture in order and returns the final state.
    pub fn state_after(fixtures: &[&str]) -> PrinterState {
        let mut merger = ReportMerger::default();
        let mut state = PrinterState::default();
        for f in fixtures {
            if let Report::Status { print, full } = parse_report(f.as_bytes()).unwrap() {
                state = merger.apply(&print, full);
            }
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn full_h2d_push_parses_print_progress_and_temperatures() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.gcode_state.as_deref(), Some("RUNNING"));
        assert_eq!(
            s.subtask_name.as_deref(),
            Some("T-pose - slim H2D dual AMS riser")
        );
        assert_eq!(s.mc_percent, Some(6));
        assert_eq!(s.mc_remaining_time, Some(549));
        assert_eq!((s.layer_num, s.total_layer_num), (Some(1), Some(200)));
        assert_eq!((s.bed_temp, s.bed_target_temp), (Some(70.0), Some(70.0)));
        assert!(s.print_error.is_none());
    }

    #[test]
    fn dual_nozzles_come_from_the_device_block() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.nozzles.len(), 2);
        assert_eq!(s.nozzles[0].id, 0);
        assert_eq!(s.nozzles[0].temp, Some(245.0));
        assert_eq!(s.nozzles[0].target_temp, Some(245.0));
        assert_eq!(s.nozzles[1].temp, Some(47.0));
        assert_eq!(s.nozzles[1].target_temp, Some(0.0));
        assert_eq!(s.nozzles[1].diameter, Some(0.4));
        assert_eq!(s.nozzles[1].nozzle_type.as_deref(), Some("HS01"));
        assert_eq!(s.active_nozzle, Some(0));
    }

    #[test]
    fn multiple_ams_units_parse_with_every_tray() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.ams_units.len(), 2);
        let b = &s.ams_units[1];
        assert_eq!(b.id, 1);
        assert_eq!(b.humidity_level, Some(5));
        assert_eq!(b.humidity_pct, Some(18));
        assert_eq!(b.temp, Some(29.7));
        assert_eq!(b.trays.len(), 4);
        assert_eq!(b.trays[2].tray_type, "PETG-CF");
        assert_eq!(b.trays[2].nozzle_temp_min, Some(240));
        assert!(b.trays[3].empty, "an id-only tray is an empty slot");
        assert_eq!(s.tray_now, Some(3));
    }

    #[test]
    fn rfid_trays_are_told_apart_from_third_party_spools() {
        let s = state_after(&[H2D_FULL]);
        let a = &s.ams_units[0];
        assert!(a.trays[1].has_rfid());
        assert_eq!(a.trays[1].remain, Some(31));
        assert!(!a.trays[2].has_rfid(), "all-zero tag ids are not RFID");
        assert_eq!(a.trays[2].tray_info_idx, "GFG99");
    }

    #[test]
    fn h2_external_spools_come_from_vir_slot() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.external_spools.len(), 2);
        assert_eq!(s.external_spools[0].id, 254);
        assert_eq!(s.external_spools[0].tray_info_idx, "GFA01");
        assert!(!s.external_spools[0].empty);
        assert_eq!(s.external_spools[1].id, 255);
        assert!(s.external_spools[1].empty);
    }

    #[test]
    fn hms_codes_decode_to_the_wiki_form() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.hms.len(), 1);
        assert_eq!(s.hms[0].display(), "0300_0100_0001_0007");
    }

    #[test]
    fn delta_merge_keeps_unmentioned_fields() {
        let s = state_after(&[H2D_FULL, H2D_DELTA_PROGRESS]);
        assert_eq!(s.mc_percent, Some(7));
        assert_eq!(s.layer_num, Some(2));
        assert_eq!(s.nozzles[0].temp, Some(244.0));
        // Not in the delta: unchanged.
        assert_eq!(s.nozzles[1].temp, Some(47.0));
        assert_eq!(s.nozzles[1].nozzle_type.as_deref(), Some("HS01"));
        assert_eq!(
            s.subtask_name.as_deref(),
            Some("T-pose - slim H2D dual AMS riser")
        );
        assert_eq!(s.ams_units.len(), 2);
        assert_eq!(s.hms.len(), 1);
    }

    #[test]
    fn delta_for_one_tray_changes_only_that_tray() {
        let s = state_after(&[H2D_FULL, H2D_DELTA_AMS]);
        let b = &s.ams_units[1];
        assert_eq!(b.trays[1].tray_info_idx, "P1234567");
        assert_eq!(b.trays[0].tray_info_idx, "GFA00");
        assert_eq!(b.humidity_pct, Some(18));
        assert_eq!(s.ams_units[0].trays.len(), 4);
    }

    #[test]
    fn an_id_only_tray_in_a_delta_empties_the_slot() {
        let emptied = r#"{"print":{"command":"push_status","msg":1,
            "ams":{"ams":[{"id":"0","tray":[{"id":"1","state":0}]}]}}}"#;
        let s = state_after(&[H2D_FULL, emptied]);
        let tray = &s.ams_units[0].trays[1];
        assert!(tray.empty);
        assert_eq!(tray.tray_info_idx, "");
        assert!(!tray.has_rfid());
    }

    #[test]
    fn a_full_push_replaces_the_state() {
        let one_unit = r#"{"print":{"command":"push_status","msg":0,
            "ams":{"ams":[{"id":"0","tray":[{"id":"0"}]}]}}}"#;
        let s = state_after(&[H2D_FULL, one_unit]);
        assert_eq!(s.ams_units.len(), 1);
        assert!(s.gcode_state.is_none());
    }

    #[test]
    fn p1_style_delta_merges_into_a_single_nozzle_state() {
        let s = state_after(&[P1P_FULL, P1P_DELTA]);
        assert_eq!(s.gcode_state.as_deref(), Some("RUNNING"));
        assert_eq!(s.mc_percent, Some(42));
        assert_eq!(s.nozzles.len(), 1);
        assert_eq!(s.nozzles[0].temp, Some(219.5));
        assert_eq!(s.nozzles[0].diameter, Some(0.4));
        assert_eq!(s.nozzles[0].nozzle_type.as_deref(), Some("stainless_steel"));
        assert_eq!(s.active_nozzle, None);
        assert_eq!(s.bed_temp, Some(25.0));
        assert_eq!(s.tray_now, None, "255 means nothing loaded");
        assert!(s.ams_units[0].trays[0].empty);
        assert_eq!(s.external_spools.len(), 1);
        assert_eq!(s.external_spools[0].id, 254);
        assert_eq!(s.external_spools[0].tray_type, "ABS");
    }

    #[test]
    fn get_version_reports_the_model_name() {
        assert_eq!(
            parse_report(GET_VERSION_H2D.as_bytes()).unwrap(),
            Report::Version {
                model: Some("H2D".into()),
                firmware: Some("01.01.01.00".into())
            }
        );
    }

    #[test]
    fn malformed_messages_are_errors_not_panics() {
        assert!(parse_report(b"not json").is_err());
        assert!(parse_report(b"[1,2,3]").is_err());
        assert!(parse_report(br#"{"print":"nope"}"#).is_err());
        assert_eq!(
            parse_report(br#"{"print":{"command":"gcode_line","result":"success"}}"#).unwrap(),
            Report::Other
        );
    }

    #[test]
    fn wrong_typed_fields_are_ignored() {
        let odd = r#"{"print":{"command":"push_status","msg":0,
            "mc_percent":"abc","layer_num":-3,"hms":[{"attr":"x"}],"ams":{"ams":"?"}}}"#;
        let s = state_after(&[odd]);
        assert_eq!(s.mc_percent, None);
        assert_eq!(s.layer_num, None);
        assert!(s.hms.is_empty());
        assert!(s.ams_units.is_empty());
    }

    #[test]
    fn a_push_without_msg_is_full_only_when_it_is_large() {
        let small = br#"{"print":{"command":"push_status","mc_percent":1}}"#;
        assert!(matches!(
            parse_report(small).unwrap(),
            Report::Status { full: false, .. }
        ));
        assert!(matches!(
            parse_report(H2D_FULL.as_bytes()).unwrap(),
            Report::Status { full: true, .. }
        ));
    }
}
