//! What Compare shows (spec 2026-10-17 §4). Compare always fetches every
//! field and filters here, so "Changed only" works without comparing again.

pub use crate::commands::{CompareProfileDiff, DiffCategory};

/// The hero's unit for a count of differences.
pub fn differences_unit(n: usize) -> &'static str {
    if n == 1 {
        " difference"
    } else {
        " differences"
    }
}

pub fn is_changed(d: &CompareProfileDiff) -> bool {
    d.base_value != d.new_value
}

/// The categories to render: with `changed_only`, identical rows go, and so
/// does any category left empty.
pub fn visible_categories(categories: &[DiffCategory], changed_only: bool) -> Vec<DiffCategory> {
    categories
        .iter()
        .map(|c| DiffCategory {
            category: c.category.clone(),
            diffs: c
                .diffs
                .iter()
                .filter(|d| !changed_only || is_changed(d))
                .cloned()
                .collect(),
        })
        .filter(|c| !c.diffs.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(label: &str, a: &str, b: &str) -> CompareProfileDiff {
        CompareProfileDiff {
            key: label.to_lowercase(),
            label: label.into(),
            base_value: a.into(),
            new_value: b.into(),
        }
    }

    fn cats() -> Vec<DiffCategory> {
        vec![
            DiffCategory {
                category: "Temperature".into(),
                diffs: vec![diff("Nozzle", "220", "215"), diff("Bed", "60", "60")],
            },
            DiffCategory {
                category: "Cooling".into(),
                diffs: vec![diff("Fan", "100", "100")],
            },
        ]
    }

    #[test]
    fn the_difference_unit_is_singular_for_one() {
        assert_eq!(differences_unit(1), " difference");
        assert_eq!(differences_unit(0), " differences");
        assert_eq!(differences_unit(7), " differences");
    }

    #[test]
    fn changed_only_drops_identical_rows_and_empty_categories() {
        let shown = visible_categories(&cats(), true);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].category, "Temperature");
        assert_eq!(shown[0].diffs.len(), 1);
        assert_eq!(shown[0].diffs[0].label, "Nozzle");
    }

    #[test]
    fn showing_everything_keeps_every_row_in_order() {
        let shown = visible_categories(&cats(), false);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].diffs.len(), 2);
        assert!(is_changed(&shown[0].diffs[0]));
        assert!(!is_changed(&shown[0].diffs[1]));
    }
}
