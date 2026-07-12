//! Pure HTML extractor for filament specifications.
//!
//! Extracts filament printing parameters from a manufacturer page without
//! any AI or network calls. Tries sources in priority order:
//!
//! 1. JSON-LD structured data (`<script type="application/ld+json">`)
//! 2. HTML `<table>` rows (spec sheets)
//! 3. Definition lists (`<dt>/<dd>` pairs)
//! 4. Regex patterns on the full page text
//!
//! Confidence is computed from the richness of what was found.

use scraper::{Html, Selector};
use serde_json::Value;
use std::sync::LazyLock;
use tracing::info;

use super::types::{FilamentSpecs, MaterialType};

// ─── Regex patterns ────────────────────────────────────────────────────────
//
// All patterns are compiled once via `LazyLock<Regex>` so extraction does
// not pay the ~100–500 µs compilation cost per call.

/// Range: "200-230°C", "200 ~ 230 °C", "200°C to 230°C", "200 to 230°C"
const NOZZLE_RANGE_RE: &str = r"(?i)(?:nozzle|print(?:ing)?|extrusion|hotend)[^\d]{0,30}?(\d{3})\s*[-–~to°]+\s*(\d{3})\s*°?\s*[Cc]";
/// Single nozzle temp: "Nozzle: 210°C"
const NOZZLE_SINGLE_RE: &str =
    r"(?i)(?:nozzle|print(?:ing)?|extrusion)[^\d]{0,20}?(\d{3})\s*°?\s*[Cc]";
/// Bed range: "Bed: 55-70°C", "Build Plate: 55–70 °C"
const BED_RANGE_RE: &str = r"(?i)(?:bed|build\s*plate|heated\s*bed|platform)[^\d]{0,30}?(\d{2,3})\s*[-–~to°]+\s*(\d{2,3})\s*°?\s*[Cc]";
/// Single bed temp
const BED_SINGLE_RE: &str =
    r"(?i)(?:bed|build\s*plate|heated\s*bed|platform)[^\d]{0,20}?(\d{2,3})\s*°?\s*[Cc]";
/// Density: "1.24 g/cm³" or "1.24 g/cm3"
const DENSITY_RE: &str = r"(\d+\.\d+)\s*g/cm[³3]";
/// Diameter: "1.75 mm" near diameter context
const DIAMETER_RE: &str = r"(?i)diam(?:eter)?[^\d]{0,10}?(\d\.\d+)\s*mm";
/// Bare temperature range like "200-230°C" used by `parse_temp_range`.
const BARE_RANGE_RE: &str = r"(\d{2,3})\s*[-–~to°]+\s*(\d{2,3})\s*°?\s*[Cc]?";
/// Bare single temperature like "210°C" used by `parse_temp_range`.
const BARE_SINGLE_RE: &str = r"(\d{2,3})\s*°?\s*[Cc]";

static NOZZLE_RANGE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(NOZZLE_RANGE_RE).expect("valid regex"));
static NOZZLE_SINGLE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(NOZZLE_SINGLE_RE).expect("valid regex"));
static BED_RANGE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(BED_RANGE_RE).expect("valid regex"));
static BED_SINGLE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(BED_SINGLE_RE).expect("valid regex"));
static DENSITY: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(DENSITY_RE).expect("valid regex"));
static DIAMETER: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(DIAMETER_RE).expect("valid regex"));
static BARE_RANGE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(BARE_RANGE_RE).expect("valid regex"));
static BARE_SINGLE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(BARE_SINGLE_RE).expect("valid regex"));

/// Attempt to parse the re-encoded text for nozzle/bed numbers.
/// Returns `(min, max)` or `(single, single)`.
fn parse_range(cap: &regex::Captures, idx1: usize, idx2: usize) -> Option<(u16, u16)> {
    let a: u16 = cap.get(idx1)?.as_str().parse().ok()?;
    let b: u16 = cap.get(idx2)?.as_str().parse().ok()?;
    if a == 0 || b == 0 {
        return None;
    }
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    Some((lo, hi))
}

