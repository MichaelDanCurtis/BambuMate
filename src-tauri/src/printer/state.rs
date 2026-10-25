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
            None => looks_full(print),
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

/// Without a `msg` marker a push is full only when it is large and carries
/// the AMS block plus a device or external-spool block, as real full pushes
/// do. Anything else merges: replacing state on a mistaken delta would lose
/// everything, while merging a real full push only loses removals.
fn looks_full(print: &Map<String, Value>) -> bool {
    print.len() >= FULL_PUSH_MIN_FIELDS
        && print.contains_key("ams")
        && ["device", "vt_tray", "vir_slot"]
            .iter()
            .any(|k| print.contains_key(*k))
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
        prune_absent_ams_units(&mut self.raw);
        PrinterState::from_print(&self.raw)
    }

    /// True once a full push has arrived.
    pub fn has_full(&self) -> bool {
        self.has_full
    }
}

/// Merges `delta` into `base` field by field. Objects merge recursively.
/// Arrays of objects that all carry an `id` merge element by element on
/// that id; any other array replaces, except that an empty array never
/// empties an id list (Bambu does not send empty lists to mean removal;
/// vanished AMS units are pruned through `ams_exist_bits`). A tray element
/// (`ams.ams[].tray[]`, `vir_slot[]`, `vt_tray`) whose only key is `id`
/// replaces its old element: that is how an emptied slot is sent. Every
/// other partial element, such as `{"id":"1","state":10}`, merges.
pub fn merge_into(base: &mut Map<String, Value>, delta: &Map<String, Value>) {
    merge_object(base, delta, "");
}

fn merge_object(base: &mut Map<String, Value>, delta: &Map<String, Value>, path: &str) {
    for (key, value) in delta {
        let child = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        merge_value(
            base.entry(key.clone()).or_insert(Value::Null),
            value,
            &child,
        );
    }
}

/// Paths whose `id`-only elements mean "slot emptied".
fn is_tray_path(path: &str) -> bool {
    matches!(path, "ams.ams.tray" | "vir_slot" | "vt_tray")
}

fn merge_value(base: &mut Value, delta: &Value, path: &str) {
    match (base, delta) {
        (Value::Object(b), Value::Object(d)) => {
            if is_tray_path(path) && is_id_only(delta) {
                *b = d.clone();
            } else {
                merge_object(b, d, path);
            }
        }
        (Value::Array(b), Value::Array(d)) if d.is_empty() && is_id_list(b) => {}
        (Value::Array(b), Value::Array(d)) if is_id_list(b) && is_id_list(d) => {
            for item in d {
                let id = id_key(item);
                match b.iter_mut().find(|old| id_key(old) == id) {
                    Some(old) if is_tray_path(path) && is_id_only(item) => *old = item.clone(),
                    Some(old) => merge_value(old, item, path),
                    None => b.push(item.clone()),
                }
            }
        }
        // A malformed number keeps the value already merged.
        (_, d) if is_numeric_path(path) && !num_f64(d).is_some_and(|f| f >= 0.0) => {}
        (b, d) => *b = d.clone(),
    }
}

