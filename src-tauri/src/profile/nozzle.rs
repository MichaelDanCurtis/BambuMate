//! Nozzle-size-aware limits for generated filament profiles.
//!
//! Bambu Studio filament profiles are written per printer *and* nozzle
//! (`Bambu Lab X1 Carbon 0.2 nozzle`), but the scraped/AI specs we start from
//! describe the *filament*, not the hot end. Manufacturer datasheets and AI
//! knowledge almost always quote numbers for a standard 0.4 mm nozzle.
//!
//! Volumetric flow is the setting where that mismatch actually breaks prints:
//! a 0.2 mm nozzle physically cannot push the ~21 mm³/s a 0.4 mm profile asks
//! for, and Bambu Studio will happily accept the value and then under-extrude
//! or clog. This module clamps the generated profile to a physically sane
//! ceiling for the nozzle the profile is being written for.

use super::types::FilamentProfile;

/// Bambu Studio profile key holding the volumetric flow ceiling (mm³/s).
const MAX_VOLUMETRIC_SPEED_KEY: &str = "filament_max_volumetric_speed";

/// Practical maximum volumetric flow (mm³/s) per nozzle diameter (mm).
///
/// These are ceilings, not targets: a value below the ceiling is always left
/// alone. They intentionally sit at the optimistic end of what a well-tuned
/// hot end achieves, so high-flow filaments are not needlessly throttled while
/// physically impossible values still get clamped.
///
/// The 0.2 mm entry is the important one — small nozzles are limited by melt
/// pressure far more than by cross-sectional area, so the practical ceiling is
/// around 2 mm³/s regardless of how "high flow" the filament claims to be.
///
/// Must stay sorted by ascending diameter; `max_volumetric_speed_cap`
/// interpolates between entries.
pub const NOZZLE_FLOW_CAPS: &[(f32, f32)] = &[
    (0.2, 2.0),
    (0.25, 3.5),
    (0.4, 32.0),
    (0.6, 45.0),
    (0.8, 65.0),
];

/// Absolute floor for any computed cap, so extrapolation can never produce a
/// value that makes a profile unusable.
const MIN_FLOW_CAP: f32 = 0.5;

/// A single setting that was changed to respect the nozzle's physical limits.
#[derive(Debug, Clone, PartialEq)]
pub struct NozzleAdjustment {
    /// Bambu Studio profile key that was clamped.
    pub field: String,
    /// Value before clamping.
    pub from: String,
    /// Value after clamping.
    pub to: String,
    /// Human-readable explanation for the UI.
    pub reason: String,
}

/// Parse the nozzle diameter out of a Bambu target printer label.
///
/// Labels follow the form `"Bambu Lab <model> <diameter> nozzle"`, e.g.
/// `"Bambu Lab X1 Carbon 0.2 nozzle"` -> `Some(0.2)`.
/// Returns `None` when the label does not carry a parseable diameter.
pub fn parse_nozzle_diameter(target_printer_label: &str) -> Option<f32> {
    let trimmed = target_printer_label.trim();
    let without_suffix = trimmed.strip_suffix("nozzle").unwrap_or(trimmed).trim_end();
    let token = without_suffix.rsplit(' ').next()?;
    let diameter: f32 = token.parse().ok()?;
    if diameter.is_finite() && diameter > 0.0 {
        Some(diameter)
    } else {
        None
    }
}

/// Return the maximum sensible volumetric flow (mm³/s) for a nozzle diameter.
///
/// Known diameters come straight from [`NOZZLE_FLOW_CAPS`]. Diameters between
/// two table entries are linearly interpolated. Diameters outside the table are
/// extrapolated from the nearest entry by area ratio, which keeps the result
/// monotonic in diameter.
pub fn max_volumetric_speed_cap(nozzle_diameter: f32) -> f32 {
    debug_assert!(!NOZZLE_FLOW_CAPS.is_empty());

    let (first_d, first_cap) = NOZZLE_FLOW_CAPS[0];
    let (last_d, last_cap) = NOZZLE_FLOW_CAPS[NOZZLE_FLOW_CAPS.len() - 1];

    let cap = if nozzle_diameter <= first_d {
        // Below the table: scale by area so smaller nozzles get lower ceilings.
        first_cap * (nozzle_diameter / first_d).powi(2)
    } else if nozzle_diameter >= last_d {
        last_cap * (nozzle_diameter / last_d).powi(2)
    } else {
        let mut interpolated = last_cap;
        for window in NOZZLE_FLOW_CAPS.windows(2) {
            let (low_d, low_cap) = window[0];
            let (high_d, high_cap) = window[1];
            if nozzle_diameter >= low_d && nozzle_diameter <= high_d {
                let t = (nozzle_diameter - low_d) / (high_d - low_d);
                interpolated = low_cap + t * (high_cap - low_cap);
                break;
            }
        }
        interpolated
    };

    cap.max(MIN_FLOW_CAP)
}

/// Clamp a volumetric flow value to the ceiling for `nozzle_diameter`.
///
/// Returns the value unchanged when it is already within limits.
pub fn clamp_volumetric_speed(value: f32, nozzle_diameter: f32) -> f32 {
    let cap = max_volumetric_speed_cap(nozzle_diameter);
    if value > cap {
        cap
    } else {
        value
    }
}