/// Extract filament specs from raw HTML without any AI calls.
/// Returns a `FilamentSpecs` with `extraction_confidence` reflecting how
/// much data was found (typically 0.10–0.65 for pure HTML extraction).
pub fn extract(html: &str, filament_name: &str) -> FilamentSpecs {
    let document = Html::parse_document(html);
    let text = html_to_text_simple(html);

    let mut specs = FilamentSpecs {
        serial: infer_serial(filament_name),
        ..Default::default()
    };

    // Derive material and brand from the filament name
    specs.material = infer_material(filament_name);
    specs.brand = infer_brand(filament_name);

    let mut confidence: f32 = 0.0;

    // 1. JSON-LD
    if try_json_ld(&document, &mut specs, &mut confidence) {
        info!(
            "html_extractor: JSON-LD extraction contributed confidence {:.2}",
            confidence
        );
    }

    // 2. Table rows
    if confidence < 0.5 {
        try_tables(&document, &mut specs, &mut confidence);
    }

    // 3. Definition lists
    if confidence < 0.5 {
        try_definition_lists(&document, &mut specs, &mut confidence);
    }

    // 4. Regex fallback on full text
    try_regex(&text, &mut specs, &mut confidence);

    // Cap confidence for pure-HTML extraction
    specs.extraction_confidence = confidence.min(0.65);
    specs.source_url = String::new(); // caller sets this

    info!(
        "html_extractor: '{}' final confidence {:.2} nozzle={:?}/{:?} bed={:?}/{:?}",
        filament_name,
        specs.extraction_confidence,
        specs.nozzle_temp_min,
        specs.nozzle_temp_max,
        specs.bed_temp_min,
        specs.bed_temp_max,
    );

    specs
}

// ─── JSON-LD ────────────────────────────────────────────────────────────────

fn try_json_ld(document: &Html, specs: &mut FilamentSpecs, confidence: &mut f32) -> bool {
    let script_sel = match Selector::parse("script[type='application/ld+json']") {
        Ok(s) => s,
        Err(_) => return false,
    };

    for el in document.select(&script_sel) {
        let json_text = el.text().collect::<String>();
        if let Ok(val) = serde_json::from_str::<Value>(&json_text) {
            let found = extract_from_json_ld_value(&val, specs, confidence);
            if found {
                return true;
            }
        }
    }
    false
}

fn extract_from_json_ld_value(
    val: &Value,
    specs: &mut FilamentSpecs,
    confidence: &mut f32,
) -> bool {
    // Handle arrays at the top level
    if let Some(arr) = val.as_array() {
        for item in arr {
            if extract_from_json_ld_value(item, specs, confidence) {
                return true;
            }
        }
        return false;
    }

    // Look for additionalProperty or description fields with temperature info
    let mut found = false;

    // Try to get serial from product name in JSON-LD
    if specs.serial.is_empty() {
        if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
            if !name.is_empty() {
                specs.serial = infer_serial(name);
            }
        }
    }

    // additionalProperty array: [{name:"Nozzle Temperature", value:"200-230°C"}, ...]
    if let Some(props) = val.get("additionalProperty").and_then(|v| v.as_array()) {
        for prop in props {
            let prop_name = prop
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let prop_value = prop
                .get("value")
                .and_then(|v| v.as_str())
                .or_else(|| prop.get("unitText").and_then(|v| v.as_str()))
                .unwrap_or("");

            if prop_name.contains("nozzle")
                || prop_name.contains("print")
                || prop_name.contains("extru")
            {
                if let Some((lo, hi)) = parse_temp_range(prop_value) {
                    specs.nozzle_temp_min = Some(lo);
                    specs.nozzle_temp_max = Some(hi);
                    specs.nozzle_temperature = Some((lo + hi) / 2);
                    *confidence += 0.35;
                    found = true;
                }
            } else if prop_name.contains("bed")
                || prop_name.contains("build plate")
                || prop_name.contains("platform")
            {
                if let Some((lo, hi)) = parse_temp_range(prop_value) {
                    specs.bed_temp_min = Some(lo);
                    specs.bed_temp_max = Some(hi);
                    let mid = (lo + hi) / 2;
                    specs.hot_plate_temp = Some(mid);
                    specs.textured_plate_temp = Some(mid);
                    *confidence += 0.15;
                    found = true;
                }
            } else if prop_name.contains("material") || prop_name.contains("type") {
                if !prop_value.is_empty() && specs.material.is_empty() {
                    specs.material = prop_value.to_string();
                }
            } else if prop_name.contains("diameter") || prop_name.contains("diam") {
                if let Ok(d) = prop_value.trim_end_matches("mm").trim().parse::<f32>() {
                    specs.diameter_mm = Some(d);
                }
            } else if prop_name.contains("densit") {
                if let Ok(d) = prop_value
                    .trim_end_matches("g/cm³")
                    .trim_end_matches("g/cm3")
                    .trim()
                    .parse::<f32>()
                {
                    specs.density_g_cm3 = Some(d);
                }
            }
        }
    }

    // Recurse into @graph
    if let Some(graph) = val.get("@graph").and_then(|v| v.as_array()) {
        for item in graph {
            found |= extract_from_json_ld_value(item, specs, confidence);
        }
    }

    found
}