/// Paths `from_print` reads as numbers.
fn is_numeric_path(path: &str) -> bool {
    matches!(
        path,
        "mc_percent"
            | "mc_remaining_time"
            | "layer_num"
            | "total_layer_num"
            | "plate_idx"
            | "print_error"
            | "bed_temper"
            | "bed_target_temper"
            | "nozzle_temper"
            | "nozzle_target_temper"
            | "nozzle_diameter"
            | "ams.tray_now"
            | "device.extruder.state"
            | "device.extruder.info.snow"
    )
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

/// An element with exactly one key, `id`.
fn is_id_only(item: &Value) -> bool {
    item.as_object()
        .is_some_and(|o| o.len() == 1 && o.contains_key("id"))
}

/// `ams.ams_exist_bits`: hex bitmask, bit n set when AMS unit id n exists.
fn ams_exist_mask(p: &Map<String, Value>) -> Option<u64> {
    let bits = p.get("ams")?.get("ams_exist_bits")?.as_str()?.trim();
    let bits = bits.strip_prefix("0x").unwrap_or(bits);
    u64::from_str_radix(bits, 16).ok()
}

/// Whether unit `id` survives the mask. Ids outside the mask's range (AMS HT
/// uses 128+) and an absent mask keep the unit.
fn ams_unit_exists(mask: Option<u64>, id: u32) -> bool {
    match mask {
        Some(m) if id < 64 => (m >> id) & 1 == 1,
        _ => true,
    }
}

/// Drops units the printer no longer reports from the raw merged object so a
/// later delta cannot bring them back.
fn prune_absent_ams_units(raw: &mut Map<String, Value>) {
    let mask = ams_exist_mask(raw);
    if mask.is_none() {
        return;
    }
    if let Some(Value::Array(units)) = raw.get_mut("ams").and_then(|a| a.get_mut("ams")) {
        units.retain(|u| {
            u.get("id")
                .and_then(num_u32)
                .is_none_or(|id| ams_unit_exists(mask, id))
        });
    }
}

/// The live printer state shown on the Printer page and to the agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrinterState {
    /// `gcode_state`: IDLE, PREPARE, RUNNING, PAUSE, FINISH, FAILED, ...
    pub gcode_state: Option<String>,
    /// `subtask_name`: the file being printed.
    pub subtask_name: Option<String>,
    /// `task_id`: the cloud task, `"0"` for local and SD jobs.
    pub task_id: Option<String>,
    /// `subtask_id`, `"0"` for local and SD jobs.
    pub subtask_id: Option<String>,
    /// `job_id`, a number or a string on the wire; kept as a string.
    pub job_id: Option<String>,
    /// `gcode_file`: the path or name of the file being printed.
    pub gcode_file: Option<String>,
    /// `plate_idx`, when the printer reports it (0 means it doesn't).
    pub plate_idx: Option<u32>,
    /// `print_type`: `"system"` for the printer's own calibration jobs.
    pub print_type: Option<String>,
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
#[serde(default)]
pub struct Nozzle {
    /// Extruder id. On the H2 series 0 is the right nozzle, 1 the left.
    pub id: u32,
    pub temp: Option<f64>,
    pub target_temp: Option<f64>,
    pub diameter: Option<f64>,
    /// `HS01` style on the H2 series, `stainless_steel` style elsewhere.
    pub nozzle_type: Option<String>,
    /// `snow` on the H2 series: the tray feeding this extruder, packed as
    /// `(ams_id << 8) | slot`, 0xFFFF for none. See [`decode_extruder_tray`].
    pub tray_now: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
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
#[serde(default)]
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

/// A tray in the numbering `slot_assignments` and the Printer page use:
/// AMS units 0–3 and AMS HT units 128+ with their tray, and the external
/// spools as `ams_id` 255 with tray 254 or 255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrayRef {
    pub ams_id: u32,
    pub tray_id: u32,
}

/// `ams_id` of the external spool holders.
const EXTERNAL_UNIT: u32 = 255;

/// `ams.tray_now`: 255 is none and 254 the external spool. 0x80–0x87 are
/// standalone units (AMS HT, one tray each). Any other value packs four
/// trays per unit: `ams_id = v >> 2`, `tray = v & 3`.
pub fn decode_tray_now(v: u32) -> Option<TrayRef> {
    match v {
        255 => None,
        254 => Some(TrayRef {
            ams_id: EXTERNAL_UNIT,
            tray_id: 254,
        }),
        0x80..=0x87 => Some(TrayRef {
            ams_id: v,
            tray_id: 0,
        }),
        0..=0x7F => Some(TrayRef {
            ams_id: v >> 2,
            tray_id: v & 3,
        }),
        _ => None,
    }
}

/// `device.extruder.info[].snow` on the H2 series: `(ams_id << 8) | slot`,
/// 0xFFFF for none. Units 254 and 255 are the external spool holders, read
/// as the external tray with that id.
pub fn decode_extruder_tray(v: u32) -> Option<TrayRef> {
    if v >= 0xFFFF {
        return None;
    }
    let (ams_id, slot) = (v >> 8, v & 0xFF);
    if ams_id >= 254 {
        return Some(TrayRef {
            ams_id: EXTERNAL_UNIT,
            tray_id: ams_id,
        });
    }
    Some(TrayRef {
        ams_id,
        tray_id: slot,
    })
}

/// The plate number in a local job's file name: the digits between the
/// last `_` and the last `.` of the name itself, not of any directory
/// (`plate_2.gcode` is plate 2).
pub fn plate_from_gcode_file(file: &str) -> Option<u32> {
    // Only the file name counts, never a directory above it.
    let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let dot = file.rfind('.')?;
    let under = file[..dot].rfind('_')?;
    let digits = &file[under + 1..dot];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

impl PrinterState {
    /// `plate_idx` when the printer reports one, else the plate in the
    /// file name.
    pub fn plate(&self) -> Option<u32> {
        self.plate_idx
            .filter(|n| *n > 0)
            .or_else(|| self.gcode_file.as_deref().and_then(plate_from_gcode_file))
    }

    /// The tray feeding the print. On the H2 series this is the active
    /// extruder's `snow`; elsewhere it is `ams.tray_now`.
    pub fn current_tray(&self) -> Option<TrayRef> {
        let packed: Vec<&Nozzle> = self
            .nozzles
            .iter()
            .filter(|n| n.tray_now.is_some())
            .collect();
        if packed.is_empty() {
            return self.tray_now.and_then(decode_tray_now);
        }
        // Never guess the extruder: without a known active one, no tray.
        let active = self.active_nozzle?;
        let nozzle = packed.iter().find(|n| n.id == active)?;
        nozzle.tray_now.and_then(decode_extruder_tray)
    }

    /// What the printer reports for the tray at `at`. An external tray
    /// (254 or 255) is the spool with that id; when the printer has only one
    /// external spool, that spool, whichever id it reports.
    pub fn tray_at(&self, at: TrayRef) -> Option<&Tray> {
        if at.ams_id == EXTERNAL_UNIT {
            return self.external_spools.iter().find(|t| t.id == at.tray_id).or(
                match self.external_spools.as_slice() {
                    [only] => Some(only),
                    _ => None,
                },
            );
        }
        self.ams_units
            .iter()
            .find(|u| u.id == at.ams_id)?
            .trays
            .iter()
            .find(|t| t.id == at.tray_id)
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
            task_id: id_field(p, "task_id"),
            subtask_id: id_field(p, "subtask_id"),
            job_id: id_field(p, "job_id"),
            gcode_file: str_field(p, "gcode_file"),
            plate_idx: p.get("plate_idx").and_then(num_u32),
            print_type: str_field(p, "print_type"),
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
                .map(|units| {
                    units
                        .iter()
                        .filter_map(ams_unit)
                        .filter(|u| ams_unit_exists(ams_exist_mask(p), u.id))
                        .collect::<Vec<_>>()
                })
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
                    tray_now: e.get("snow").and_then(num_u32),
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
        tray_now: None,
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
    if tray_type.is_empty() && tray_info_idx.is_empty() {
        // An empty slot reports nothing, whatever stale fields came along.
        return Some(Tray {
            id,
            empty: true,
            ..Tray::default()
        });
    }
    Some(Tray {
        id,
        empty: false,
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

/// An id that arrives as a string or a number, kept as a string.
fn id_field(p: &Map<String, Value>, key: &str) -> Option<String> {
    let id = match p.get(key)? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9e15 => (f as i64).to_string(),
            _ => n.to_string(),
        },
        _ => return None,
    };
    // "5521.0" is the same id as "5521".
    let id = match id.split_once('.') {
        Some((whole, frac))
            if !whole.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && !frac.is_empty()
                && frac.bytes().all(|b| b == b'0') =>
        {
            whole.to_string()
        }
        _ => id,
    };
    (!id.is_empty()).then_some(id)
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
            "ams":{"ams":[{"id":"0","tray":[{"id":"1"}]}]}}}"#;
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

    fn delta(json: &str) -> String {
        format!(r#"{{"print":{{"command":"push_status","msg":1,{json}}}}}"#)
    }

    #[test]
    fn a_large_no_msg_delta_without_ams_merges_instead_of_replacing() {
        let fields: Vec<String> = (0..22).map(|i| format!(r#""extra_{i}":{i}"#)).collect();
        let big = format!(
            r#"{{"print":{{"command":"push_status","mc_percent":9,{}}}}}"#,
            fields.join(",")
        );
        assert!(matches!(
            parse_report(big.as_bytes()).unwrap(),
            Report::Status { full: false, .. }
        ));
        let s = state_after(&[H2D_FULL, &big]);
        assert_eq!(s.mc_percent, Some(9));
        assert_eq!(s.ams_units.len(), 2);
        assert_eq!(s.nozzles.len(), 2);
        assert_eq!(s.gcode_state.as_deref(), Some("RUNNING"));
    }

    #[test]
    fn a_no_msg_push_needs_ams_and_a_device_block_to_count_as_full() {
        let fields: Vec<String> = (0..22).map(|i| format!(r#""extra_{i}":{i}"#)).collect();
        let ams_only = format!(
            r#"{{"print":{{"command":"push_status","ams":{{"ams":[]}},{}}}}}"#,
            fields.join(",")
        );
        assert!(matches!(
            parse_report(ams_only.as_bytes()).unwrap(),
            Report::Status { full: false, .. }
        ));
    }

    #[test]
    fn ams_exist_bits_prunes_units_that_are_gone() {
        let gone = delta(r#""ams":{"ams_exist_bits":"1"}"#);
        let s = state_after(&[H2D_FULL, &gone]);
        assert_eq!(s.ams_units.len(), 1);
        assert_eq!(s.ams_units[0].id, 0);
        // The unit stays gone even if the bitmask later says it exists again,
        // because the raw merged object was pruned too.
        let back = delta(r#""ams":{"ams_exist_bits":"3"}"#);
        let s = state_after(&[H2D_FULL, &gone, &back]);
        assert_eq!(s.ams_units.len(), 1);
    }

    #[test]
    fn without_ams_exist_bits_no_unit_is_pruned() {
        let no_bits = r#"{"print":{"command":"push_status","msg":0,
            "ams":{"ams":[{"id":"0","tray":[{"id":"0"}]},{"id":"1","tray":[{"id":"0"}]}]}}}"#;
        assert_eq!(state_after(&[no_bits]).ams_units.len(), 2);
    }

    #[test]
    fn a_tray_delta_with_only_id_and_state_keeps_the_tray() {
        let d = delta(r#""ams":{"ams":[{"id":"0","tray":[{"id":"1","state":10}]}]}"#);
        let s = state_after(&[H2D_FULL, &d]);
        let t = &s.ams_units[0].trays[1];
        assert!(!t.empty);
        assert_eq!(t.tray_type, "PLA");
        assert_eq!(t.tray_color, "FFFFFFFF");
        assert!(t.has_rfid());
    }

    #[test]
    fn an_id_only_element_outside_trays_does_not_wipe_it() {
        let d = delta(r#""device":{"extruder":{"info":[{"id":0}]}}"#);
        let s = state_after(&[H2D_FULL, &d]);
        assert_eq!(s.nozzles[0].temp, Some(245.0));
        assert_eq!(s.nozzles[0].diameter, Some(0.4));
    }

    #[test]
    fn an_id_only_vir_slot_empties_the_external_spool() {
        let d = delta(r#""vir_slot":[{"id":"254"}]"#);
        let s = state_after(&[H2D_FULL, &d]);
        assert!(s.external_spools[0].empty);
        assert_eq!(s.external_spools[0].tray_info_idx, "");
    }

    #[test]
    fn an_empty_tray_reports_no_stale_fields() {
        let stale = r#"{"print":{"command":"push_status","msg":0,
            "ams":{"ams":[{"id":"0","tray":[{"id":"0","tray_type":"","tray_info_idx":"",
            "tray_color":"FF0000FF","tray_sub_brands":"Old","remain":50,
            "nozzle_temp_min":"190","nozzle_temp_max":"230",
            "tag_uid":"ABCDEF0123456789","tray_uuid":"DE69ECEBB924468D99CA2A9DFCB5FD3B"}]}]}}}"#;
        let t = &state_after(&[stale]).ams_units[0].trays[0];
        assert!(t.empty);
        assert_eq!(
            t,
            &Tray {
                id: 0,
                empty: true,
                ..Tray::default()
            }
        );
        assert!(!t.has_rfid());
    }

    #[test]
    fn active_nozzle_decodes_the_high_nibble_of_the_extruder_state() {
        let d = delta(r#""device":{"extruder":{"state":18}}"#);
        let s = state_after(&[H2D_FULL, &d]);
        assert_eq!(s.active_nozzle, Some(1));
    }

    #[test]
    fn an_empty_tray_list_in_a_delta_is_a_no_op() {
        let d = delta(r#""ams":{"ams":[{"id":"0","tray":[]}]}"#);
        let s = state_after(&[H2D_FULL, &d]);
        assert_eq!(s.ams_units[0].trays.len(), 4);
        assert_eq!(s.ams_units[0].trays[1].tray_info_idx, "GFA00");
    }

    #[test]
    fn an_empty_hms_list_clears_the_codes() {
        let d = delta(r#""hms":[]"#);
        assert!(state_after(&[H2D_FULL, &d]).hms.is_empty());
    }

    #[test]
    fn partial_json_deserializes_into_a_printer_state() {
        let s: PrinterState = serde_json::from_str(r#"{"gcode_state":"IDLE"}"#).unwrap();
        assert_eq!(s.gcode_state.as_deref(), Some("IDLE"));
        assert!(s.ams_units.is_empty());
        let t: Tray = serde_json::from_str(r#"{"id":2}"#).unwrap();
        assert_eq!(t.id, 2);
        let full = state_after(&[H2D_FULL]);
        let round: PrinterState =
            serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(round, full);
    }

    #[test]
    fn print_type_parses_and_survives_a_diff_without_it() {
        let system = delta(r#""print_type":"system""#);
        let s = state_after(&[H2D_FULL, &system]);
        assert_eq!(s.print_type.as_deref(), Some("system"));
        let s = state_after(&[H2D_FULL, &system, H2D_DELTA_PROGRESS]);
        assert_eq!(s.print_type.as_deref(), Some("system"), "kept by the merge");
        assert_eq!(state_after(&[H2D_FULL]).print_type, None);
    }

    #[test]
    fn job_identity_fields_parse_from_strings_and_numbers() {
        let local = delta(
            r#""task_id":"0","subtask_id":"0","job_id":5521,"gcode_file":"/data/Metadata/plate_2.gcode","plate_idx":"2""#,
        );
        let s = state_after(&[H2D_FULL, &local]);
        assert_eq!(s.task_id.as_deref(), Some("0"));
        assert_eq!(s.subtask_id.as_deref(), Some("0"));
        assert_eq!(s.job_id.as_deref(), Some("5521"));
        assert_eq!(
            s.gcode_file.as_deref(),
            Some("/data/Metadata/plate_2.gcode")
        );
        assert_eq!(s.plate_idx, Some(2));
        assert_eq!(s.plate(), Some(2));
        let cloud = delta(r#""task_id":"812345678","job_id":"77""#);
        let s = state_after(&[H2D_FULL, &cloud]);
        assert_eq!(s.task_id.as_deref(), Some("812345678"));
        assert_eq!(s.job_id.as_deref(), Some("77"));
    }

    #[test]
    fn the_plate_falls_back_to_the_gcode_file_name() {
        assert_eq!(
            plate_from_gcode_file("/data/Metadata/plate_3.gcode"),
            Some(3)
        );
        assert_eq!(plate_from_gcode_file("cube_plate_12.gcode"), Some(12));
        assert_eq!(plate_from_gcode_file("cube.gcode.3mf"), None);
        assert_eq!(plate_from_gcode_file("plate_.gcode"), None);
        assert_eq!(plate_from_gcode_file("no-dot"), None);
        let s = PrinterState {
            plate_idx: Some(0),
            gcode_file: Some("/data/Metadata/plate_4.gcode".into()),
            ..Default::default()
        };
        assert_eq!(s.plate(), Some(4), "plate_idx 0 means not reported");
    }

    #[test]
    fn tray_now_decodes_none_external_standalone_and_packed_values() {
        assert_eq!(decode_tray_now(255), None);
        assert_eq!(
            decode_tray_now(254),
            Some(TrayRef {
                ams_id: 255,
                tray_id: 254
            })
        );
        assert_eq!(
            decode_tray_now(0x80),
            Some(TrayRef {
                ams_id: 128,
                tray_id: 0
            })
        );
        assert_eq!(
            decode_tray_now(0x87),
            Some(TrayRef {
                ams_id: 135,
                tray_id: 0
            })
        );
        assert_eq!(
            decode_tray_now(0),
            Some(TrayRef {
                ams_id: 0,
                tray_id: 0
            })
        );
        assert_eq!(
            decode_tray_now(6),
            Some(TrayRef {
                ams_id: 1,
                tray_id: 2
            })
        );
        assert_eq!(
            decode_tray_now(15),
            Some(TrayRef {
                ams_id: 3,
                tray_id: 3
            })
        );
        assert_eq!(decode_tray_now(300), None);
    }

    #[test]
    fn h2_extruder_trays_decode_from_ams_and_slot() {
        assert_eq!(decode_extruder_tray(0xFFFF), None);
        assert_eq!(
            decode_extruder_tray(259),
            Some(TrayRef {
                ams_id: 1,
                tray_id: 3
            })
        );
        assert_eq!(
            decode_extruder_tray(128 << 8),
            Some(TrayRef {
                ams_id: 128,
                tray_id: 0
            })
        );
        assert_eq!(
            decode_extruder_tray(0xFEFF),
            Some(TrayRef {
                ams_id: 255,
                tray_id: 254
            })
        );
        assert_eq!(
            decode_extruder_tray(0xFF00),
            Some(TrayRef {
                ams_id: 255,
                tray_id: 255
            })
        );
        assert_eq!(decode_extruder_tray(0x1_0000), None);
    }

    #[test]
    fn the_current_tray_comes_from_the_active_extruder_on_the_h2() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.nozzles[0].tray_now, Some(259));
        assert_eq!(s.nozzles[1].tray_now, Some(0xFEFF));
        assert_eq!(
            s.current_tray(),
            Some(TrayRef {
                ams_id: 1,
                tray_id: 3
            }),
            "snow on the active extruder beats tray_now"
        );
        let left = delta(r#""device":{"extruder":{"state":18}}"#);
        let s = state_after(&[H2D_FULL, &left]);
        let ext = TrayRef {
            ams_id: 255,
            tray_id: 254,
        };
        assert_eq!(s.current_tray(), Some(ext));
        assert_eq!(
            s.tray_at(ext).map(|t| t.tray_info_idx.as_str()),
            Some("GFA01")
        );
    }

    #[test]
    fn the_current_tray_comes_from_tray_now_elsewhere() {
        let s = state_after(&[P1P_FULL, P1P_DELTA]);
        assert_eq!(s.current_tray(), None, "255 is nothing loaded");
        let s = PrinterState {
            tray_now: Some(1),
            ams_units: state_after(&[H2D_FULL]).ams_units,
            ..Default::default()
        };
        let at = s.current_tray().unwrap();
        assert_eq!(
            at,
            TrayRef {
                ams_id: 0,
                tray_id: 1
            }
        );
        assert_eq!(s.tray_at(at).map(|t| t.tray_type.as_str()), Some("PLA"));
        assert_eq!(
            s.tray_at(TrayRef {
                ams_id: 9,
                tray_id: 0
            }),
            None
        );
    }

    #[test]
    fn an_external_tray_resolves_to_the_only_external_spool_whatever_its_id() {
        let one = |id| PrinterState {
            external_spools: vec![Tray {
                id,
                tray_type: "PLA".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        for spool_id in [254, 255] {
            let s = one(spool_id);
            for tray_id in [254, 255] {
                let at = TrayRef {
                    ams_id: 255,
                    tray_id,
                };
                assert_eq!(s.tray_at(at).map(|t| t.id), Some(spool_id));
            }
        }
        let mut two = one(254);
        two.external_spools.push(Tray {
            id: 255,
            tray_type: "PETG".into(),
            ..Default::default()
        });
        let right = TrayRef {
            ams_id: 255,
            tray_id: 255,
        };
        assert_eq!(
            two.tray_at(right).map(|t| t.tray_type.as_str()),
            Some("PETG")
        );
        two.external_spools.remove(1);
        two.external_spools.push(Tray {
            id: 7,
            ..Default::default()
        });
        assert_eq!(two.tray_at(right), None, "two spools, no exact match");
    }

    #[test]
    fn job_ids_are_normalised_and_empty_ids_are_absent() {
        let id = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            id_field(v.as_object().unwrap(), "job_id")
        };
        assert_eq!(id(r#"{"job_id":""}"#), None);
        assert_eq!(id(r#"{"job_id":"  "}"#), None);
        assert_eq!(id(r#"{"job_id":5521.0}"#).as_deref(), Some("5521"));
        assert_eq!(id(r#"{"job_id":"5521.0"}"#).as_deref(), Some("5521"));
        assert_eq!(id(r#"{"job_id":" 5521 "}"#).as_deref(), Some("5521"));
        assert_eq!(id(r#"{"job_id":5521}"#).as_deref(), Some("5521"));
        assert_eq!(id(r#"{"job_id":"12.5"}"#).as_deref(), Some("12.5"));
        assert_eq!(id(r#"{"job_id":"0"}"#).as_deref(), Some("0"));
        assert_eq!(id(r#"{"job_id":null}"#), None);
        assert_eq!(id(r#"{"job_id":true}"#), None);
        assert_eq!(id(r#"{"other":1}"#), None);
        let s = state_after(&[H2D_FULL, &delta(r#""task_id":"","job_id":5521.0"#)]);
        assert_eq!(s.task_id, None);
        assert_eq!(s.job_id.as_deref(), Some("5521"));
    }

    #[test]
    fn the_plate_is_read_from_the_file_name_only() {
        assert_eq!(plate_from_gcode_file("/data/proj_5.d/plate"), None);
        assert_eq!(plate_from_gcode_file("/data/my_2.x/cube"), None);
        assert_eq!(plate_from_gcode_file("/data/my_2.x/plate_7.gcode"), Some(7));
        assert_eq!(plate_from_gcode_file(r"C:\jobs_9.d\plate_3.gcode"), Some(3));
        assert_eq!(plate_from_gcode_file("a.b_2.3mf"), Some(2));
        assert_eq!(plate_from_gcode_file("plate_0.gcode"), None);
        assert_eq!(plate_from_gcode_file("é_2.gcode"), Some(2));
        assert_eq!(
            plate_from_gcode_file("plate_99999999999999999999.gcode"),
            None
        );
        assert_eq!(plate_from_gcode_file(""), None);
        assert_eq!(plate_from_gcode_file("/"), None);
    }

    #[test]
    fn the_extruder_is_never_guessed() {
        let mut s = state_after(&[H2D_FULL]);
        s.active_nozzle = None;
        assert_eq!(s.current_tray(), None, "no active extruder, no tray");
        s.active_nozzle = Some(5);
        assert_eq!(s.current_tray(), None, "unknown extruder id");
        s.active_nozzle = Some(1);
        s.nozzles[1].tray_now = None;
        assert_eq!(s.current_tray(), None, "active extruder reports no snow");
    }

    #[test]
    fn a_malformed_value_in_a_diff_keeps_the_merged_one() {
        let good = delta(
            r#""plate_idx":"2","mc_percent":40,"ams":{"tray_now":"1"},"device":{"extruder":{"info":[{"id":0,"snow":259}]}}"#,
        );
        let bad = delta(
            r#""plate_idx":"abc","mc_percent":null,"ams":{"tray_now":"0x1F"},"device":{"extruder":{"info":[{"id":0,"snow":"zz"}]}}"#,
        );
        let s = state_after(&[H2D_FULL, &good, &bad]);
        assert_eq!(s.plate_idx, Some(2));
        assert_eq!(s.mc_percent, Some(40));
        assert_eq!(s.tray_now, Some(1));
        assert_eq!(s.nozzles[0].tray_now, Some(259));
        let neg = delta(r#""plate_idx":-3,"layer_num":"-1""#);
        let s2 = state_after(&[H2D_FULL, &good, &neg]);
        assert_eq!(s2.plate_idx, Some(2), "a negative number is malformed too");
        assert_eq!(s2.layer_num, state_after(&[H2D_FULL]).layer_num);
    }

    #[test]
    fn a_diff_that_omits_the_job_fields_keeps_them() {
        let job =
            delta(r#""task_id":"812","job_id":"77","gcode_file":"plate_2.gcode","plate_idx":"2""#);
        let progress = delta(r#""mc_percent":50"#);
        let s = state_after(&[H2D_FULL, &job, &progress]);
        assert_eq!(s.task_id.as_deref(), Some("812"));
        assert_eq!(s.job_id.as_deref(), Some("77"));
        assert_eq!(s.gcode_file.as_deref(), Some("plate_2.gcode"));
        assert_eq!(s.plate_idx, Some(2));
        assert_eq!(s.mc_percent, Some(50));
    }

    #[test]
    fn odd_numbers_are_skipped_not_panicked_on() {
        for v in [
            r#""0x1F""#,
            "-3",
            r#""NaN""#,
            r#""inf""#,
            "1e30",
            "null",
            "[]",
        ] {
            let d = delta(&format!(r#""plate_idx":{v}"#));
            let s = state_after(&[&d]);
            assert_eq!(s.plate_idx, None, "{v}");
        }
        let s = state_after(&[&delta(r#""plate_idx":"2.5""#)]);
        assert_eq!(s.plate_idx, Some(2));
    }

    #[test]
    fn decode_tray_now_leaves_the_gap_undecoded() {
        assert_eq!(decode_tray_now(0x88), None);
        assert_eq!(decode_tray_now(253), None);
        assert_eq!(
            decode_tray_now(0x7F),
            Some(TrayRef {
                ams_id: 31,
                tray_id: 3
            })
        );
        assert_eq!(decode_tray_now(u32::MAX), None);
        assert_eq!(decode_extruder_tray(u32::MAX), None);
    }

    #[test]
    fn current_trays_get_the_labels_the_printer_page_shows() {
        use crate::printer::slots::slot_label;
        let label = |at: TrayRef, ext: usize| slot_label(at.ams_id, at.tray_id, ext);
        assert_eq!(label(decode_tray_now(1).unwrap(), 0), "A2");
        assert_eq!(label(decode_tray_now(0x80).unwrap(), 0), "HT1");
        assert_eq!(label(decode_tray_now(254).unwrap(), 1), "Ext");
        assert_eq!(label(decode_extruder_tray(259).unwrap(), 0), "B4");
        assert_eq!(label(decode_extruder_tray(0xFEFF).unwrap(), 2), "Ext-L");

        let ht = |tray_type: &str| AmsUnit {
            id: 128,
            trays: vec![Tray {
                id: 0,
                tray_type: tray_type.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let s = PrinterState {
            tray_now: Some(0x80),
            ams_units: vec![ht("TPU")],
            ..Default::default()
        };
        let at = s.current_tray().unwrap();
        assert_eq!(
            at,
            TrayRef {
                ams_id: 128,
                tray_id: 0
            }
        );
        assert_eq!(s.tray_at(at).map(|t| t.tray_type.as_str()), Some("TPU"));
    }
}