/// Apply nozzle-size limits to a generated profile.
///
/// Reads the profile's current `filament_max_volumetric_speed` (which came from
/// the scraped specs, the AI, or the inherited base profile) and clamps it to
/// the ceiling for `nozzle_diameter`. Returns the adjustments that were made so
/// the caller can surface them to the user.
pub fn apply_nozzle_limits(
    profile: &mut FilamentProfile,
    nozzle_diameter: f32,
) -> Vec<NozzleAdjustment> {
    let mut adjustments = Vec::new();
    let cap = max_volumetric_speed_cap(nozzle_diameter);

    let current = profile
        .get_first_array_value(MAX_VOLUMETRIC_SPEED_KEY)
        .and_then(|s| s.trim().parse::<f32>().ok());

    if let Some(value) = current {
        if value > cap {
            let clamped = format_flow(cap);
            profile.set_string_array(
                MAX_VOLUMETRIC_SPEED_KEY,
                vec![clamped.clone(), clamped.clone()],
            );
            adjustments.push(NozzleAdjustment {
                field: MAX_VOLUMETRIC_SPEED_KEY.to_string(),
                from: format_flow(value),
                to: clamped,
                reason: format!(
                    "A {:.1} mm nozzle cannot sustain {} mm³/s; capped to {} mm³/s.",
                    nozzle_diameter,
                    format_flow(value),
                    format_flow(cap)
                ),
            });
        }
    }

    adjustments
}

/// Format a flow value the way Bambu Studio writes it: whole numbers stay
/// integral, fractional caps keep one decimal.
fn format_flow(value: f32) -> String {
    if (value - value.round()).abs() < f32::EPSILON {
        format!("{:.0}", value)
    } else {
        format!("{:.1}", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile_from(raw: serde_json::Value) -> FilamentProfile {
        FilamentProfile::from_map(raw.as_object().expect("object").clone())
    }

    fn profile_with_flow(value: &str) -> FilamentProfile {
        profile_from(json!({
            "name": "Test PLA",
            "filament_max_volumetric_speed": [value, value],
        }))
    }

    #[test]
    fn parses_nozzle_diameter_from_label() {
        assert_eq!(
            parse_nozzle_diameter("Bambu Lab X1 Carbon 0.2 nozzle"),
            Some(0.2)
        );
        assert_eq!(parse_nozzle_diameter("Bambu Lab H2C 0.4 nozzle"), Some(0.4));
        assert_eq!(
            parse_nozzle_diameter("Bambu Lab A1 mini 0.8 nozzle"),
            Some(0.8)
        );
    }

    #[test]
    fn rejects_labels_without_diameter() {
        assert_eq!(parse_nozzle_diameter("Bambu Lab X1 Carbon"), None);
        assert_eq!(parse_nozzle_diameter(""), None);
        assert_eq!(parse_nozzle_diameter("nozzle"), None);
    }

    #[test]
    fn point_two_nozzle_caps_flow_at_two() {
        assert_eq!(max_volumetric_speed_cap(0.2), 2.0);
        assert_eq!(clamp_volumetric_speed(21.0, 0.2), 2.0);
        // Already-low values are untouched.
        assert_eq!(clamp_volumetric_speed(1.5, 0.2), 1.5);
    }

    #[test]
    fn caps_increase_with_nozzle_diameter() {
        let caps: Vec<f32> = [0.2, 0.25, 0.4, 0.6, 0.8]
            .iter()
            .map(|d| max_volumetric_speed_cap(*d))
            .collect();
        for pair in caps.windows(2) {
            assert!(
                pair[1] > pair[0],
                "cap should grow with diameter, got {:?}",
                caps
            );
        }
    }

    #[test]
    fn interpolates_unknown_diameters() {
        let cap = max_volumetric_speed_cap(0.5);
        assert!(cap > 32.0 && cap < 45.0, "unexpected 0.5mm cap: {}", cap);
    }

    #[test]
    fn extrapolates_outside_the_table() {
        // Smaller than the smallest table entry -> lower than its cap.
        assert!(max_volumetric_speed_cap(0.15) < 2.0);
        assert!(max_volumetric_speed_cap(0.15) >= MIN_FLOW_CAP);
        // Larger than the largest table entry -> higher than its cap.
        assert!(max_volumetric_speed_cap(1.0) > 65.0);
    }

    #[test]
    fn does_not_raise_conservative_profiles() {
        // A 0.4mm profile at 21 mm³/s stays at 21 — we only ever clamp down.
        let mut profile = profile_with_flow("21");
        let adjustments = apply_nozzle_limits(&mut profile, 0.4);
        assert!(adjustments.is_empty());
        assert_eq!(
            profile.get_string_array(MAX_VOLUMETRIC_SPEED_KEY),
            Some(vec!["21", "21"])
        );
    }

    #[test]
    fn clamps_profile_for_small_nozzle() {
        let mut profile = profile_with_flow("21");
        let adjustments = apply_nozzle_limits(&mut profile, 0.2);
        assert_eq!(adjustments.len(), 1);
        assert_eq!(adjustments[0].field, MAX_VOLUMETRIC_SPEED_KEY);
        assert_eq!(adjustments[0].from, "21");
        assert_eq!(adjustments[0].to, "2");
        assert_eq!(
            profile.get_string_array(MAX_VOLUMETRIC_SPEED_KEY),
            Some(vec!["2", "2"])
        );
    }

    #[test]
    fn ignores_profiles_without_a_flow_value() {
        let mut profile = profile_from(json!({ "name": "Test PLA" }));
        assert!(apply_nozzle_limits(&mut profile, 0.2).is_empty());
    }
}
