//! Scoring materials against what a part needs, the printer's hard
//! constraints and the model's geometry. Deterministic: equal totals keep
//! the material table's order.

use serde::Serialize;

use super::materials::{Material, MaterialId, MATERIALS};
use super::needs::{with_default, Need};
use crate::model::risks::RiskKind;
use crate::model::view::ModelView;
use crate::printer::discovery::model_name;
use crate::printer::service::PrinterView;
use crate::printer::state::Nozzle;

/// Materials returned as recommendations.
pub const TOP_N: usize = 3;
/// A footprint over this on either side lowers warp-prone materials.
pub const LARGE_FOOTPRINT_MM: f32 = 150.0;
pub const ENCLOSED_MODELS: &[&str] = &["H2C", "H2D", "H2S", "X1", "X1C", "X1E", "P1S", "P2S"];
pub const OPEN_MODELS: &[&str] = &["A1", "A1 mini", "P1P"];
pub const OVERHANG_NOTE: &str = "Supports likely, whatever the material";
pub const FOOD_CAUTION: &str = "Food contact: BambuMate never calls a material food safe. Check the maker's own food-contact statement, and remember that printed layer lines can trap bacteria.";
pub const NO_PRINTER_LINE: &str = "Assuming an enclosed printer; nozzle unknown.";
pub const HARDENED_NOTE: &str =
    "Needs a hardened nozzle; BambuMate can't see which nozzle is fitted.";
pub const DRYING_NOTE: &str = "Dry the spool before printing.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enclosure {
    Enclosed,
    Open,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NozzleKind {
    Hardened,
    NotHardened,
    Unknown,
}

/// What the advisor knows about the printer. Unknown is never a reason to
/// filter a material out: it adds a note instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterFacts {
    pub model: Option<String>,
    pub enclosure: Enclosure,
    pub nozzle: NozzleKind,
}

impl PrinterFacts {
    pub fn unknown() -> Self {
        Self {
            model: None,
            enclosure: Enclosure::Unknown,
            nozzle: NozzleKind::Unknown,
        }
    }

    /// The model from settings (or `get_version`), the nozzle from the last
    /// known live state.
    pub fn from_view(view: &PrinterView) -> Self {
        let model = view
            .printer
            .as_ref()
            .map(|p| p.model.trim().to_string())
            .filter(|m| !m.is_empty());
        let enclosure = model
            .as_deref()
            .map(enclosure_for_model)
            .unwrap_or(Enclosure::Unknown);
        let nozzle = view
            .state
            .as_ref()
            .map(|s| nozzle_kind(&s.nozzles))
            .unwrap_or(NozzleKind::Unknown);
        Self {
            model,
            enclosure,
            nozzle,
        }
    }
}

/// Letters and digits only, uppercased: "A1 mini" and "A1MINI" compare equal.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Whether a model is enclosed. Discovery codes (`C12`) are named first;
/// "X1 Carbon" and "H2D Pro" are spellings of the listed X1C and H2D.
pub fn enclosure_for_model(model: &str) -> Enclosure {
    let named = squash(&model_name(model.trim()));
    let named = match named.as_str() {
        "X1CARBON" => "X1C".to_string(),
        "H2DPRO" => "H2D".to_string(),
        _ => named,
    };
    if named.is_empty() {
        Enclosure::Unknown
    } else if ENCLOSED_MODELS.iter().any(|m| squash(m) == named) {
        Enclosure::Enclosed
    } else if OPEN_MODELS.iter().any(|m| squash(m) == named) {
        Enclosure::Open
    } else {
        Enclosure::Unknown
    }
}

/// Hardened when any nozzle is; not hardened only when every nozzle is
/// known and none is.
pub fn nozzle_kind(nozzles: &[Nozzle]) -> NozzleKind {
    let kinds: Vec<NozzleKind> = nozzles
        .iter()
        .map(|n| one_nozzle(n.nozzle_type.as_deref()))
        .collect();
    if kinds.contains(&NozzleKind::Hardened) {
        NozzleKind::Hardened
    } else if !kinds.is_empty() && kinds.iter().all(|k| *k == NozzleKind::NotHardened) {
        NozzleKind::NotHardened
    } else {
        NozzleKind::Unknown
    }
}

