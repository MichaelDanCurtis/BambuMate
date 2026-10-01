//! Reads what Bambu Studio wrote: the output `.gcode.3mf` (a ZIP) and the
//! `result.json` the CLI leaves in its output directory.
//!
//! The parser is lenient about content (optional fields, unknown tags
//! ignored) and strict about the container: entry counts and sizes are
//! capped, and an archive with an unsafe entry name is rejected outright.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use super::SlicerError;

/// More entries than any real Bambu Studio output (a 3-plate project has ~45).
pub const MAX_ENTRIES: usize = 4096;
const MAX_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SETTINGS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PNG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PLATES: u32 = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceResult {
    pub plates: Vec<PlateResult>,
    /// `printer_settings_id`, e.g. "Bambu Lab H2C 0.4 nozzle".
    pub printer: String,
    /// `printer_model`, e.g. "Bambu Lab H2C".
    pub printer_model: String,
    /// `print_settings_id`, e.g. "0.20mm Standard @BBL H2C".
    pub process: String,
    pub filaments: Vec<FilamentInfo>,
    pub bed_type: Option<String>,
    pub bambu_studio_version: String,
    /// Absolute path of the cached `.gcode.3mf`. Empty until the job layer
    /// moves the output into the cache.
    pub output_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlateResult {
    /// 1-based plate number.
    pub index: u32,
    pub time_seconds: u64,
    pub weight_g: f64,
    /// Sum of the filament costs, when any filament has a cost set.
    pub cost: Option<f64>,
    pub filaments: Vec<FilamentUse>,
    pub warnings: Vec<SliceWarning>,
    pub objects: Vec<SliceObject>,
    /// File name of the extracted plate thumbnail, next to the cached output.
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilamentUse {
    /// 1-based filament slot in the project.
    pub slot: u32,
    pub filament_type: String,
    pub color: String,
    pub used_g: f64,
    pub used_m: f64,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilamentInfo {
    pub slot: u32,
    pub preset: String,
    pub filament_type: String,
    pub color: String,
    /// `filament_cost`, per kilogram. `None` when unset (Bambu Studio's 0).
    pub cost_per_kg: Option<f64>,
    pub density: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningLevel {
    Notice,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceWarning {
    pub level: WarningLevel,
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceObject {
    pub id: String,
    pub name: String,
}

/// The CLI's own `result.json`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CliResult {
    #[serde(default)]
    pub return_code: i64,
    #[serde(default)]
    pub error_string: String,
    #[serde(default)]
    pub sliced_plates: Vec<CliPlate>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CliPlate {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub warning_message: String,
}

pub fn parse_cli_result(text: &str) -> Option<CliResult> {
    serde_json::from_str(text).ok()
}

/// Bambu Studio's `CLI_SLICING_ERROR`: a generic message whose detail is on stderr.
const CLI_SLICING_ERROR: i64 = -100;
const MAX_MESSAGE_CHARS: usize = 300;

/// The CLI's own words for a failed run, trimmed. Prefers the specific
/// stderr line when `result.json` only has the generic slicing error.
pub fn cli_error_message(
    result: Option<&CliResult>,
    stderr: &str,
    exit_code: Option<i32>,
) -> String {
    let stderr_line = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with('[') && *l != "run found error, exit")
        .map(str::to_string);
    let from_result = result
        .filter(|r| r.return_code != 0 && !r.error_string.trim().is_empty())
        .map(|r| (r.return_code, r.error_string.trim().to_string()));
    let message = match (from_result, stderr_line) {
        (Some((CLI_SLICING_ERROR, _)), Some(line)) => line,
        (Some((_, text)), _) => text,
        (None, Some(line)) => line,
        (None, None) => match exit_code {
            Some(code) => format!("it exited with status {code}"),
            None => "it stopped unexpectedly".to_string(),
        },
    };
    truncate_chars(&message, MAX_MESSAGE_CHARS)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Parses a sliced `.gcode.3mf`. `cli_result` is the CLI's `result.json`
/// text, when present: it carries the per-plate warnings (such as "floating
/// cantilever") that `slice_info.config` does not. When `thumbnails_to` is
/// set, each `Metadata/plate_N.png` is written there as `plate_N.png`.
pub fn parse_output(
    gcode_3mf: &Path,
    cli_result: Option<&str>,
    thumbnails_to: Option<&Path>,
) -> Result<SliceResult, SlicerError> {
    parse_output_inner(gcode_3mf, cli_result, thumbnails_to).map_err(|e| {
        debug!("could not read slicer output {}: {e}", gcode_3mf.display());
        SlicerError::BadOutput
    })
}

fn parse_output_inner(
    gcode_3mf: &Path,
    cli_result: Option<&str>,
    thumbnails_to: Option<&Path>,
) -> Result<SliceResult, String> {
    let file = std::fs::File::open(gcode_3mf).map_err(|e| format!("open: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("zip: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} entries (cap {MAX_ENTRIES})", zip.len()));
    }
    let mut thumbnails: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| format!("entry {i}: {e}"))?;
        if !is_safe_entry_name(entry.name()) || entry.enclosed_name().is_none() {
            return Err(format!("unsafe entry name {:?}", entry.name()));
        }
        if let Some(n) = plate_png_index(entry.name()) {
            thumbnails.push((n, entry.name().to_string()));
        }
    }

    let slice_info = read_text(&mut zip, "Metadata/slice_info.config", MAX_XML_BYTES)?
        .ok_or("no Metadata/slice_info.config")?;
    let settings: Value = match read_text(
        &mut zip,
        "Metadata/project_settings.config",
        MAX_SETTINGS_BYTES,
    )? {
        Some(text) => serde_json::from_str(&text).map_err(|e| format!("project_settings: {e}"))?,
        None => Value::Null,
    };
    let info = parse_slice_info(&slice_info)?;
    if info.plates.is_empty() {
        return Err("slice_info has no plates".into());
    }

    let filaments = filament_info(&settings);
    let cost_by_slot: HashMap<u32, f64> = filaments
        .iter()
        .filter_map(|f| f.cost_per_kg.map(|c| (f.slot, c)))
        .collect();
    let cli = cli_result.and_then(parse_cli_result);

    let mut plates = Vec::new();
    for raw in info.plates {
        let mut uses = Vec::new();
        for f in raw.filaments {
            let cost = cost_by_slot
                .get(&f.slot)
                .map(|per_kg| f.used_g * per_kg / 1000.0);
            uses.push(FilamentUse { cost, ..f });
        }
        let cost = if uses.iter().any(|u| u.cost.is_some()) {
            Some(uses.iter().filter_map(|u| u.cost).sum())
        } else {
            None
        };
        let weight_g = raw
            .weight_g
            .unwrap_or_else(|| uses.iter().map(|u| u.used_g).sum());
        let mut warnings = raw.warnings;
        if let Some(cli_plate) = cli
            .as_ref()
            .and_then(|c| c.sliced_plates.iter().find(|p| p.id == raw.index))
        {
            let msg = cli_plate.warning_message.trim();
            if !msg.is_empty() && !warnings.iter().any(|w| w.message == msg) {
                warnings.push(SliceWarning {
                    level: WarningLevel::Warning,
                    message: msg.to_string(),
                    code: None,
                });
            }
        }
        let mut thumbnail = None;
        if let (Some(dir), Some((_, entry))) = (
            thumbnails_to,
            thumbnails.iter().find(|(n, _)| *n == raw.index),
        ) {
            if let Some(bytes) = read_bytes(&mut zip, entry, MAX_PNG_BYTES)? {
                let name = format!("plate_{}.png", raw.index);
                std::fs::write(dir.join(&name), bytes).map_err(|e| format!("thumbnail: {e}"))?;
                thumbnail = Some(name);
            }
        }
        plates.push(PlateResult {
            index: raw.index,
            time_seconds: raw.time_seconds,
            weight_g,
            cost,
            filaments: uses,
            warnings,
            objects: raw.objects,
            thumbnail,
        });
    }
    plates.sort_by_key(|p| p.index);

    Ok(SliceResult {
        plates,
        printer: setting_str(&settings, "printer_settings_id"),
        printer_model: setting_str(&settings, "printer_model"),
        process: setting_str(&settings, "print_settings_id"),
        filaments,
        bed_type: Some(setting_str(&settings, "curr_bed_type")).filter(|s| !s.is_empty()),
        bambu_studio_version: info.version,
        output_path: String::new(),
    })
}

/// A relative, forward-slash path with no `..`, drive or NUL. Anything else
/// in a slicer output means the file isn't what Bambu Studio wrote.
fn is_safe_entry_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.contains('\0')
        && name.split('/').all(|part| part != "..")
}

/// `Metadata/plate_<n>.png` → `n`. Other PNGs (`plate_1_small.png`,
/// `plate_no_light_1.png`, `top_1.png`) are not thumbnails.
fn plate_png_index(name: &str) -> Option<u32> {
    let n: u32 = name
        .strip_prefix("Metadata/plate_")?
        .strip_suffix(".png")?
        .parse()
        .ok()?;
    (1..=MAX_PLATES).contains(&n).then_some(n)
}

fn read_bytes<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    cap: u64,
) -> Result<Option<Vec<u8>>, String> {
    let entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("{name}: {e}")),
    };
    if entry.size() > cap {
        return Err(format!(
            "{name} declares {} bytes (cap {cap})",
            entry.size()
        ));
    }
    // Never trust the declared size: read at most cap + 1 bytes.
    let mut buf = Vec::new();
    entry
        .take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("{name}: {e}"))?;
    if buf.len() as u64 > cap {
        return Err(format!("{name} is larger than {cap} bytes"));
    }
    Ok(Some(buf))
}

