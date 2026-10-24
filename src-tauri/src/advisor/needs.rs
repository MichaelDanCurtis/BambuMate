//! What a part needs from its material: the chips on the Advise page.

use serde::{Deserialize, Serialize};

/// The declaration order is the order chips are shown and needs are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Need {
    #[serde(rename = "strong")]
    Strong,
    #[serde(rename = "stiff")]
    Stiff,
    #[serde(rename = "flexible")]
    Flexible,
    #[serde(rename = "impact_tough")]
    ImpactTough,
    #[serde(rename = "heat_60")]
    Heat60,
    #[serde(rename = "heat_80")]
    Heat80,
    #[serde(rename = "heat_100")]
    Heat100,
    #[serde(rename = "outdoor_uv")]
    OutdoorUv,
    #[serde(rename = "chemical")]
    Chemical,
    #[serde(rename = "food_contact")]
    FoodContact,
    #[serde(rename = "easy_print")]
    EasyPrint,
    #[serde(rename = "fine_detail")]
    FineDetail,
    #[serde(rename = "low_cost")]
    LowCost,
}

pub const ALL_NEEDS: &[Need] = &[
    Need::Strong,
    Need::Stiff,
    Need::Flexible,
    Need::ImpactTough,
    Need::Heat60,
    Need::Heat80,
    Need::Heat100,
    Need::OutdoorUv,
    Need::Chemical,
    Need::FoodContact,
    Need::EasyPrint,
    Need::FineDetail,
    Need::LowCost,
];

impl Need {
    /// The wire key, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Need::Strong => "strong",
            Need::Stiff => "stiff",
            Need::Flexible => "flexible",
            Need::ImpactTough => "impact_tough",
            Need::Heat60 => "heat_60",
            Need::Heat80 => "heat_80",
            Need::Heat100 => "heat_100",
            Need::OutdoorUv => "outdoor_uv",
            Need::Chemical => "chemical",
            Need::FoodContact => "food_contact",
            Need::EasyPrint => "easy_print",
            Need::FineDetail => "fine_detail",
            Need::LowCost => "low_cost",
        }
    }

    /// A key from untrusted text, ignoring case and surrounding space.
    pub fn from_key(key: &str) -> Option<Need> {
        let key = key.trim().to_ascii_lowercase();
        ALL_NEEDS.iter().copied().find(|n| n.key() == key)
    }

    /// The temperature a heat need asks the part to survive, °C.
    pub fn heat_threshold(self) -> Option<u16> {
        match self {
            Need::Heat60 => Some(60),
            Need::Heat80 => Some(80),
            Need::Heat100 => Some(100),
            _ => None,
        }
    }

    /// Food contact is never scored: it only adds a caution.
    pub fn is_scored(self) -> bool {
        self != Need::FoodContact
    }
}

/// Deduplicated, in display order, with only the hottest heat need kept
/// (the heat options are one choice).
pub fn tidy(needs: &[Need]) -> Vec<Need> {
    let hottest = needs
        .iter()
        .copied()
        .filter(|n| n.heat_threshold().is_some())
        .max();
    let mut out: Vec<Need> = needs
        .iter()
        .copied()
        .filter(|n| n.heat_threshold().is_none() || Some(*n) == hottest)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// `tidy`, plus easy print when nothing scored was chosen: an empty set of
/// needs scores on easy print, the default.
pub fn with_default(needs: &[Need]) -> Vec<Need> {
    let mut out = tidy(needs);
    if !out.iter().any(|n| n.is_scored()) {
        out.push(Need::EasyPrint);
        out.sort();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_the_serde_names() {
        for n in ALL_NEEDS {
            assert_eq!(serde_json::to_value(n).unwrap(), serde_json::json!(n.key()));
            assert_eq!(
                serde_json::from_value::<Need>(serde_json::json!(n.key())).unwrap(),
                *n
            );
            assert_eq!(Need::from_key(n.key()), Some(*n));
        }
        assert_eq!(Need::from_key("  HEAT_80 "), Some(Need::Heat80));
        assert_eq!(Need::from_key("rocket_proof"), None);
    }

    #[test]
    fn tidy_keeps_the_hottest_heat_once_in_order() {
        let got = tidy(&[
            Need::OutdoorUv,
            Need::Heat60,
            Need::Strong,
            Need::Heat100,
            Need::Strong,
        ]);
        assert_eq!(got, [Need::Strong, Need::Heat100, Need::OutdoorUv]);
    }

    #[test]
    fn nothing_scored_means_easy_print() {
        assert_eq!(with_default(&[]), [Need::EasyPrint]);
        assert_eq!(
            with_default(&[Need::FoodContact]),
            [Need::FoodContact, Need::EasyPrint]
        );
        assert_eq!(with_default(&[Need::Stiff]), [Need::Stiff]);
    }
}
