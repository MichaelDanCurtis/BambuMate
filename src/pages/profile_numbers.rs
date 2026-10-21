//! A profile's headline numbers for the Library hero and stat rows (spec
//! 2026-10-17 §4). Bambu Studio stores most values as one-item string
//! arrays; a value the profile inherits from its parent is absent and shows
//! as a dash.

/// Shown for a value the profile doesn't set itself.
const MISSING: &str = "—";

#[derive(Debug, Clone, PartialEq)]
pub struct KeyNumbers {
    pub nozzle: String,
    pub bed: String,
    pub flow: String,
    pub pressure_advance: String,
    pub max_volumetric: String,
}

/// The first value of `key`, from a string, a number, or the first item of
/// an array of either; `None` when absent or blank.
fn first(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    let value = match obj.get(key)? {
        serde_json::Value::Array(items) => items.first()?.clone(),
        other => other.clone(),
    };
    let text = match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn key_numbers(raw_json: &str) -> KeyNumbers {
    let raw: serde_json::Value = serde_json::from_str(raw_json).unwrap_or_default();
    let empty = serde_json::Map::new();
    let obj = raw.as_object().unwrap_or(&empty);
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| MISSING.to_string());
    KeyNumbers {
        nozzle: or_dash(first(obj, "nozzle_temperature").map(|t| format!("{t}°"))),
        bed: or_dash(
            first(obj, "hot_plate_temp")
                .or_else(|| first(obj, "bed_temperature"))
                .map(|t| format!("{t}°")),
        ),
        flow: or_dash(first(obj, "filament_flow_ratio")),
        pressure_advance: or_dash(first(obj, "pressure_advance")),
        max_volumetric: or_dash(
            first(obj, "filament_max_volumetric_speed").map(|v| format!("{v} mm³/s")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headline_numbers_come_from_the_first_array_value() {
        let n = key_numbers(
            r#"{"nozzle_temperature":["220","225"],"hot_plate_temp":["65"],
                "filament_flow_ratio":["0.98"],"pressure_advance":["0.04"],
                "filament_max_volumetric_speed":["12"]}"#,
        );
        assert_eq!(n.nozzle, "220°");
        assert_eq!(n.bed, "65°");
        assert_eq!(n.flow, "0.98");
        assert_eq!(n.pressure_advance, "0.04");
        assert_eq!(n.max_volumetric, "12 mm³/s");
    }

    #[test]
    fn missing_or_blank_values_show_a_dash() {
        let n = key_numbers(r#"{"nozzle_temperature":[""],"bed_temperature":"60"}"#);
        assert_eq!(n.nozzle, "—");
        // Falls back to bed_temperature when there is no hot_plate_temp.
        assert_eq!(n.bed, "60°");
        assert_eq!(n.flow, "—");
        assert_eq!(key_numbers("not json").nozzle, "—");
    }

    #[test]
    fn numbers_stored_as_numbers_are_read_too() {
        let n = key_numbers(r#"{"filament_flow_ratio":0.95,"nozzle_temperature":[215]}"#);
        assert_eq!(n.flow, "0.95");
        assert_eq!(n.nozzle, "215°");
    }
}