fn read_text<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    cap: u64,
) -> Result<Option<String>, String> {
    read_bytes(zip, name, cap)?
        .map(|b| String::from_utf8(b).map_err(|e| format!("{name}: {e}")))
        .transpose()
}

fn setting_str(settings: &Value, key: &str) -> String {
    match settings.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.first().and_then(|v| v.as_str()).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn setting_list(settings: &Value, key: &str) -> Vec<String> {
    match settings.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn positive(s: Option<&String>) -> Option<f64> {
    s.and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| *v > 0.0)
}

fn filament_info(settings: &Value) -> Vec<FilamentInfo> {
    let presets = setting_list(settings, "filament_settings_id");
    let types = setting_list(settings, "filament_type");
    let colours = setting_list(settings, "filament_colour");
    let costs = setting_list(settings, "filament_cost");
    let densities = setting_list(settings, "filament_density");
    (0..presets.len().max(types.len()))
        .map(|i| FilamentInfo {
            slot: i as u32 + 1,
            preset: presets.get(i).cloned().unwrap_or_default(),
            filament_type: types.get(i).cloned().unwrap_or_default(),
            color: colours.get(i).cloned().unwrap_or_default(),
            cost_per_kg: positive(costs.get(i)),
            density: positive(densities.get(i)),
        })
        .collect()
}

#[derive(Default)]
struct RawPlate {
    index: u32,
    time_seconds: u64,
    weight_g: Option<f64>,
    filaments: Vec<FilamentUse>,
    warnings: Vec<SliceWarning>,
    objects: Vec<SliceObject>,
}

#[derive(Default)]
struct SliceInfo {
    version: String,
    plates: Vec<RawPlate>,
}

fn attrs(e: &BytesStart) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(|err| format!("attribute: {err}"))?;
        let key = a.key.as_ref().trim().to_string();
        let value = a
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| format!("attribute {key}: {err}"))?
            .into_owned();
        out.insert(key, value);
    }
    Ok(out)
}

