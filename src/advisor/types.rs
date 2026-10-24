//! Serde mirrors of the backend's `Advice` (src-tauri/src/advisor/advice.rs).

use serde::{Deserialize, Serialize};

use crate::commands::CatalogEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

/// Chip order, the same as the backend's.
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
    pub fn label(self) -> &'static str {
        match self {
            Need::Strong => "Strong",
            Need::Stiff => "Stiff",
            Need::Flexible => "Flexible",
            Need::ImpactTough => "Impact-tough",
            Need::Heat60 => "Heat ≥60 °C",
            Need::Heat80 => "Heat ≥80 °C",
            Need::Heat100 => "Heat ≥100 °C",
            Need::OutdoorUv => "Outdoor/UV",
            Need::Chemical => "Chemical/fuel",
            Need::FoodContact => "Food-contact-adjacent",
            Need::EasyPrint => "Easy print",
            Need::FineDetail => "Fine detail",
            Need::LowCost => "Low cost",
        }
    }

    pub fn is_heat(self) -> bool {
        matches!(self, Need::Heat60 | Need::Heat80 | Need::Heat100)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Met,
    Partly,
    NotMet,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NeedReason {
    pub need: Need,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductStatus {
    Loaded,
    Preferred,
    Catalog,
}

impl ProductStatus {
    /// `data-status` on the product row.
    pub fn key(self) -> &'static str {
        match self {
            ProductStatus::Loaded => "loaded",
            ProductStatus::Preferred => "preferred",
            ProductStatus::Catalog => "catalog",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Product {
    pub name: String,
    pub brand: String,
    pub status: ProductStatus,
    pub slot: Option<String>,
    pub buy_url: Option<String>,
    pub entry: Option<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MaterialFit {
    /// The backend's `MaterialId` key, e.g. `pa_cf`.
    pub material: String,
    pub name: String,
    pub total: u32,
    pub fit_pct: u8,
    pub reasons: Vec<NeedReason>,
    pub printer_notes: Vec<String>,
    pub geometry_notes: Vec<String>,
    pub material_notes: Vec<String>,
    pub products: Vec<Product>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Excluded {
    pub material: String,
    pub name: String,
    /// Every reason it fails, in the backend's order: enclosure, nozzle, heat.
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Advice {
    pub needs: Vec<Need>,
    pub fits: Vec<MaterialFit>,
    pub ruled_out: Vec<Excluded>,
    pub printer_line: Option<String>,
    pub model_error: Option<String>,
    pub catalog_empty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn need_keys_match_the_backend() {
        let keys = serde_json::to_value(ALL_NEEDS).unwrap();
        assert_eq!(
            keys,
            serde_json::json!([
                "strong",
                "stiff",
                "flexible",
                "impact_tough",
                "heat_60",
                "heat_80",
                "heat_100",
                "outdoor_uv",
                "chemical",
                "food_contact",
                "easy_print",
                "fine_detail",
                "low_cost"
            ])
        );
        let heats: Vec<Need> = ALL_NEEDS.iter().copied().filter(|n| n.is_heat()).collect();
        assert_eq!(heats, [Need::Heat60, Need::Heat80, Need::Heat100]);
        assert_eq!(Need::FoodContact.label(), "Food-contact-adjacent");
    }

    #[test]
    fn a_backend_advice_deserializes() {
        let json = r#"{"needs":["heat_80","outdoor_uv"],
            "fits":[{"material":"asa","name":"ASA","total":6,"fit_pct":100,
                "reasons":[{"need":"heat_80","verdict":"met","detail":"heat deflection ≈95 °C"},
                           {"need":"outdoor_uv","verdict":"not_met","detail":"0/3"}],
                "printer_notes":[],"geometry_notes":["Supports likely, whatever the material"],"material_notes":[],
                "products":[{"name":"ASA","brand":"Bambu Lab","status":"loaded","slot":"A2","buy_url":null,"entry":null},
                            {"name":"ASA","brand":"Polymaker","status":"preferred","slot":null,
                             "buy_url":"https://www.spoolscout.com/data-sheets/polymaker/asa-asa",
                             "entry":{"brand":"Polymaker","name":"ASA","material":"ASA","url_slug":"asa-asa",
                                      "full_url":"https://www.spoolscout.com/data-sheets/polymaker/asa-asa"}}]}],
            "ruled_out":[{"material":"pc","name":"PC","reasons":["Needs an enclosed printer; the A1 is open.","Softens around 110 °C"]}],
            "printer_line":null,"model_error":null,"catalog_empty":false}"#;
        let a: Advice = serde_json::from_str(json).unwrap();
        assert_eq!(a.needs, [Need::Heat80, Need::OutdoorUv]);
        assert_eq!(a.fits[0].reasons[1].verdict, Verdict::NotMet);
        assert_eq!(a.fits[0].products[0].status, ProductStatus::Loaded);
        assert_eq!(
            a.fits[0].products[1].entry.as_ref().unwrap().brand,
            "Polymaker"
        );
        assert_eq!(a.ruled_out[0].material, "pc");
        assert_eq!(a.ruled_out[0].reasons.len(), 2);
        assert_eq!(ProductStatus::Preferred.key(), "preferred");
    }
}
