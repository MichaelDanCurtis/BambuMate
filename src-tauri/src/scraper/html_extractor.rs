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
use tracing::info;

use super::types::{FilamentSpecs, MaterialType};

// ─── Regex patterns ────────────────────────────────────────────────────────

/// Range: "200-230°C", "200 ~ 230 °C", "200°C to 230°C", "200 to 230°C"
const NOZZLE_RANGE_RE: &str =
    r"(?i)(?:nozzle|print(?:ing)?|extrusion|hotend)[^\d]{0,30}?(\d{3})\s*[-–~to°]+\s*(\d{3})\s*°?\s*[Cc]";
/// Single nozzle temp: "Nozzle: 210°C"
const NOZZLE_SINGLE_RE: &str =
    r"(?i)(?:nozzle|print(?:ing)?|extrusion)[^\d]{0,20}?(\d{3})\s*°?\s*[Cc]";
/// Bed range: "Bed: 55-70°C", "Build Plate: 55–70 °C"
const BED_RANGE_RE: &str =
    r"(?i)(?:bed|build\s*plate|heated\s*bed|platform)[^\d]{0,30}?(\d{2,3})\s*[-–~to°]+\s*(\d{2,3})\s*°?\s*[Cc]";
/// Single bed temp
const BED_SINGLE_RE: &str =
    r"(?i)(?:bed|build\s*plate|heated\s*bed|platform)[^\d]{0,20}?(\d{2,3})\s*°?\s*[Cc]";
/// Density: "1.24 g/cm³" or "1.24 g/cm3"
const DENSITY_RE: &str = r"(\d+\.\d+)\s*g/cm[³3]";
/// Diameter: "1.75 mm" near diameter context
const DIAMETER_RE: &str = r"(?i)diam(?:eter)?[^\d]{0,10}?(\d\.\d+)\s*mm";

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
        name: filament_name.to_string(),
        ..Default::default()
    };

    // Derive material and brand from the filament name
    specs.material = infer_material(filament_name);
    specs.brand = infer_brand(filament_name);

    let mut confidence: f32 = 0.0;

    // 1. JSON-LD
    if try_json_ld(&document, &mut specs, &mut confidence) {
        info!("html_extractor: JSON-LD extraction contributed confidence {:.2}", confidence);
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

fn extract_from_json_ld_value(val: &Value, specs: &mut FilamentSpecs, confidence: &mut f32) -> bool {
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

    // Try to get name
    if specs.name.is_empty() || specs.name == "Unknown Filament" {
        if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
            if !name.is_empty() {
                specs.name = name.to_string();
            }
        }
    }

    // additionalProperty array: [{name:"Nozzle Temperature", value:"200-230°C"}, ...]
    if let Some(props) = val.get("additionalProperty").and_then(|v| v.as_array()) {
        for prop in props {
            let prop_name = prop.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let prop_value = prop.get("value").and_then(|v| v.as_str())
                .or_else(|| prop.get("unitText").and_then(|v| v.as_str()))
                .unwrap_or("");

            if prop_name.contains("nozzle") || prop_name.contains("print") || prop_name.contains("extru") {
                if let Some((lo, hi)) = parse_temp_range(prop_value) {
                    specs.nozzle_temp_min = Some(lo);
                    specs.nozzle_temp_max = Some(hi);
                    specs.nozzle_temperature = Some((lo + hi) / 2);
                    *confidence += 0.35;
                    found = true;
                }
            } else if prop_name.contains("bed") || prop_name.contains("build plate") || prop_name.contains("platform") {
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
                    .trim_end_matches("g/cm³").trim_end_matches("g/cm3").trim()
                    .parse::<f32>() {
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

    let dts: Vec<String> = document.select(&dt_sel)
        .map(|el| el.text().collect::<String>().trim().to_lowercase())
        .collect();
    let dds: Vec<String> = document.select(&dd_sel)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .collect();

    for (label, value) in dts.iter().zip(dds.iter()) {
        apply_label_value(label, value, specs, confidence);
    }
}

// ─── Shared label→field mapping ─────────────────────────────────────────────

fn apply_label_value(label: &str, value: &str, specs: &mut FilamentSpecs, confidence: &mut f32) {
    let is_nozzle = label.contains("nozzle") || label.contains("print temp")
        || label.contains("extrusion") || label.contains("hotend");
    let is_bed = label.contains("bed") || label.contains("build plate")
        || label.contains("heated") || label.contains("platform");
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
        if let Ok(d) = value.trim_end_matches("g/cm³").trim_end_matches("g/cm3").trim().parse::<f32>() {
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
    use regex::Regex;

    // Nozzle range
    if specs.nozzle_temp_min.is_none() {
        if let Ok(re) = Regex::new(NOZZLE_RANGE_RE) {
            if let Some(cap) = re.captures(text) {
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
    }

    // Nozzle single fallback
    if specs.nozzle_temp_min.is_none() {
        if let Ok(re) = Regex::new(NOZZLE_SINGLE_RE) {
            if let Some(cap) = re.captures(text) {
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
    }

    // Bed range
    if specs.bed_temp_min.is_none() {
        if let Ok(re) = Regex::new(BED_RANGE_RE) {
            if let Some(cap) = re.captures(text) {
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
    }

    // Bed single fallback
    if specs.bed_temp_min.is_none() {
        if let Ok(re) = Regex::new(BED_SINGLE_RE) {
            if let Some(cap) = re.captures(text) {
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
    }

    // Density
    if specs.density_g_cm3.is_none() {
        if let Ok(re) = Regex::new(DENSITY_RE) {
            if let Some(cap) = re.captures(text) {
                if let Some(d) = cap.get(1).and_then(|m| m.as_str().parse::<f32>().ok()) {
                    if d > 0.5 && d < 3.0 {
                        specs.density_g_cm3 = Some(d);
                    }
                }
            }
        }
    }

    // Diameter
    if specs.diameter_mm.is_none() {
        if let Ok(re) = Regex::new(DIAMETER_RE) {
            if let Some(cap) = re.captures(text) {
                if let Some(d) = cap.get(1).and_then(|m| m.as_str().parse::<f32>().ok()) {
                    if (d - 1.75_f32).abs() < 0.1 || (d - 2.85_f32).abs() < 0.1 {
                        specs.diameter_mm = Some(d);
                    }
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
    use regex::Regex;

    // Range
    let range_re = Regex::new(r"(\d{2,3})\s*[-–~to°]+\s*(\d{2,3})\s*°?\s*[Cc]?").ok()?;
    if let Some(cap) = range_re.captures(s) {
        let a: u16 = cap.get(1)?.as_str().parse().ok()?;
        let b: u16 = cap.get(2)?.as_str().parse().ok()?;
        if a >= 20 && b <= 400 {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            return Some((lo, hi));
        }
    }

    // Single
    let single_re = Regex::new(r"(\d{2,3})\s*°?\s*[Cc]").ok()?;
    if let Some(cap) = single_re.captures(s) {
        let t: u16 = cap.get(1)?.as_str().parse().ok()?;
        if t >= 20 && t <= 400 {
            return Some((t, t));
        }
    }

    None
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
    fn test_infer_material() {
        assert_eq!(infer_material("Polymaker PLA Pro"), "PLA");
        assert_eq!(infer_material("eSUN PETG-CF"), "PETG");
        assert_eq!(infer_material("Bambu Lab ABS"), "ABS");
    }
}