/// One reported nozzle type. `hardened_steel` and `stainless_steel` style
/// names are read directly. H2-series codes such as `HS01` start with the
/// nozzle material, and `H` is hardened steel. Anything else is unknown.
fn one_nozzle(kind: Option<&str>) -> NozzleKind {
    let Some(raw) = kind.map(str::trim).filter(|k| !k.is_empty()) else {
        return NozzleKind::Unknown;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.contains("hardened") || lower.contains("tungsten") {
        return NozzleKind::Hardened;
    }
    if lower.contains("stainless") || lower.contains("brass") {
        return NozzleKind::NotHardened;
    }
    let b = raw.as_bytes();
    let is_code = b.len() == 4
        && b[0].is_ascii_alphabetic()
        && b[1].is_ascii_alphabetic()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit();
    if is_code && b[0].eq_ignore_ascii_case(&b'H') {
        NozzleKind::Hardened
    } else {
        NozzleKind::Unknown
    }
}

/// What scoring uses from a loaded model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub footprint_mm: [f32; 2],
    pub height_mm: f32,
    pub overhang: bool,
    pub small_bed_contact: bool,
    pub tall_thin: bool,
}

impl Geometry {
    pub fn from_view(view: &ModelView) -> Self {
        let size = |i: usize| (view.bounds_max[i] - view.bounds_min[i]).max(0.0);
        let has = |k: RiskKind| view.risks.iter().any(|r| r.kind == k);
        Self {
            footprint_mm: [size(0), size(1)],
            height_mm: size(2),
            overhang: has(RiskKind::Overhang),
            small_bed_contact: has(RiskKind::SmallBedContact),
            tall_thin: has(RiskKind::TallThin),
        }
    }