// ─── Table extraction ────────────────────────────────────────────────────────

fn try_tables(document: &Html, specs: &mut FilamentSpecs, confidence: &mut f32) {
    let row_sel = Selector::parse("tr").unwrap();
    let cell_sel = Selector::parse("td, th").unwrap();

    for row in document.select(&row_sel) {
        let cells: Vec<String> = row
            .select(&cell_sel)
            .map(|c| c.text().collect::<String>().trim().to_string())
            .collect();

        if cells.len() < 2 {
            continue;
        }

        let label = cells[0].to_lowercase();
        let value = &cells[1];

        apply_label_value(&label, value, specs, confidence);

        // Some tables put label in col 0 and value in col 2
        if cells.len() >= 3 {
            apply_label_value(&label, &cells[2], specs, confidence);
        }
    }
}

// ─── Definition lists ───────────────────────────────────────────────────────

fn try_definition_lists(document: &Html, specs: &mut FilamentSpecs, confidence: &mut f32) {
    let dt_sel = Selector::parse("dt").unwrap();
    let dd_sel = Selector::parse("dd").unwrap();

    let dts: Vec<String> = document
        .select(&dt_sel)
        .map(|el| el.text().collect::<String>().trim().to_lowercase())
        .collect();
    let dds: Vec<String> = document
        .select(&dd_sel)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .collect();

    for (label, value) in dts.iter().zip(dds.iter()) {
        apply_label_value(label, value, specs, confidence);
    }
}

// ─── Shared label→field mapping ─────────────────────────────────────────────

fn apply_label_value(label: &str, value: &str, specs: &mut FilamentSpecs, confidence: &mut f32) {
    let is_nozzle = label.contains("nozzle")
        || label.contains("print temp")
        || label.contains("extrusion")
        || label.contains("hotend");
    let is_bed = label.contains("bed")
        || label.contains("build plate")
        || label.contains("heated")
        || label.contains("platform");
    let is_density = label.contains("densit");
    let is_diameter = label.contains("diam");

    if is_nozzle && specs.nozzle_temp_min.is_none() {
        if let Some((lo, hi)) = parse_temp_range(value) {
            specs.nozzle_temp_min = Some(lo);
            specs.nozzle_temp_max = Some(hi);
            specs.nozzle_temperature = Some((lo + hi) / 2);
            *confidence += 0.20;
        }
    } else if is_bed && specs.bed_temp_min.is_none() {
        if let Some((lo, hi)) = parse_temp_range(value) {
            specs.bed_temp_min = Some(lo);
            specs.bed_temp_max = Some(hi);
            let mid = (lo + hi) / 2;
            specs.hot_plate_temp = Some(mid);
            specs.textured_plate_temp = Some(mid);
            *confidence += 0.10;
        }
    } else if is_density && specs.density_g_cm3.is_none() {
        if let Ok(d) = value
            .trim_end_matches("g/cm³")
            .trim_end_matches("g/cm3")
            .trim()
            .parse::<f32>()
        {
            specs.density_g_cm3 = Some(d);
        }
    } else if is_diameter && specs.diameter_mm.is_none() {
        if let Ok(d) = value.trim_end_matches("mm").trim().parse::<f32>() {
            specs.diameter_mm = Some(d);
        }
    }
}

