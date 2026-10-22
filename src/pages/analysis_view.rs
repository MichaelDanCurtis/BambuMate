//! What Analyze leads with (spec 2026-10-17 §4, amended 2026-10-22): the
//! overall quality word, toned; the analysis returns a word, not a score,
//! and no defect positions.

use crate::components::patterns::Tone;
pub use crate::pages::print_analysis::{DefectReport, DetectedDefect};

pub fn quality_tone(quality: &str) -> Tone {
    match quality.trim().to_ascii_lowercase().as_str() {
        "excellent" | "good" => Tone::Signal,
        "acceptable" | "fair" => Tone::Warning,
        "poor" | "failed" => Tone::Accent,
        _ => Tone::Neutral,
    }
}

pub fn quality_word(quality: &str) -> String {
    let w = quality.trim();
    if w.is_empty() {
        "—".into()
    } else {
        w.to_uppercase()
    }
}

/// Most severe first; ties keep their order.
pub fn defects_by_severity(defects: &[DetectedDefect]) -> Vec<DetectedDefect> {
    let mut sorted = defects.to_vec();
    sorted.sort_by(|a, b| {
        b.severity
            .partial_cmp(&a.severity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

/// `2 defects · PLA`, `1 defect`, `No defects · PETG`.
pub fn analysis_detail(report: &DefectReport, material: &str) -> String {
    let count = match report.defects.len() {
        0 => "No defects".to_string(),
        1 => "1 defect".to_string(),
        n => format!("{n} defects"),
    };
    let material = material.trim();
    if material.is_empty() {
        count
    } else {
        format!("{count} · {material}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defect(t: &str, s: f32) -> DetectedDefect {
        DetectedDefect {
            defect_type: t.into(),
            severity: s,
            confidence: 0.9,
        }
    }

    #[test]
    fn quality_words_are_toned() {
        assert_eq!(quality_tone("excellent"), Tone::Signal);
        assert_eq!(quality_tone("good"), Tone::Signal);
        assert_eq!(quality_tone("acceptable"), Tone::Warning);
        assert_eq!(quality_tone("fair"), Tone::Warning);
        assert_eq!(quality_tone("poor"), Tone::Accent);
        assert_eq!(quality_tone("failed"), Tone::Accent);
        assert_eq!(quality_tone("whatever"), Tone::Neutral);
        assert_eq!(quality_word(" fair "), "FAIR");
        assert_eq!(quality_word(""), "—");
    }

    #[test]
    fn defects_are_ordered_most_severe_first() {
        let sorted = defects_by_severity(&[defect("a", 0.35), defect("b", 0.65), defect("c", 0.1)]);
        let order: Vec<&str> = sorted.iter().map(|d| d.defect_type.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
    }

    #[test]
    fn the_detail_counts_defects_and_names_the_material() {
        let report = DefectReport {
            defects: vec![defect("a", 0.5), defect("b", 0.2)],
            overall_quality: "fair".into(),
            notes: None,
        };
        assert_eq!(analysis_detail(&report, "PLA"), "2 defects · PLA");
        let one = DefectReport {
            defects: vec![defect("a", 0.5)],
            ..report.clone()
        };
        assert_eq!(analysis_detail(&one, ""), "1 defect");
        let none = DefectReport {
            defects: vec![],
            ..report
        };
        assert_eq!(analysis_detail(&none, "PETG"), "No defects · PETG");
    }
}