    pub fn large_footprint(&self) -> bool {
        self.footprint_mm.iter().any(|s| *s > LARGE_FOOTPRINT_MM)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Met,
    Partly,
    NotMet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedReason {
    pub need: Need,
    pub verdict: Verdict,
    /// `3/3`, or `heat deflection ≈95 °C`.
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaterialFit {
    pub material: MaterialId,
    pub name: String,
    pub total: u32,
    pub fit_pct: u8,
    pub reasons: Vec<NeedReason>,
    pub printer_notes: Vec<String>,
    pub geometry_notes: Vec<String>,
    pub material_notes: Vec<String>,
}

/// A material that fails a hard constraint, with why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Excluded {
    pub material: MaterialId,
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scored {
    pub fits: Vec<MaterialFit>,
    pub not_on_printer: Vec<Excluded>,
}

fn verdict(points: u8) -> Verdict {
    match points {
        0 => Verdict::NotMet,
        1 => Verdict::Partly,
        _ => Verdict::Met,
    }
}

/// Points 0–3 and the reason's detail, or `None` for a need that isn't scored.
fn need_points(m: &Material, need: Need) -> Option<(u8, String)> {
    let p = &m.props;
    let prop = |v: u8| Some((v, format!("{v}/3")));
    match need {
        Need::Strong => prop(p.strength),
        Need::Stiff => prop(p.stiffness),
        Need::Flexible => prop(p.flexibility),
        Need::ImpactTough => prop(p.impact),
        Need::OutdoorUv => prop(p.uv),
        Need::Chemical => prop(p.chemical),
        Need::EasyPrint => prop(p.ease),
        Need::FineDetail => prop(p.detail),
        Need::LowCost => prop(p.cheapness),
        Need::Heat60 | Need::Heat80 | Need::Heat100 => {
            let margin = i32::from(m.hdt_c) - i32::from(need.heat_threshold()?);
            let points = if margin >= 10 {
                3
            } else if margin >= 0 {
                2
            } else if margin >= -10 {
                1
            } else {
                0
            };
            Some((points, format!("heat deflection ≈{} °C", m.hdt_c)))
        }
        Need::FoodContact => None,
    }
}

fn exclusion(m: &Material, printer: &PrinterFacts) -> Option<String> {
    if m.needs_enclosure && printer.enclosure == Enclosure::Open {
        return Some(match printer.model.as_deref() {
            Some(model) => format!("Needs an enclosed printer; the {model} is open."),
            None => "Needs an enclosed printer; this printer is open.".to_string(),
        });
    }
    if m.abrasive && printer.nozzle == NozzleKind::NotHardened {
        return Some(
            "Abrasive: needs a hardened nozzle, and this printer reports a standard one."
                .to_string(),
        );
    }
    None
}

/// One material's fit, or why it can't be printed here.
pub fn fit_for(
    m: &Material,
    needs: &[Need],
    printer: &PrinterFacts,
    geometry: Option<&Geometry>,
) -> Result<MaterialFit, Excluded> {
    if let Some(reason) = exclusion(m, printer) {
        return Err(Excluded {
            material: m.id,
            name: m.name.to_string(),
            reason,
        });
    }
    let needs = with_default(needs);
    let mut reasons = Vec::new();
    let mut points: i32 = 0;
    let mut max: i32 = 0;
    for need in &needs {
        if let Some((p, detail)) = need_points(m, *need) {
            points += i32::from(p);
            max += 3;
            reasons.push(NeedReason {
                need: *need,
                verdict: verdict(p),
                detail,
            });
        }
    }

    let mut printer_notes = Vec::new();
    if m.abrasive && printer.nozzle == NozzleKind::Unknown {
        printer_notes.push(HARDENED_NOTE.to_string());
    }
    if m.needs_enclosure && printer.enclosure == Enclosure::Unknown {
        printer_notes.push("Needs an enclosure; assuming this printer has one.".to_string());
    }

    let mut geometry_notes = Vec::new();
    if let Some(g) = geometry {
        if (g.large_footprint() || g.small_bed_contact) && m.warp > 0 {
            points -= i32::from(m.warp);
            geometry_notes.push(if g.large_footprint() {
                format!(
                    "Large footprint ({:.0} × {:.0} mm): {} may warp; a brim helps.",
                    g.footprint_mm[0], g.footprint_mm[1], m.name
                )
            } else {
                format!("Small bed contact: {} may lift; a brim helps.", m.name)
            });
        }
        if g.tall_thin {
            points += i32::from(m.props.stiffness);
            max += 3;
            geometry_notes.push(format!(
                "Tall and thin: stiffness {}/3 helps it stay straight.",
                m.props.stiffness
            ));
        }
        if g.overhang {
            geometry_notes.push(OVERHANG_NOTE.to_string());
        }
    }

    let mut material_notes = Vec::new();
    if m.needs_drying {
        material_notes.push(DRYING_NOTE.to_string());
    }
    if needs.contains(&Need::FoodContact) {
        material_notes.push(FOOD_CAUTION.to_string());
    }

    let total = points.max(0) as u32;
    // `with_default` guarantees a scored need, so `max` is at least 3.
    let fit_pct = (total * 100 / max.max(1) as u32).min(100) as u8;
    Ok(MaterialFit {
        material: m.id,
        name: m.name.to_string(),
        total,
        fit_pct,
        reasons,
        printer_notes,
        geometry_notes,
        material_notes,
    })
}

/// The top `TOP_N` fits, plus every material that fails a hard constraint.
pub fn score(needs: &[Need], printer: &PrinterFacts, geometry: Option<&Geometry>) -> Scored {
    let mut fits = Vec::new();
    let mut not_on_printer = Vec::new();
    for m in MATERIALS {
        match fit_for(m, needs, printer, geometry) {
            Ok(fit) => fits.push(fit),
            Err(e) => not_on_printer.push(e),
        }
    }
    // A stable sort: equal totals keep the table's order.
    fits.sort_by_key(|f| std::cmp::Reverse(f.total));
    fits.truncate(TOP_N);
    Scored {
        fits,
        not_on_printer,
    }
}

/// The page's one line about the printer, when it is assuming something.
pub fn printer_line(printer: &PrinterFacts, configured: bool) -> Option<String> {
    if !configured {
        return Some(NO_PRINTER_LINE.to_string());
    }
    if printer.enclosure == Enclosure::Unknown {
        return Some(match printer.model.as_deref() {
            Some(m) => {
                format!("BambuMate doesn't know whether the {m} is enclosed; assuming it is.")
            }
            None => "Assuming an enclosed printer.".to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advisor::materials::{material, MaterialId, MaterialId::*};
    use crate::model::risks::Risk;

    fn enclosed() -> PrinterFacts {
        PrinterFacts {
            model: Some("X1C".into()),
            enclosure: Enclosure::Enclosed,
            nozzle: NozzleKind::Hardened,
        }
    }
    fn a1() -> PrinterFacts {
        PrinterFacts {
            model: Some("A1".into()),
            enclosure: Enclosure::Open,
            nozzle: NozzleKind::Unknown,
        }
    }
    fn ids(s: &Scored) -> Vec<MaterialId> {
        s.fits.iter().map(|f| f.material).collect()
    }
    fn excluded(s: &Scored) -> Vec<MaterialId> {
        s.not_on_printer.iter().map(|e| e.material).collect()
    }
    fn flat(w: f32, d: f32) -> Geometry {
        Geometry {
            footprint_mm: [w, d],
            height_mm: 10.0,
            overhang: false,
            small_bed_contact: false,
            tall_thin: false,
        }
    }

    #[test]
    fn outdoor_and_heat_80_is_asa_on_an_enclosed_printer() {
        let s = score(&[Need::OutdoorUv, Need::Heat80], &enclosed(), None);
        assert_eq!(ids(&s), [Asa, PaCf, PetCf]);
        assert!(s.not_on_printer.is_empty());
        let asa = &s.fits[0];
        assert_eq!(asa.total, 6);
        assert_eq!(asa.fit_pct, 100);
        assert!(asa.reasons.iter().all(|r| r.verdict == Verdict::Met));
        assert_eq!(asa.reasons[0].need, Need::Heat80);
        assert_eq!(asa.reasons[0].detail, "heat deflection ≈95 °C");
        assert_eq!(asa.reasons[1].detail, "3/3");
    }

    #[test]
    fn on_an_a1_asa_is_not_on_this_printer() {
        let s = score(&[Need::OutdoorUv, Need::Heat80], &a1(), None);
        assert_eq!(excluded(&s), [Abs, Asa, Pa, PaCf, Pc]);
        let asa = s.not_on_printer.iter().find(|e| e.material == Asa).unwrap();
        assert_eq!(asa.reason, "Needs an enclosed printer; the A1 is open.");
        assert_eq!(ids(&s), [PetCf, Petg, Tpu]);
    }

    #[test]
    fn flexible_gives_tpu() {
        assert_eq!(
            score(&[Need::Flexible], &enclosed(), None).fits[0].material,
            Tpu
        );
    }

    #[test]
    fn a_large_footprint_lowers_abs() {
        let needs = [Need::ImpactTough];
        let abs = material(Abs);
        let plain = fit_for(abs, &needs, &enclosed(), None).unwrap();
        let large = fit_for(abs, &needs, &enclosed(), Some(&flat(200.0, 100.0))).unwrap();
        assert!(
            large.total < plain.total,
            "{} vs {}",
            large.total,
            plain.total
        );
        assert_eq!(
            large.geometry_notes[0],
            "Large footprint (200 × 100 mm): ABS may warp; a brim helps."
        );
        // 150 mm exactly is not over the limit.
        let edge = fit_for(abs, &needs, &enclosed(), Some(&flat(150.0, 150.0))).unwrap();
        assert_eq!(edge.total, plain.total);
        // PLA doesn't warp, so it keeps its score.
        let pla = material(Pla);
        assert_eq!(
            fit_for(pla, &needs, &enclosed(), Some(&flat(200.0, 100.0)))
                .unwrap()
                .total,
            fit_for(pla, &needs, &enclosed(), None).unwrap().total
        );
        // Small bed contact is treated the same way.
        let contact = Geometry {
            small_bed_contact: true,
            ..flat(20.0, 20.0)
        };
        assert!(
            fit_for(abs, &needs, &enclosed(), Some(&contact))
                .unwrap()
                .total
                < plain.total
        );
    }

    #[test]
    fn an_unknown_nozzle_notes_cf_materials_without_filtering_them() {
        let s = score(&[Need::Stiff], &PrinterFacts::unknown(), None);
        assert_eq!(ids(&s), [PlaCf, PetCf, PaCf]);
        assert!(s.not_on_printer.is_empty());
        for f in &s.fits {
            assert!(
                f.printer_notes.contains(&HARDENED_NOTE.to_string()),
                "{f:?}"
            );
        }
        let stainless = PrinterFacts {
            nozzle: NozzleKind::NotHardened,
            ..PrinterFacts::unknown()
        };
        let s = score(&[Need::Stiff], &stainless, None);
        assert_eq!(excluded(&s), [PlaCf, PetCf, PaCf]);
        assert!(s.not_on_printer[0].reason.contains("hardened nozzle"));
        assert_eq!(ids(&s), [Pla, PlaPlus, Abs]);
    }

    #[test]
    fn ties_keep_the_table_order_and_repeat() {
        let a = score(&[], &PrinterFacts::unknown(), None);
        assert_eq!(ids(&a), [Pla, PlaPlus, PlaCf]);
        assert_eq!(a.fits[0].reasons[0].need, Need::EasyPrint);
        assert_eq!(a, score(&[], &PrinterFacts::unknown(), None));
    }

    #[test]
    fn food_contact_only_ever_adds_a_caution() {
        let s = score(&[Need::FoodContact, Need::Strong], &enclosed(), None);
        for f in &s.fits {
            assert!(f.reasons.iter().all(|r| r.need != Need::FoodContact));
            assert!(f.material_notes.contains(&FOOD_CAUTION.to_string()));
        }
        assert!(FOOD_CAUTION.contains("never calls a material food safe"));
    }

    #[test]
    fn heat_reasons_are_graded_by_margin() {
        let verdict = |id| {
            fit_for(material(id), &[Need::Heat80], &enclosed(), None)
                .unwrap()
                .reasons[0]
                .verdict
        };
        assert_eq!(verdict(Pla), Verdict::NotMet); // 55 °C: 25 short
        assert_eq!(verdict(Petg), Verdict::Partly); // 70 °C: 10 short
        assert_eq!(verdict(PetCf), Verdict::Met); // 80 °C: just enough
    }

    #[test]
    fn tall_and_thin_favours_stiffness_and_overhangs_are_noted() {
        let tall = Geometry {
            tall_thin: true,
            overhang: true,
            ..flat(10.0, 10.0)
        };
        let cf = material(PlaCf);
        let with = fit_for(cf, &[Need::Flexible], &enclosed(), Some(&tall)).unwrap();
        let without = fit_for(cf, &[Need::Flexible], &enclosed(), None).unwrap();
        assert!(with.total > without.total);
        assert!(with.geometry_notes.contains(&OVERHANG_NOTE.to_string()));
        assert_eq!(OVERHANG_NOTE, "Supports likely, whatever the material");
    }

    #[test]
    fn printer_models_map_to_enclosures() {
        for m in [
            "H2C",
            "H2D",
            "H2D Pro",
            "H2S",
            "X1",
            "X1C",
            "X1 Carbon",
            "X1E",
            "P1S",
            "P2S",
            "C12",
            "O1D",
        ] {
            assert_eq!(enclosure_for_model(m), Enclosure::Enclosed, "{m}");
        }
        for m in ["A1", "A1 mini", "a1 MINI", "N2S", "N1", "P1P"] {
            assert_eq!(enclosure_for_model(m), Enclosure::Open, "{m}");
        }
        for m in ["", "Ender 3"] {
            assert_eq!(enclosure_for_model(m), Enclosure::Unknown, "{m}");
        }
    }

    #[test]
    fn nozzle_types_are_read_conservatively() {
        let n = |t: Option<&str>| Nozzle {
            nozzle_type: t.map(str::to_string),
            ..Default::default()
        };
        assert_eq!(nozzle_kind(&[]), NozzleKind::Unknown);
        assert_eq!(
            nozzle_kind(&[n(Some("hardened_steel"))]),
            NozzleKind::Hardened
        );
        assert_eq!(
            nozzle_kind(&[n(Some("stainless_steel"))]),
            NozzleKind::NotHardened
        );
        assert_eq!(
            nozzle_kind(&[n(Some("HS01")), n(Some("HS01"))]),
            NozzleKind::Hardened
        );
        assert_eq!(
            nozzle_kind(&[n(Some("stainless_steel")), n(None)]),
            NozzleKind::Unknown
        );
        assert_eq!(nozzle_kind(&[n(Some("ZZ99"))]), NozzleKind::Unknown);
        assert_eq!(
            nozzle_kind(&[n(Some("stainless_steel")), n(Some("hardened_steel"))]),
            NozzleKind::Hardened
        );
    }

    #[test]
    fn the_page_says_when_it_assumes_an_enclosure() {
        assert_eq!(
            NO_PRINTER_LINE,
            "Assuming an enclosed printer; nozzle unknown."
        );
        assert_eq!(
            printer_line(&PrinterFacts::unknown(), false).as_deref(),
            Some(NO_PRINTER_LINE)
        );
        assert_eq!(printer_line(&enclosed(), true), None);
        assert_eq!(printer_line(&a1(), true), None);
        let other = PrinterFacts {
            model: Some("Ender 3".into()),
            ..PrinterFacts::unknown()
        };
        assert_eq!(
            printer_line(&other, true).as_deref(),
            Some("BambuMate doesn't know whether the Ender 3 is enclosed; assuming it is.")
        );
    }

    #[test]
    fn printer_facts_come_from_the_view() {
        assert_eq!(
            PrinterFacts::from_view(&PrinterView::unconfigured()),
            PrinterFacts::unknown()
        );
    }

    #[test]
    fn geometry_comes_from_the_model_view() {
        let view = ModelView {
            name: "part.stl".into(),
            triangle_count: 12,
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [200.0, 40.0, 10.0],
            dots: String::new(),
            dot_normals: String::new(),
            edges: String::new(),
            risks: vec![Risk {
                kind: RiskKind::Overhang,
                position: [0.0; 3],
                value: 64.0,
                message: String::new(),
            }],
        };
        let g = Geometry::from_view(&view);
        assert_eq!(g.footprint_mm, [200.0, 40.0]);
        assert_eq!(g.height_mm, 10.0);
        assert!(g.large_footprint());
        assert!(g.overhang && !g.tall_thin && !g.small_bed_contact);
    }
}