// ─── Regex fallback ──────────────────────────────────────────────────────────

fn try_regex(text: &str, specs: &mut FilamentSpecs, confidence: &mut f32) {
    // Nozzle range
    if specs.nozzle_temp_min.is_none() {
        if let Some(cap) = NOZZLE_RANGE.captures(text) {
            if let Some((lo, hi)) = parse_range(&cap, 1, 2) {
                if lo >= 140 && hi <= 340 {
                    specs.nozzle_temp_min = Some(lo);
                    specs.nozzle_temp_max = Some(hi);
                    specs.nozzle_temperature = Some((lo + hi) / 2);
                    *confidence += 0.15;
                }
            }
        }
    }

    // Nozzle single fallback
    if specs.nozzle_temp_min.is_none() {
        if let Some(cap) = NOZZLE_SINGLE.captures(text) {
            if let Some(t) = cap.get(1).and_then(|m| m.as_str().parse::<u16>().ok()) {
                if t >= 140 && t <= 340 {
                    specs.nozzle_temperature = Some(t);
                    specs.nozzle_temp_min = Some(t.saturating_sub(10));
                    specs.nozzle_temp_max = Some(t + 10);
                    *confidence += 0.10;
                }
            }
        }
    }

    // Bed range
    if specs.bed_temp_min.is_none() {
        if let Some(cap) = BED_RANGE.captures(text) {
            if let Some((lo, hi)) = parse_range(&cap, 1, 2) {
                if lo <= 130 && hi <= 130 {
                    specs.bed_temp_min = Some(lo);
                    specs.bed_temp_max = Some(hi);
                    let mid = (lo + hi) / 2;
                    specs.hot_plate_temp = Some(mid);
                    specs.textured_plate_temp = Some(mid);
                    *confidence += 0.08;
                }
            }
        }
    }

    // Bed single fallback
    if specs.bed_temp_min.is_none() {
        if let Some(cap) = BED_SINGLE.captures(text) {
            if let Some(t) = cap.get(1).and_then(|m| m.as_str().parse::<u16>().ok()) {
                if t <= 130 {
                    specs.hot_plate_temp = Some(t);
                    specs.textured_plate_temp = Some(t);
                    specs.bed_temp_min = Some(t.saturating_sub(5));
                    specs.bed_temp_max = Some(t + 5);
                    *confidence += 0.05;
                }
            }
        }
    }

    // Density
    if specs.density_g_cm3.is_none() {
        if let Some(cap) = DENSITY.captures(text) {
            if let Some(d) = cap.get(1).and_then(|m| m.as_str().parse::<f32>().ok()) {
                if d > 0.5 && d < 3.0 {
                    specs.density_g_cm3 = Some(d);
                }
            }
        }
    }

    // Diameter
    if specs.diameter_mm.is_none() {
        if let Some(cap) = DIAMETER.captures(text) {
            if let Some(d) = cap.get(1).and_then(|m| m.as_str().parse::<f32>().ok()) {
                if (d - 1.75_f32).abs() < 0.1 || (d - 2.85_f32).abs() < 0.1 {
                    specs.diameter_mm = Some(d);
                }
            }
        }
    }

    // If we haven't found a diameter, default to 1.75
    if specs.diameter_mm.is_none() {
        specs.diameter_mm = Some(1.75);
    }

    // Boost confidence slightly if both nozzle and bed found
    if specs.nozzle_temp_min.is_some() && specs.bed_temp_min.is_some() {
        *confidence += 0.05;
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Parse a temperature range from a string like "200-230°C", "200 to 230 °C", "210°C".
/// Returns (lo, hi). If single value, returns (val, val).
fn parse_temp_range(s: &str) -> Option<(u16, u16)> {
    // Range
    if let Some(cap) = BARE_RANGE.captures(s) {
        let a: u16 = cap.get(1)?.as_str().parse().ok()?;
        let b: u16 = cap.get(2)?.as_str().parse().ok()?;
        if a >= 20 && b <= 400 {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            return Some((lo, hi));
        }
    }

    // Single
    if let Some(cap) = BARE_SINGLE.captures(s) {
        let t: u16 = cap.get(1)?.as_str().parse().ok()?;
        if t >= 20 && t <= 400 {
            return Some((t, t));
        }
    }

    None
}

/// Derive the serial (product identifier) from a filament name by stripping brand and material.
///
/// Serial is the product-specific part after brand and material tokens:
/// - "Sunlu PLA High Flow" → "High Flow"
/// - "Bambu Lab PLA Basic" → "Basic"
/// - "eSUN PETG-CF"        → "Basic" (no distinct serial)
/// - "eSUN ePLA+ Silk"     → "Silk"
/// - "eSUN ePLA-Silk"      → "Silk"
///
/// Matching is word-based (splitting on whitespace) and case-insensitive.
/// Material keywords are checked longest-first to avoid "PLA" matching inside "PLA-CF".
/// The `@printer` suffix (e.g. "@Bambu Lab H2C 0.4 nozzle") is stripped before processing.
pub fn infer_serial(filament_name: &str) -> String {
    // Strip the "@printer" suffix that Bambu Studio appends to profile names.
    let name = match filament_name.find(" @") {
        Some(idx) => &filament_name[..idx],
        None => filament_name,
    };

    let words: Vec<&str> = name.split_whitespace().collect();
    // Ordered longest-first so compound materials match before their prefixes.
    // Includes eSUN "e"-prefixed variants (ePLA, ePLA+, ePETG-CF, …).
    let material_keywords: &[&str] = &[
        "EPETG-CF", "EPLA-CF", "EPLA+", "EPLA",
        "PETG-CF", "PLA-CF", "PC-ABS", "PA6-CF", "PA12",
        "PA6", "PETG", "PLA+", "PLA", "ABS", "ASA", "TPU", "TPE",
        "PA", "NYLON", "PC", "PVA", "HIPS",
    ];

    for (i, word) in words.iter().enumerate() {
        let upper = word.to_uppercase();
        let is_material = material_keywords.iter().any(|kw| {
            upper == *kw
                || (upper.starts_with(kw)
                    && upper[kw.len()..].starts_with(['-', '+', '_']))
        });
        if is_material {
            // If the word itself encodes a variant after a hyphen (e.g. "ePLA-Silk"),
            // extract that suffix — but only when no keyword exactly matches the whole
            // word (so "PETG-CF" is not incorrectly split into "PETG" + "CF").
            let exact_match = material_keywords.iter().any(|kw| upper == *kw);
            let variant_in_word = if exact_match {
                None
            } else {
                // Find the longest keyword that is a prefix of this word and is
                // followed by '-<variant>'.
                material_keywords.iter().find_map(|kw| {
                    if upper.starts_with(kw) && upper.len() > kw.len() {
                        let rest = &upper[kw.len()..];
                        if rest.starts_with('-') && rest.len() > 1 {
                            // e.g. "ePLA-Silk" with kw="EPLA" → variant = "Silk"
                            let variant = word[kw.len() + 1..].to_string();
                            if !variant.is_empty() {
                                return Some(variant);
                            }
                        }
                    }
                    None
                })
            };

            let rest_words = words[i + 1..].join(" ");
            let after = match (variant_in_word, rest_words.as_str()) {
                (Some(v), "") => v,
                (Some(v), rest) => format!("{} {}", v, rest),
                (None, rest) => rest.to_string(),
            };
            return if after.is_empty() { "Basic".to_string() } else { after };
        }
    }

    // No material keyword found — return everything after the first word (brand)
    let fallback = words.get(1..).map(|s| s.join(" ")).unwrap_or_default();
    if fallback.is_empty() { "Basic".to_string() } else { fallback }
}


/// Derive material type string from filament name.
fn infer_material(name: &str) -> String {
    let m = MaterialType::from_str(name);
    match m {
        MaterialType::PLA => "PLA".to_string(),
        MaterialType::PETG => "PETG".to_string(),
        MaterialType::ABS => "ABS".to_string(),
        MaterialType::ASA => "ASA".to_string(),
        MaterialType::TPU => "TPU".to_string(),
        MaterialType::Nylon => "PA".to_string(),
        MaterialType::PC => "PC".to_string(),
        MaterialType::PVA => "PVA".to_string(),
        MaterialType::HIPS => "HIPS".to_string(),
        MaterialType::Other(s) => s,
    }
}

/// Derive brand from the first word of the filament name.
fn infer_brand(name: &str) -> String {
    name.split_whitespace().next().unwrap_or("").to_string()
}

/// Quick HTML-to-plaintext: strip tags, collapse whitespace.
fn html_to_text_simple(html: &str) -> String {
    use scraper::Html;
    let doc = Html::parse_document(html);
    let text = doc.root_element().text().collect::<Vec<_>>().join(" ");
    // Collapse runs of whitespace
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_temp_range_dash() {
        assert_eq!(parse_temp_range("200-230°C"), Some((200, 230)));
    }

    #[test]
    fn test_parse_temp_range_to() {
        assert_eq!(parse_temp_range("200 to 230 °C"), Some((200, 230)));
    }

    #[test]
    fn test_parse_temp_range_single() {
        assert_eq!(parse_temp_range("210°C"), Some((210, 210)));
    }

    #[test]
    fn test_parse_temp_range_none() {
        assert_eq!(parse_temp_range("some random text"), None);
    }

    #[test]
    fn test_extract_regex_nozzle_range() {
        let html = "<html><body><p>Nozzle Temperature: 200-230°C, Bed: 55-65°C</p></body></html>";
        let specs = extract(html, "Test PLA");
        assert_eq!(specs.nozzle_temp_min, Some(200));
        assert_eq!(specs.nozzle_temp_max, Some(230));
        assert_eq!(specs.bed_temp_min, Some(55));
        assert_eq!(specs.bed_temp_max, Some(65));
        assert!(specs.extraction_confidence > 0.0);
    }

    #[test]
    fn test_infer_serial_strips_brand_and_material() {
        assert_eq!(infer_serial("Sunlu PLA High Flow"), "High Flow");
        assert_eq!(infer_serial("Bambu Lab PLA Basic"), "Basic");
        assert_eq!(infer_serial("eSUN PETG-CF"), "Basic");
        assert_eq!(infer_serial("Polymaker PolyLite PLA Pro"), "Pro");
        assert_eq!(infer_serial("SUNLU PLA+"), "Basic");
        assert_eq!(infer_serial("SUNLU PLA+ Matte"), "Matte");
        assert_eq!(infer_serial("Generic PLA"), "Basic");
    }

    #[test]
    fn test_infer_serial_silk_filaments() {
        // Standard silk filament names
        assert_eq!(infer_serial("eSUN PLA Silk"), "Silk");
        assert_eq!(infer_serial("eSUN PLA+ Silk"), "Silk");
        assert_eq!(infer_serial("Bambu Lab PLA Silk"), "Silk");
        assert_eq!(infer_serial("Bambu Lab PLA+ Silk"), "Silk");
        assert_eq!(infer_serial("Hatchbox PLA Silk"), "Silk");
        assert_eq!(infer_serial("Sunlu PLA+ Silk"), "Silk");

        // eSUN "ePLA+" / "ePLA-Silk" naming conventions
        assert_eq!(infer_serial("eSUN ePLA+ Silk"), "Silk");
        assert_eq!(infer_serial("eSUN ePLA-Silk"), "Silk");

        // @printer suffix must be stripped, not included in the serial
        assert_eq!(infer_serial("Bambu Lab PLA Silk @Bambu Lab H2C 0.4 nozzle"), "Silk");
        assert_eq!(infer_serial("eSUN PLA+ Silk @Bambu Lab A1 0.4 nozzle"), "Silk");
    }
}
