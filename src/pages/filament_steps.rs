//! Where Filament Search is in its flow, and what its hero shows (spec
//! 2026-10-17 §4, amended 2026-10-20: a step strip instead of collapsible
//! sections). Pure, so it is tested without a browser.

use crate::commands::FilamentSpecs;

/// The steps the strip shows, in order.
pub const SEARCH_STEPS: &[&str] = &["Specs", "Tune", "Install"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStep {
    /// No filament picked yet: the search box is the hero.
    Search,
    /// The filament card and the base-profile choice.
    Specs,
    /// The optional merge, then the editor.
    Tune,
    /// The preview, then the install result.
    Install,
}

pub fn search_step(
    has_specs: bool,
    merging: bool,
    editing: bool,
    has_results: bool,
    has_installs: bool,
) -> SearchStep {
    if !has_specs {
        SearchStep::Search
    } else if has_results || has_installs {
        SearchStep::Install
    } else if merging || editing {
        SearchStep::Tune
    } else {
        SearchStep::Specs
    }
}

/// The step's position in `SEARCH_STEPS`; Search sits before the strip and
/// reads as the first step.
pub fn step_index(step: SearchStep) -> usize {
    match step {
        SearchStep::Search | SearchStep::Specs => 0,
        SearchStep::Tune => 1,
        SearchStep::Install => 2,
    }
}

/// `Polymaker · PLA`, leaving out what's blank.
pub fn spec_label(s: &FilamentSpecs) -> String {
    [s.brand.trim(), s.material.trim()]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Nozzle, bed and flow for the hero: the printing values when known, else
/// the top of the range, else a dash.
pub fn spec_stats(s: &FilamentSpecs) -> Vec<(String, String)> {
    let temp = |v: Option<u16>| v.map(|t| format!("{t}°")).unwrap_or_else(|| "—".into());
    vec![
        (
            "Nozzle".into(),
            temp(s.nozzle_temperature.or(s.nozzle_temp_max)),
        ),
        ("Bed".into(), temp(s.hot_plate_temp.or(s.bed_temp_max))),
        (
            "Flow".into(),
            s.filament_flow_ratio
                .map(|f| format!("{f:.2}"))
                .unwrap_or_else(|| "—".into()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(json: serde_json::Value) -> FilamentSpecs {
        let mut base = serde_json::json!({
            "serial": "", "brand": "Polymaker", "material": "PLA",
            "source_url": "", "extraction_confidence": 0.9
        });
        base.as_object_mut()
            .unwrap()
            .extend(json.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn the_step_follows_the_flow() {
        assert_eq!(
            search_step(false, false, false, false, false),
            SearchStep::Search
        );
        assert_eq!(
            search_step(true, false, false, false, false),
            SearchStep::Specs
        );
        assert_eq!(
            search_step(true, true, false, false, false),
            SearchStep::Tune
        );
        assert_eq!(
            search_step(true, false, true, false, false),
            SearchStep::Tune
        );
        assert_eq!(
            search_step(true, false, true, true, false),
            SearchStep::Install
        );
        assert_eq!(
            search_step(true, false, false, false, true),
            SearchStep::Install
        );
    }

    #[test]
    fn step_indexes_match_the_strip() {
        assert_eq!(SEARCH_STEPS, &["Specs", "Tune", "Install"]);
        assert_eq!(step_index(SearchStep::Specs), 0);
        assert_eq!(step_index(SearchStep::Tune), 1);
        assert_eq!(step_index(SearchStep::Install), 2);
        assert_eq!(step_index(SearchStep::Search), 0);
    }

    #[test]
    fn hero_stats_prefer_the_printing_values() {
        let s = specs(serde_json::json!({
            "nozzle_temperature": 220, "nozzle_temp_max": 230,
            "hot_plate_temp": 65, "bed_temp_max": 70, "filament_flow_ratio": 0.98
        }));
        assert_eq!(spec_label(&s), "Polymaker · PLA");
        assert_eq!(
            spec_stats(&s),
            vec![
                ("Nozzle".to_string(), "220°".to_string()),
                ("Bed".to_string(), "65°".to_string()),
                ("Flow".to_string(), "0.98".to_string()),
            ]
        );
    }

    #[test]
    fn hero_stats_fall_back_to_the_ranges_then_a_dash() {
        let s = specs(serde_json::json!({ "nozzle_temp_max": 230, "bed_temp_max": 70 }));
        assert_eq!(
            spec_stats(&s),
            vec![
                ("Nozzle".to_string(), "230°".to_string()),
                ("Bed".to_string(), "70°".to_string()),
                ("Flow".to_string(), "—".to_string()),
            ]
        );
        let unbranded = specs(serde_json::json!({ "brand": "" }));
        assert_eq!(spec_label(&unbranded), "PLA");
    }
}