fn parse_slice_info(xml: &str) -> Result<SliceInfo, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut info = SliceInfo::default();
    let mut plate: Option<RawPlate> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("slice_info at {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let a = attrs(&e)?;
                match (e.name().as_ref(), plate.as_mut()) {
                    ("plate", _) => plate = Some(RawPlate::default()),
                    ("header_item", _) => {
                        if a.get("key").map(String::as_str) == Some("X-BBL-Client-Version") {
                            info.version = a.get("value").cloned().unwrap_or_default();
                        }
                    }
                    ("metadata", Some(p)) => {
                        let value = a.get("value").map(String::as_str).unwrap_or("");
                        match a.get("key").map(String::as_str) {
                            Some("index") => p.index = value.parse().unwrap_or(0),
                            Some("prediction") => p.time_seconds = value.parse().unwrap_or(0),
                            Some("weight") => p.weight_g = value.parse().ok(),
                            _ => {}
                        }
                    }
                    ("object", Some(p)) => p.objects.push(SliceObject {
                        id: a.get("identify_id").cloned().unwrap_or_default(),
                        name: a.get("name").cloned().unwrap_or_default(),
                    }),
                    ("filament", Some(p)) => p.filaments.push(FilamentUse {
                        slot: a.get("id").and_then(|v| v.parse().ok()).unwrap_or(0),
                        filament_type: a.get("type").cloned().unwrap_or_default(),
                        color: a.get("color").cloned().unwrap_or_default(),
                        used_g: a.get("used_g").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                        used_m: a.get("used_m").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                        cost: None,
                    }),
                    ("warning", Some(p)) => {
                        let key = a.get("msg").cloned().unwrap_or_default();
                        let level: u8 = a.get("level").and_then(|v| v.parse().ok()).unwrap_or(1);
                        let message = warning_text(&key);
                        if !p.warnings.iter().any(|w| w.message == message) {
                            p.warnings.push(SliceWarning {
                                level: if level >= 2 {
                                    WarningLevel::Warning
                                } else {
                                    WarningLevel::Notice
                                },
                                message,
                                code: a.get("error_code").cloned().filter(|c| !c.is_empty()),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if e.name().as_ref() == "plate" => {
                if let Some(p) = plate.take() {
                    if p.index > 0 && p.index <= MAX_PLATES {
                        info.plates.push(p);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(info)
}

/// `slice_info.config` warnings carry a key, not text. Known keys get
/// BambuMate's wording; unknown ones are shown as readable words.
fn warning_text(key: &str) -> String {
    match key {
        "smooth_timelapse_without_prime_tower" => {
            "A smooth timelapse needs a prime tower; without one the video may show defects."
                .to_string()
        }
        "not_generate_timelapse" => "No timelapse will be recorded for this print.".to_string(),
        "not_support_traditional_timelapse" => {
            "This print doesn't support a traditional timelapse.".to_string()
        }
        "the_actual_nozzle_hrc_smaller_than_the_required_nozzle_hrc" => {
            "The nozzle is softer than this filament needs.".to_string()
        }
        "activate_long_retraction_when_cut" => {
            "Long retraction when cutting is turned on.".to_string()
        }
        other => {
            let words = other.replace('_', " ");
            let mut chars = words.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Bambu Studio reported a warning.".to_string(),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    pub(crate) fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/slicer")
            .join(name)
    }

    fn result_json(name: &str) -> String {
        std::fs::read_to_string(fixture(name)).unwrap()
    }

    #[test]
    fn cube_has_one_plate_with_time_weight_cost_and_object() {
        let dir = tempfile::tempdir().unwrap();
        let r = parse_output(
            &fixture("cube_h2c.gcode.3mf"),
            Some(&result_json("cube_h2c.result.json")),
            Some(dir.path()),
        )
        .unwrap();
        assert_eq!(r.bambu_studio_version, "02.08.02.61");
        assert_eq!(r.printer, "Bambu Lab H2C 0.4 nozzle");
        assert_eq!(r.printer_model, "Bambu Lab H2C");
        assert_eq!(r.process, "0.20mm Standard @BBL H2C");
        assert_eq!(r.bed_type.as_deref(), Some("Textured PEI Plate"));
        assert_eq!(r.plates.len(), 1);
        let p = &r.plates[0];
        assert_eq!(p.index, 1);
        assert_eq!(p.time_seconds, 843);
        assert!((p.weight_g - 3.69).abs() < 1e-9);
        assert_eq!(p.filaments.len(), 1);
        let f = &p.filaments[0];
        assert_eq!(
            (f.slot, f.filament_type.as_str(), f.color.as_str()),
            (1, "PLA", "#00AE42")
        );
        assert!((f.used_m - 1.22).abs() < 1e-9);
        // 3.69 g × 19.99 per kg / 1000
        let cost = f.cost.unwrap();
        assert!((cost - 3.69 * 19.99 / 1000.0).abs() < 1e-9, "{cost}");
        assert_eq!(p.cost, Some(cost));
        assert_eq!(
            p.objects,
            vec![SliceObject {
                id: "15".into(),
                name: "cube20.stl".into()
            }]
        );
        assert!(p.warnings.is_empty());
        assert_eq!(p.thumbnail.as_deref(), Some("plate_1.png"));
        let png = std::fs::read(dir.path().join("plate_1.png")).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
        assert_eq!(
            r.filaments,
            vec![FilamentInfo {
                slot: 1,
                preset: "Bambu PLA Basic @BBL H2C".into(),
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                cost_per_kg: Some(19.99),
                density: Some(1.26),
            }]
        );
    }

    #[test]
    fn two_plate_output_reads_each_plate() {
        let dir = tempfile::tempdir().unwrap();
        let r = parse_output(
            &fixture("two_plates_h2c.gcode.3mf"),
            Some(&result_json("two_plates_h2c.result.json")),
            Some(dir.path()),
        )
        .unwrap();
        let summary: Vec<(u32, u64, f64, &str)> = r
            .plates
            .iter()
            .map(|p| {
                (
                    p.index,
                    p.time_seconds,
                    p.weight_g,
                    p.objects[0].name.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (1, 4817, 77.98, "plateA.stl"),
                (2, 4478, 71.87, "plateB.stl")
            ]
        );
        assert!(dir.path().join("plate_2.png").is_file());
    }

    #[test]
    fn warnings_come_from_slice_info_and_result_json() {
        let r = parse_output(
            &fixture("warning_h2c.gcode.3mf"),
            Some(&result_json("warning_h2c.result.json")),
            None,
        )
        .unwrap();
        let w = &r.plates[0].warnings;
        assert_eq!(w.len(), 2, "{w:?}");
        assert_eq!(w[0].level, WarningLevel::Warning);
        assert_eq!(w[0].code.as_deref(), Some("10018004"));
        assert!(w[0].message.contains("prime tower"));
        assert_eq!(
            w[1],
            SliceWarning {
                level: WarningLevel::Warning,
                message: "It seems object tee.stl has floating cantilever. Please re-orient the object or enable support generation.".into(),
                code: None,
            }
        );
        assert_eq!(r.plates[0].thumbnail, None, "no thumbnail dir given");
    }

    #[test]
    fn missing_result_json_still_parses() {
        let r = parse_output(&fixture("warning_h2c.gcode.3mf"), None, None).unwrap();
        assert_eq!(r.plates[0].warnings.len(), 1);
    }

    #[test]
    fn cli_errors_prefer_specific_stderr_for_the_generic_slicing_error() {
        let nozzle = parse_cli_result(&result_json("error_no_nozzle.result.json")).unwrap();
        assert_eq!(nozzle.return_code, -100);
        assert_eq!(
            cli_error_message(
                Some(&nozzle),
                "No valid nozzle found. Please check nozzle count.\nrun found error, exit\n",
                Some(156)
            ),
            "No valid nozzle found. Please check nozzle count."
        );
        let empty = parse_cli_result(&result_json("error_empty_plate.result.json")).unwrap();
        assert_eq!(
            cli_error_message(Some(&empty), "", Some(206)),
            "One of the plate is empty or has no object fully inside it. Please check that the 3mf contains no empty plate in Bambu Studio before uploading."
        );
        assert_eq!(
            cli_error_message(None, "[2026-10-01 11:20:35] [error] noise\n", Some(3)),
            "it exited with status 3"
        );
        assert_eq!(cli_error_message(None, "", None), "it stopped unexpectedly");
        let long = "x".repeat(400);
        assert_eq!(cli_error_message(None, &long, None).chars().count(), 301);
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    const MIN_SLICE_INFO: &[u8] = br##"<?xml version="1.0"?><config><plate><metadata key="index" value="1"/><metadata key="prediction" value="60"/><metadata key="weight" value=""/><filament id="1" type="PLA" color="#FFFFFF" used_m="1" used_g="2.5"/></plate></config>"##;

    #[test]
    fn rejects_unsafe_entry_names() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../evil.png", "/abs/evil.png", "Metadata/../../evil"] {
            let p = dir.path().join("bad.gcode.3mf");
            write_zip(
                &p,
                &[("Metadata/slice_info.config", MIN_SLICE_INFO), (bad, b"x")],
            );
            assert_eq!(
                parse_output(&p, None, Some(dir.path())),
                Err(SlicerError::BadOutput),
                "{bad}"
            );
        }
    }

    #[test]
    fn rejects_too_many_entries() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("many.gcode.3mf");
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("junk/{i}")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![("Metadata/slice_info.config", MIN_SLICE_INFO)];
        entries.extend(names.iter().map(|n| (n.as_str(), &b""[..])));
        write_zip(&p, &entries);
        assert_eq!(parse_output(&p, None, None), Err(SlicerError::BadOutput));
    }

    #[test]
    fn rejects_oversized_entries() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.gcode.3mf");
        let huge = vec![b' '; (MAX_XML_BYTES + 1) as usize];
        write_zip(&p, &[("Metadata/slice_info.config", &huge)]);
        assert_eq!(parse_output(&p, None, None), Err(SlicerError::BadOutput));
    }

    #[test]
    fn rejects_non_zip_and_missing_slice_info() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.gcode.3mf");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert_eq!(parse_output(&junk, None, None), Err(SlicerError::BadOutput));
        let empty = dir.path().join("empty.gcode.3mf");
        write_zip(&empty, &[("3D/3dmodel.model", b"<model/>")]);
        assert_eq!(
            parse_output(&empty, None, None),
            Err(SlicerError::BadOutput)
        );
    }

    #[test]
    fn missing_weight_falls_back_to_filament_sum_and_no_cost_without_settings() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("min.gcode.3mf");
        write_zip(&p, &[("Metadata/slice_info.config", MIN_SLICE_INFO)]);
        let r = parse_output(&p, None, None).unwrap();
        assert_eq!(r.plates[0].weight_g, 2.5);
        assert_eq!(r.plates[0].cost, None);
        assert_eq!(r.plates[0].time_seconds, 60);
        assert!(r.filaments.is_empty());
    }

    #[test]
    fn only_plain_plate_pngs_are_thumbnails() {
        assert_eq!(plate_png_index("Metadata/plate_1.png"), Some(1));
        assert_eq!(plate_png_index("Metadata/plate_12.png"), Some(12));
        assert_eq!(plate_png_index("Metadata/plate_1_small.png"), None);
        assert_eq!(plate_png_index("Metadata/plate_no_light_1.png"), None);
        assert_eq!(plate_png_index("Metadata/top_1.png"), None);
        assert_eq!(plate_png_index("Metadata/plate_0.png"), None);
    }

    #[test]
    fn entry_names_must_be_plain_relative_paths() {
        assert!(is_safe_entry_name("Metadata/plate_1.png"));
        assert!(is_safe_entry_name("[Content_Types].xml"));
        for bad in ["", "/etc/x", "a/../b", "..", "C:/x", "a\\b", "a\0b"] {
            assert!(!is_safe_entry_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn unknown_warning_keys_become_readable() {
        assert_eq!(warning_text("bed_temp_too_high"), "Bed temp too high");
    }
}
