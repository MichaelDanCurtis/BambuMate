//! The curated material table. Pure data.
//!
//! These are general guidance values for each material family, taken from
//! widely published datasheet ranges and common practice. They are not
//! per-product data: one brand's PETG can differ from another's, and the
//! catalog has no per-product properties. Scores run from 0 (poor) to
//! 3 (excellent); `cheapness` is 3 for the least expensive. Heat deflection
//! temperatures are approximate, in °C. `warp` is how prone the material is
//! to warping, 0 to 3.

use serde::{Deserialize, Serialize};

use crate::scraper::types::MaterialType;

/// Every material the advisor knows. The declaration order is the fixed
/// tie-break order when two materials score the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialId {
    Pla,
    PlaPlus,
    PlaCf,
    Petg,
    PetCf,
    Abs,
    Asa,
    Tpu,
    Pa,
    PaCf,
    Pc,
}

/// How well a material does at each property, 0 to 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Properties {
    pub strength: u8,
    pub stiffness: u8,
    pub flexibility: u8,
    pub impact: u8,
    pub uv: u8,
    pub chemical: u8,
    pub ease: u8,
    pub detail: u8,
    pub cheapness: u8,
}

impl Properties {
    pub fn all(&self) -> [u8; 9] {
        [
            self.strength,
            self.stiffness,
            self.flexibility,
            self.impact,
            self.uv,
            self.chemical,
            self.ease,
            self.detail,
            self.cheapness,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Material {
    pub id: MaterialId,
    pub name: &'static str,
    /// Passed to `MaterialType::from_str` for profile generation.
    pub base: &'static str,
    pub props: Properties,
    /// Approximate heat deflection temperature, °C.
    pub hdt_c: u16,
    pub needs_enclosure: bool,
    /// Wears a brass or stainless nozzle: needs a hardened one.
    pub abrasive: bool,
    pub needs_drying: bool,
    /// 0 to 3.
    pub warp: u8,
}

impl Material {
    /// The base material for profile generation (`generator::base_profile_name`).
    pub fn material_type(&self) -> MaterialType {
        MaterialType::from_str(self.base)
    }
}

/// strength, stiffness, flexibility, impact, uv, chemical, ease, detail, cheapness.
#[allow(clippy::too_many_arguments)]
const fn p(
    strength: u8,
    stiffness: u8,
    flexibility: u8,
    impact: u8,
    uv: u8,
    chemical: u8,
    ease: u8,
    detail: u8,
    cheapness: u8,
) -> Properties {
    Properties {
        strength,
        stiffness,
        flexibility,
        impact,
        uv,
        chemical,
        ease,
        detail,
        cheapness,
    }
}

pub const MATERIALS: &[Material] = &[
    Material {
        id: MaterialId::Pla,
        name: "PLA",
        base: "PLA",
        props: p(2, 2, 0, 1, 1, 0, 3, 3, 3),
        hdt_c: 55,
        needs_enclosure: false,
        abrasive: false,
        needs_drying: false,
        warp: 0,
    },
    Material {
        id: MaterialId::PlaPlus,
        name: "PLA+",
        base: "PLA",
        props: p(2, 2, 0, 2, 1, 0, 3, 3, 3),
        hdt_c: 58,
        needs_enclosure: false,
        abrasive: false,
        needs_drying: false,
        warp: 0,
    },
    Material {
        id: MaterialId::PlaCf,
        name: "PLA-CF",
        base: "PLA",
        props: p(2, 3, 0, 1, 1, 0, 2, 2, 1),
        hdt_c: 60,
        needs_enclosure: false,
        abrasive: true,
        needs_drying: false,
        warp: 0,
    },
    Material {
        id: MaterialId::Petg,
        name: "PETG",
        base: "PETG",
        props: p(2, 1, 0, 2, 2, 2, 2, 2, 3),
        hdt_c: 70,
        needs_enclosure: false,
        abrasive: false,
        needs_drying: true,
        warp: 1,
    },
    Material {
        id: MaterialId::PetCf,
        name: "PET-CF",
        base: "PETG",
        props: p(3, 3, 0, 1, 2, 2, 2, 2, 1),
        hdt_c: 80,
        needs_enclosure: false,
        abrasive: true,
        needs_drying: true,
        warp: 1,
    },
    Material {
        id: MaterialId::Abs,
        name: "ABS",
        base: "ABS",
        props: p(2, 2, 0, 2, 1, 1, 1, 2, 3),
        hdt_c: 95,
        needs_enclosure: true,
        abrasive: false,
        needs_drying: false,
        warp: 3,
    },
    Material {
        id: MaterialId::Asa,
        name: "ASA",
        base: "ASA",
        props: p(2, 2, 0, 2, 3, 1, 1, 2, 2),
        hdt_c: 95,
        needs_enclosure: true,
        abrasive: false,
        needs_drying: false,
        warp: 2,
    },
    Material {
        id: MaterialId::Tpu,
        name: "TPU 95A",
        base: "TPU",
        props: p(1, 0, 3, 3, 2, 2, 1, 1, 2),
        hdt_c: 60,
        needs_enclosure: false,
        abrasive: false,
        needs_drying: true,
        warp: 0,
    },
    Material {
        id: MaterialId::Pa,
        name: "PA",
        base: "PA",
        props: p(3, 1, 1, 3, 1, 3, 0, 1, 1),
        hdt_c: 90,
        needs_enclosure: true,
        abrasive: false,
        needs_drying: true,
        warp: 3,
    },
    Material {
        id: MaterialId::PaCf,
        name: "PA-CF",
        base: "PA",
        props: p(3, 3, 0, 2, 2, 3, 1, 2, 0),
        hdt_c: 120,
        needs_enclosure: true,
        abrasive: true,
        needs_drying: true,
        warp: 2,
    },
    Material {
        id: MaterialId::Pc,
        name: "PC",
        base: "PC",
        props: p(3, 2, 0, 3, 1, 1, 0, 1, 1),
        hdt_c: 115,
        needs_enclosure: true,
        abrasive: false,
        needs_drying: true,
        warp: 3,
    },
];

/// The table row for `id`.
pub fn material(id: MaterialId) -> &'static Material {
    MATERIALS
        .iter()
        .find(|m| m.id == id)
        .expect("every MaterialId is in MATERIALS")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::generator::base_profile_name;

    #[test]
    fn every_material_is_listed_once_in_id_order() {
        let ids: Vec<MaterialId> = MATERIALS.iter().map(|m| m.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids, sorted);
        assert_eq!(ids.len(), 11);
        for m in MATERIALS {
            assert_eq!(material(m.id).name, m.name);
        }
        let names: Vec<&str> = MATERIALS.iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            [
                "PLA", "PLA+", "PLA-CF", "PETG", "PET-CF", "ABS", "ASA", "TPU 95A", "PA", "PA-CF",
                "PC"
            ]
        );
    }

    #[test]
    fn every_score_and_warp_is_zero_to_three() {
        for m in MATERIALS {
            assert!(m.props.all().iter().all(|s| *s <= 3), "{}", m.name);
            assert!(m.warp <= 3, "{}", m.name);
        }
    }

    #[test]
    fn heat_deflection_temperatures_are_plausible() {
        for m in MATERIALS {
            assert!(
                (45..=150).contains(&m.hdt_c),
                "{} at {} °C",
                m.name,
                m.hdt_c
            );
        }
        let hdt = |id| material(id).hdt_c;
        assert!(hdt(MaterialId::Pla) < hdt(MaterialId::Petg));
        assert!(hdt(MaterialId::Petg) < hdt(MaterialId::Abs));
        assert!(hdt(MaterialId::Abs) < hdt(MaterialId::Pc));
        assert!(hdt(MaterialId::Pa) < hdt(MaterialId::PaCf));
    }

    #[test]
    fn carbon_fibre_materials_and_only_they_are_abrasive() {
        for m in MATERIALS {
            assert_eq!(m.abrasive, m.name.ends_with("-CF"), "{}", m.name);
        }
    }

    #[test]
    fn tpu_is_the_flexible_one() {
        assert_eq!(material(MaterialId::Tpu).props.flexibility, 3);
        for m in MATERIALS.iter().filter(|m| m.id != MaterialId::Tpu) {
            assert!(m.props.flexibility < 3, "{}", m.name);
        }
    }

    #[test]
    fn enclosures_are_for_the_warping_engineering_materials() {
        let enclosed: Vec<&str> = MATERIALS
            .iter()
            .filter(|m| m.needs_enclosure)
            .map(|m| m.name)
            .collect();
        assert_eq!(enclosed, ["ABS", "ASA", "PA", "PA-CF", "PC"]);
    }

    #[test]
    fn each_material_maps_to_a_real_base_profile() {
        let base = |id| base_profile_name(&material(id).material_type());
        assert_eq!(base(MaterialId::Pla), "fdm_filament_pla");
        assert_eq!(base(MaterialId::PlaPlus), "fdm_filament_pla");
        assert_eq!(base(MaterialId::PlaCf), "fdm_filament_pla");
        assert_eq!(base(MaterialId::Petg), "fdm_filament_pet");
        assert_eq!(base(MaterialId::PetCf), "fdm_filament_pet");
        assert_eq!(base(MaterialId::Abs), "fdm_filament_abs");
        assert_eq!(base(MaterialId::Asa), "fdm_filament_asa");
        assert_eq!(base(MaterialId::Tpu), "fdm_filament_tpu");
        assert_eq!(base(MaterialId::Pa), "fdm_filament_pa");
        assert_eq!(base(MaterialId::PaCf), "fdm_filament_pa");
        assert_eq!(base(MaterialId::Pc), "fdm_filament_pc");
        for m in MATERIALS {
            assert!(
                !matches!(m.material_type(), MaterialType::Other(_)),
                "{}",
                m.name
            );
        }
    }

    /// Every material's pinned values: id, hdt_c, needs_drying, warp,
    /// needs_enclosure, abrasive, serde name. Changing any table value
    /// must change this list on purpose.
    #[test]
    fn every_material_pins_its_values() {
        #[rustfmt::skip]
        let expected: [(MaterialId, u16, bool, u8, bool, bool, &str); 11] = [
            (MaterialId::Pla,     55,  false, 0, false, false, "pla"),
            (MaterialId::PlaPlus, 58,  false, 0, false, false, "pla_plus"),
            (MaterialId::PlaCf,   60,  false, 0, false, true,  "pla_cf"),
            (MaterialId::Petg,    70,  true,  1, false, false, "petg"),
            (MaterialId::PetCf,   80,  true,  1, false, true,  "pet_cf"),
            (MaterialId::Abs,     95,  false, 3, true,  false, "abs"),
            (MaterialId::Asa,     95,  false, 2, true,  false, "asa"),
            (MaterialId::Tpu,     60,  true,  0, false, false, "tpu"),
            (MaterialId::Pa,      90,  true,  3, true,  false, "pa"),
            (MaterialId::PaCf,    120, true,  2, true,  true,  "pa_cf"),
            (MaterialId::Pc,      115, true,  3, true,  false, "pc"),
        ];
        assert_eq!(expected.len(), MATERIALS.len());
        for (id, hdt, drying, warp, enclosure, abrasive, serde_name) in expected {
            let m = material(id);
            assert_eq!(m.hdt_c, hdt, "{} hdt_c", m.name);
            assert_eq!(m.needs_drying, drying, "{} needs_drying", m.name);
            assert_eq!(m.warp, warp, "{} warp", m.name);
            assert_eq!(m.needs_enclosure, enclosure, "{} needs_enclosure", m.name);
            assert_eq!(m.abrasive, abrasive, "{} abrasive", m.name);
            let json = serde_json::to_string(&id).expect("serialise");
            assert_eq!(json, format!("\"{serde_name}\""), "{} serde name", m.name);
            let back: MaterialId = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(back, id, "{} serde round trip", m.name);
        }
    }

    #[test]
    fn heavy_warpers_are_enclosed() {
        for m in MATERIALS.iter().filter(|m| m.warp >= 2) {
            assert!(
                m.needs_enclosure,
                "{} warps {} but is not enclosed",
                m.name, m.warp
            );
        }
    }
}
