//! The four defect severity tiers (spec 2026-10-17 §4). Distinct in colour
//! and form: severe is filled accent, significant accent text, noticeable
//! warning text, minor secondary text. Blue stays for links.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeverityTier {
    Minor,
    Noticeable,
    Significant,
    Severe,
}

impl SeverityTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Minor => "Minor",
            Self::Noticeable => "Noticeable",
            Self::Significant => "Significant",
            Self::Severe => "Severe",
        }
    }

    /// `severity-<key>` class and `data-tier`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Noticeable => "noticeable",
            Self::Significant => "significant",
            Self::Severe => "severe",
        }
    }
}

/// The tier thresholds: 0.3, 0.5 and 0.7.
pub fn severity_tier(severity: f32) -> SeverityTier {
    if severity < 0.3 {
        SeverityTier::Minor
    } else if severity < 0.5 {
        SeverityTier::Noticeable
    } else if severity < 0.7 {
        SeverityTier::Significant
    } else {
        SeverityTier::Severe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_split_at_the_existing_thresholds() {
        assert_eq!(severity_tier(0.0), SeverityTier::Minor);
        assert_eq!(severity_tier(0.29), SeverityTier::Minor);
        assert_eq!(severity_tier(0.3), SeverityTier::Noticeable);
        assert_eq!(severity_tier(0.49), SeverityTier::Noticeable);
        assert_eq!(severity_tier(0.5), SeverityTier::Significant);
        assert_eq!(severity_tier(0.69), SeverityTier::Significant);
        assert_eq!(severity_tier(0.7), SeverityTier::Severe);
        assert_eq!(severity_tier(1.0), SeverityTier::Severe);
    }

    #[test]
    fn every_tier_has_its_own_key_and_label() {
        let tiers = [
            SeverityTier::Minor,
            SeverityTier::Noticeable,
            SeverityTier::Significant,
            SeverityTier::Severe,
        ];
        assert_eq!(
            tiers.map(SeverityTier::key),
            ["minor", "noticeable", "significant", "severe"]
        );
        assert_eq!(
            tiers.map(SeverityTier::label),
            ["Minor", "Noticeable", "Significant", "Severe"]
        );
    }
}
