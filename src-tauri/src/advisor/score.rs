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
/// The page's line when no material is left to recommend.
pub const NOTHING_FITS: &str = "Nothing on this printer meets all of that.";

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

/// A hand-typed model with the maker and bundle words removed:
/// "Bambu Lab X1 Carbon" becomes "X1 Carbon", "P1S Combo" becomes "P1S".
fn clean_model(model: &str) -> String {
    const LEADING: &[&str] = &["bambu", "lab", "bambulab"];
    const TRAILING: &[&str] = &["combo", "ams", "with", "printer", "lite"];
    let mut words: Vec<&str> = model.split_whitespace().collect();
    while words
        .first()
        .is_some_and(|w| LEADING.contains(&w.to_ascii_lowercase().as_str()))
    {
        words.remove(0);
    }
    while words
        .last()
        .is_some_and(|w| TRAILING.contains(&w.to_ascii_lowercase().as_str()))
    {
        words.pop();
    }
    words.join(" ")
}

/// The name shown to the user: a discovery code such as `N2S` becomes "A1".
fn friendly_model(model: &str) -> String {
    let cleaned = clean_model(model);
    model_name(&cleaned)
}

/// Whether a model is enclosed. Discovery codes (`C12`) are named first;
/// "X1 Carbon" and "H2D Pro" are spellings of the listed X1C and H2D, and a
/// leading "Bambu Lab" or trailing "Combo" is ignored.
pub fn enclosure_for_model(model: &str) -> Enclosure {
    let named = squash(&friendly_model(model));
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
    // Whole words only, so a negation such as `not_hardened` is never read
    // as hardened. A name that says both is left unknown.
    let lower = raw.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    let negated = ["not", "non", "no", "un"].iter().any(|n| has(n));
    let hard_word = has("hardened") || has("tungsten");
    let soft_word = has("stainless") || has("brass") || has("unhardened") || has("nonhardened");
    if hard_word && negated {
        return NozzleKind::NotHardened;
    }
    match (hard_word, soft_word) {
        (true, false) => return NozzleKind::Hardened,
        (false, true) => return NozzleKind::NotHardened,
        (true, true) => return NozzleKind::Unknown,
        (false, false) => {}
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
    /// The needs score plus geometry, before any clamp: negative when
    /// warping outweighs the fit. Ranking uses this. Not sent to the page.
    #[serde(skip)]
    pub points: i32,
    /// The score from the needs alone, before geometry. A material with none
    /// is not shown; geometry only moves it down. Not sent to the page.
    #[serde(skip)]
    pub needs_points: i32,
    pub total: u32,
    pub fit_pct: u8,
    pub reasons: Vec<NeedReason>,
    pub printer_notes: Vec<String>,
    pub geometry_notes: Vec<String>,
    pub material_notes: Vec<String>,
}

/// A material that fails a hard constraint, with every reason it fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Excluded {
    pub material: MaterialId,
    pub name: String,
    pub reasons: Vec<String>,
}

/// The result of scoring. `fits` is the top `TOP_N` and is empty when
/// nothing fits (the page then shows `NOTHING_FITS`). `ruled_out` lists the
/// materials that fail a hard constraint (enclosure, nozzle or heat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scored {
    pub fits: Vec<MaterialFit>,
    pub ruled_out: Vec<Excluded>,
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
            // Heat is a requirement: at or above the level is Met (3 with 10 °C
            // to spare, else 2); below it scores 0 and the material is ruled out.
            let margin = i32::from(m.hdt_c) - i32::from(need.heat_threshold()?);
            let points = if margin >= 10 {
                3
            } else if margin >= 0 {
                2
            } else {
                0
            };
            Some((points, format!("heat deflection ≈{} °C", m.hdt_c)))
        }
        Need::FoodContact => None,
    }
}

/// Every hard constraint the material fails here. Heat is one: a material
/// that scores nothing on the chosen heat level is ruled out, not ranked.
fn exclusions(m: &Material, needs: &[Need], printer: &PrinterFacts) -> Vec<String> {
    let mut reasons = Vec::new();
    if m.needs_enclosure && printer.enclosure == Enclosure::Open {
        reasons.push(match printer.model.as_deref() {
            Some(model) if !friendly_model(model).is_empty() => format!(
                "Needs an enclosed printer; the {} is open.",
                friendly_model(model)
            ),
            _ => "Needs an enclosed printer; this printer is open.".to_string(),
        });
    }
    if m.abrasive && printer.nozzle == NozzleKind::NotHardened {
        reasons.push(
            "Abrasive: needs a hardened nozzle, and this printer reports a standard one."
                .to_string(),
        );
    }
    let fails_heat = needs
        .iter()
        .any(|n| matches!(need_points(m, *n), Some((0, _))) && n.heat_threshold().is_some());
    if fails_heat {
        reasons.push(format!("Softens around {} °C", m.hdt_c));
    }
    reasons
}

/// One material's fit, or why it can't be printed here.
pub fn fit_for(
    m: &Material,
    needs: &[Need],
    printer: &PrinterFacts,
    geometry: Option<&Geometry>,
) -> Result<MaterialFit, Excluded> {
    let needs = with_default(needs);
    let reasons = exclusions(m, &needs, printer);
    if !reasons.is_empty() {
        return Err(Excluded {
            material: m.id,
            name: m.name.to_string(),
            reasons,
        });
    }
    let mut reasons = Vec::new();
    let mut points: i32 = 0;
    let mut needs_max: i32 = 0;
    for need in &needs {
        if let Some((p, detail)) = need_points(m, *need) {
            points += i32::from(p);
            needs_max += 3;
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

    let needs_points = points;
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
            // The rank bonus applies to every material; the note only where
            // stiffness actually helps.
            if m.props.stiffness >= 2 {
                geometry_notes.push(format!(
                    "Tall and thin: stiffness {}/3 helps it stay straight.",
                    m.props.stiffness
                ));
            }
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

    // The fit shown comes from the needs alone; geometry only reorders.
    let total = needs_points.max(0) as u32;
    // `with_default` guarantees a scored need, so `needs_max` is at least 3.
    let fit_pct = (total * 100 / needs_max.max(1) as u32).min(100) as u8;
    Ok(MaterialFit {
        material: m.id,
        name: m.name.to_string(),
        points,
        needs_points,
        total,
        fit_pct,
        reasons,
        printer_notes,
        geometry_notes,
        material_notes,
    })
}

/// The top `TOP_N` fits, plus every material ruled out by a hard
/// constraint. A material that scores nothing on the needs themselves is not
/// shown and not listed; geometry never hides a material, it only lowers its
/// rank. The page says `NOTHING_FITS` when none is left.
pub fn score(needs: &[Need], printer: &PrinterFacts, geometry: Option<&Geometry>) -> Scored {
    let mut fits = Vec::new();
    let mut ruled_out = Vec::new();
    for m in MATERIALS {
        match fit_for(m, needs, printer, geometry) {
            Ok(fit) => fits.push(fit),
            Err(e) => ruled_out.push(e),
        }
    }
    fits.retain(|f| f.needs_points > 0);
    // A stable sort on the unclamped points, then the needs-only score;
    // anything still equal keeps the table's order.
    fits.sort_by_key(|f| {
        (
            std::cmp::Reverse(f.points),
            std::cmp::Reverse(f.needs_points),
        )
    });
    fits.truncate(TOP_N);
    Scored { fits, ruled_out }
}

/// The page's one line about the printer, when it is assuming something.
pub fn printer_line(printer: &PrinterFacts, configured: bool) -> Option<String> {
    if !configured {
        return Some(NO_PRINTER_LINE.to_string());
    }
    if printer.enclosure == Enclosure::Unknown {
        return Some(match printer.model.as_deref() {
            // A model that is only "Bambu Lab" or "Combo" names nothing.
            Some(m) if friendly_model(m).is_empty() => NO_PRINTER_LINE.to_string(),
            Some(m) => format!(
                "BambuMate doesn't know whether the {} is enclosed; assuming it is.",
                friendly_model(m)
            ),
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
        s.ruled_out.iter().map(|e| e.material).collect()
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
        // Heat is a hard requirement: these soften below 80 °C.
        assert_eq!(excluded(&s), [Pla, PlaPlus, PlaCf, Petg, Tpu]);
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
        // Open printer rules out the enclosed materials; heat rules out the
        // ones that soften below 80 °C (the controller's ruling I1).
        assert_eq!(
            excluded(&s),
            [Pla, PlaPlus, PlaCf, Petg, Abs, Asa, Tpu, Pa, PaCf, Pc]
        );
        let asa = s.ruled_out.iter().find(|e| e.material == Asa).unwrap();
        assert_eq!(asa.reasons, ["Needs an enclosed printer; the A1 is open."]);
        assert_eq!(ids(&s), [PetCf]);
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
            large.points < plain.points,
            "{} vs {}",
            large.points,
            plain.points
        );
        // The fit shown comes from the needs, so it does not move.
        assert_eq!(large.total, plain.total);
        assert_eq!(
            large.geometry_notes[0],
            "Large footprint (200 × 100 mm): ABS may warp; a brim helps."
        );
        // 150 mm exactly is not over the limit.
        let edge = fit_for(abs, &needs, &enclosed(), Some(&flat(150.0, 150.0))).unwrap();
        assert_eq!(edge.points, plain.points);
        // PLA doesn't warp, so it keeps its score.
        let pla = material(Pla);
        assert_eq!(
            fit_for(pla, &needs, &enclosed(), Some(&flat(200.0, 100.0)))
                .unwrap()
                .points,
            fit_for(pla, &needs, &enclosed(), None).unwrap().points
        );
        // Small bed contact is treated the same way.
        let contact = Geometry {
            small_bed_contact: true,
            ..flat(20.0, 20.0)
        };
        assert!(
            fit_for(abs, &needs, &enclosed(), Some(&contact))
                .unwrap()
                .points
                < plain.points
        );
    }

    #[test]
    fn an_unknown_nozzle_notes_cf_materials_without_filtering_them() {
        let s = score(&[Need::Stiff], &PrinterFacts::unknown(), None);
        assert_eq!(ids(&s), [PlaCf, PetCf, PaCf]);
        assert!(s.ruled_out.is_empty());
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
        assert!(s.ruled_out[0].reasons[0].contains("hardened nozzle"));
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
    fn heat_is_met_or_ruled_out_never_partly() {
        let at = |id, need| fit_for(material(id), &[need], &enclosed(), None);
        let softens = |id, need, hdt: u16| {
            assert_eq!(
                at(id, need).unwrap_err().reasons,
                [format!("Softens around {hdt} °C")],
                "{id:?}"
            );
        };
        let met = |id, need| {
            assert_eq!(
                at(id, need).unwrap().reasons[0].verdict,
                Verdict::Met,
                "{id:?}"
            );
        };
        // heat >= 80
        softens(Petg, Need::Heat80, 70); // 10 short is still short
        softens(Pla, Need::Heat80, 55);
        met(PetCf, Need::Heat80); // 80 °C: exactly enough
                                  // heat >= 100
        softens(Abs, Need::Heat100, 95);
        softens(Asa, Need::Heat100, 95);
        met(Pc, Need::Heat100);
        met(PaCf, Need::Heat100);
        // heat >= 60
        softens(Pla, Need::Heat60, 55);
        met(Tpu, Need::Heat60); // 60 °C: exactly enough
                                // No material ever gets a "Partly" heat row.
        for m in MATERIALS {
            for need in [Need::Heat60, Need::Heat80, Need::Heat100] {
                if let Ok(f) = fit_for(m, &[need], &enclosed(), None) {
                    assert_eq!(f.reasons[0].verdict, Verdict::Met);
                }
            }
        }
    }

    #[test]
    fn heat_margin_boundaries() {
        // A copy of PLA with a chosen heat deflection temperature, against 80 °C.
        let total = |hdt_c: u16| {
            let m = Material {
                hdt_c,
                ..*material(Pla)
            };
            fit_for(&m, &[Need::Heat80], &enclosed(), None).map(|f| f.total)
        };
        assert_eq!(total(90), Ok(3)); // +10
        assert_eq!(total(89), Ok(2)); // +9
        assert_eq!(total(80), Ok(2)); // exactly the threshold: Met
        let out = total(79).unwrap_err(); // 1 short: ruled out
        assert_eq!(out.reasons, ["Softens around 79 °C"]);
        assert!(total(70).is_err());
        // The shipped table: PA at 90 is +10, PET-CF at 80 is 0, PETG is out.
        let pts = |id| fit_for(material(id), &[Need::Heat80], &enclosed(), None);
        assert_eq!(pts(Pa).unwrap().total, 3);
        assert_eq!(pts(PetCf).unwrap().total, 2);
        assert!(pts(Petg).is_err());
    }

    #[test]
    fn an_open_printer_with_a_high_heat_need_has_nothing_to_recommend() {
        let s = score(&[Need::Heat100], &a1(), None);
        assert!(s.fits.is_empty());
        assert_eq!(NOTHING_FITS, "Nothing on this printer meets all of that.");
        let reasons = |id| {
            s.ruled_out
                .iter()
                .find(|e| e.material == id)
                .unwrap()
                .reasons
                .clone()
        };
        for (id, hdt) in [(Pla, 55), (PlaPlus, 58), (PlaCf, 60)] {
            assert_eq!(reasons(id), [format!("Softens around {hdt} °C")]);
        }
        for id in [PaCf, Pc] {
            assert_eq!(reasons(id), ["Needs an enclosed printer; the A1 is open."]);
        }
        // Several reasons at once: the enclosure, then heat.
        for (id, hdt) in [(Abs, 95), (Asa, 95), (Pa, 90)] {
            assert_eq!(
                reasons(id),
                [
                    "Needs an enclosed printer; the A1 is open.".to_string(),
                    format!("Softens around {hdt} °C")
                ]
            );
        }
        // Nothing is dropped silently: every material is ruled out here.
        assert_eq!(s.ruled_out.len(), MATERIALS.len());
    }

    #[test]
    fn a_large_part_needing_heat_is_lowered_never_hidden() {
        let big = flat(200.0, 100.0);
        let s = score(&[Need::Heat100], &enclosed(), Some(&big));
        // No PLA: heat rules it out.
        assert!(s
            .fits
            .iter()
            .all(|f| !matches!(f.material, Pla | PlaPlus | PlaCf)));
        for id in [Pla, PlaPlus, PlaCf] {
            assert!(excluded(&s).contains(&id));
        }
        // ABS, ASA and PA soften below 100 °C, so they are ruled out too.
        for id in [Abs, Asa, Pa] {
            assert!(excluded(&s).contains(&id));
        }
        // PA-CF first; PC, which meets heat, is shown below it with a warp note.
        assert_eq!(ids(&s), [PaCf, Pc]);
        let pc = &s.fits[1];
        // It meets the need, so it shows a full fit; geometry only reorders.
        assert_eq!(pc.total, 3);
        assert_eq!(pc.fit_pct, 100);
        assert!(pc.points < s.fits[0].points);
        assert_eq!(pc.reasons[0].verdict, Verdict::Met);
        assert!(pc.geometry_notes[0].contains("PC may warp"));
        // Same needs, no geometry: the same materials are shown.
        let plain = score(&[Need::Heat100], &enclosed(), None);
        assert_eq!(&ids(&plain)[..2], [PaCf, Pc]);
    }

    #[test]
    fn a_pick_that_meets_the_needs_never_shows_zero_percent() {
        let big = flat(200.0, 100.0);
        let s = score(&[Need::Heat100], &enclosed(), Some(&big));
        assert!(!s.fits.is_empty());
        for f in &s.fits {
            assert!(f.needs_points > 0 && f.total > 0 && f.fit_pct > 0, "{f:?}");
        }
        // A material that fails the needs themselves is still not shown.
        let s = score(&[Need::Flexible], &enclosed(), None);
        assert_eq!(ids(&s)[0], Tpu);
        assert!(!ids(&s).contains(&Pla));
    }

    #[test]
    fn geometry_keeps_the_fit_but_lowers_the_rank() {
        let needs = [Need::Heat80, Need::Strong];
        let big = flat(200.0, 100.0);
        let tall = Geometry {
            tall_thin: true,
            ..flat(10.0, 10.0)
        };
        for geometry in [big, tall] {
            for id in [Abs, Pc, PaCf] {
                let with = fit_for(material(id), &needs, &enclosed(), Some(&geometry)).unwrap();
                let without = fit_for(material(id), &needs, &enclosed(), None).unwrap();
                assert_eq!((with.total, with.fit_pct), (without.total, without.fit_pct));
                assert_eq!(with.needs_points, without.points);
            }
        }
        // Warping lowers the rank (points), never the displayed fit.
        let pc_big = fit_for(material(Pc), &needs, &enclosed(), Some(&big)).unwrap();
        let pc_plain = fit_for(material(Pc), &needs, &enclosed(), None).unwrap();
        assert!(pc_big.points < pc_plain.points);
        // And it can reorder the picks.
        let order = |g: Option<&Geometry>| ids(&score(&needs, &enclosed(), g));
        assert_ne!(order(Some(&big)), order(None));
    }

    #[test]
    fn flexible_on_an_open_printer_is_tpu_alone() {
        let s = score(&[Need::Flexible], &a1(), None);
        assert_eq!(ids(&s), [Tpu]);
    }

    #[test]
    fn ranking_uses_the_unclamped_points() {
        // The sort sees the signed points, which the displayed fit hides.
        let big = flat(200.0, 100.0);
        let needs = [Need::Heat80, Need::Strong];
        let pc = fit_for(material(Pc), &needs, &enclosed(), Some(&big)).unwrap();
        let abs = fit_for(material(Abs), &needs, &enclosed(), Some(&big)).unwrap();
        assert!(pc.points > abs.points);
        // ABS warps: on chemical resistance alone it goes below zero.
        let none = fit_for(material(Abs), &[Need::Chemical], &enclosed(), Some(&big)).unwrap();
        assert!(none.points < 0);
        // The fit shown is from the needs alone: chemical 1/3.
        assert_eq!(none.total, 1);
    }

    #[test]
    fn a_tie_on_points_goes_to_the_better_fit_then_table_order() {
        // Strong with a big footprint: PET-CF 3 - 1 warp ties PLA at 2, but PET-CF
        // fits the need better, so it comes first.
        let big = flat(200.0, 100.0);
        let s = score(&[Need::Strong], &enclosed(), Some(&big));
        assert_eq!(ids(&s), [PetCf, Pla, PlaPlus]);
        assert_eq!(s.fits[0].points, s.fits[1].points);
        assert!(s.fits[0].needs_points > s.fits[1].needs_points);
        // Equal on both: table order (PLA before PLA+).
        assert_eq!(s.fits[1].needs_points, s.fits[2].needs_points);
    }

    #[test]
    fn the_tall_note_is_only_for_stiff_materials() {
        let tall = Geometry {
            tall_thin: true,
            ..flat(10.0, 10.0)
        };
        let note = |id| {
            fit_for(material(id), &[Need::Flexible], &enclosed(), Some(&tall))
                .unwrap()
                .geometry_notes
                .iter()
                .any(|n| n.starts_with("Tall and thin"))
        };
        assert!(!note(Tpu)); // stiffness 0
        assert!(!note(Pa)); // stiffness 1
        assert!(note(PlaCf)); // stiffness 3
        assert!(note(Pla)); // stiffness 2
                            // The rank bonus still applies to every material.
        let pa = fit_for(material(Pa), &[Need::Flexible], &enclosed(), Some(&tall)).unwrap();
        assert!(pa.points > pa.needs_points);
    }

    #[test]
    fn a_model_that_cleans_to_nothing_is_no_model() {
        for raw in ["Bambu Lab", "Combo", "Lab", "  "] {
            let p = PrinterFacts {
                model: Some(raw.into()),
                enclosure: enclosure_for_model(raw),
                nozzle: NozzleKind::Unknown,
            };
            assert_eq!(p.enclosure, Enclosure::Unknown, "{raw:?}");
            assert_eq!(printer_line(&p, true).as_deref(), Some(NO_PRINTER_LINE));
        }
        let open = PrinterFacts {
            model: Some("Bambu Lab".into()),
            enclosure: Enclosure::Open,
            nozzle: NozzleKind::Unknown,
        };
        let out = fit_for(material(Asa), &[Need::Strong], &open, None).unwrap_err();
        assert_eq!(
            out.reasons,
            ["Needs an enclosed printer; this printer is open."]
        );
    }

    #[test]
    fn every_failing_constraint_is_reported() {
        let both = PrinterFacts {
            model: Some("A1".into()),
            enclosure: Enclosure::Open,
            nozzle: NozzleKind::NotHardened,
        };
        let out = fit_for(material(PaCf), &[Need::Strong], &both, None).unwrap_err();
        assert_eq!(out.reasons.len(), 2);
        assert!(out.reasons[0].contains("enclosed printer"));
        assert!(out.reasons[1].contains("hardened nozzle"));
        // With a heat need that PA-CF meets, a third reason does not appear;
        // PLA-CF on the same printer fails the nozzle and the heat need.
        let out = fit_for(material(PlaCf), &[Need::Heat80], &both, None).unwrap_err();
        assert_eq!(out.reasons.len(), 2);
        assert!(out.reasons[1].starts_with("Softens around 60"));
    }

    #[test]
    fn an_unknown_enclosure_notes_the_enclosed_materials() {
        let note = "Needs an enclosure; assuming this printer has one.".to_string();
        let unknown = PrinterFacts::unknown();
        for id in [Abs, Asa, Pa, PaCf, Pc] {
            let f = fit_for(material(id), &[Need::Strong], &unknown, None).unwrap();
            assert!(f.printer_notes.contains(&note), "{id:?}");
        }
        let f = fit_for(material(Pla), &[Need::Strong], &unknown, None).unwrap();
        assert!(!f.printer_notes.contains(&note));
        // A known enclosed printer needs no note.
        let f = fit_for(material(Abs), &[Need::Strong], &enclosed(), None).unwrap();
        assert!(f.printer_notes.is_empty());
        let f = fit_for(material(Petg), &[Need::Strong], &unknown, None).unwrap();
        assert!(f.material_notes.contains(&DRYING_NOTE.to_string()));
    }

    #[test]
    fn p1_and_p2_alone_are_not_guessed() {
        for m in ["P1", "P2", "X", "H2", "A", "A1 maxi"] {
            assert_eq!(enclosure_for_model(m), Enclosure::Unknown, "{m}");
        }
    }

    #[test]
    fn vendor_and_bundle_words_are_ignored() {
        assert_eq!(
            enclosure_for_model("Bambu Lab X1 Carbon"),
            Enclosure::Enclosed
        );
        assert_eq!(enclosure_for_model("P1S Combo"), Enclosure::Enclosed);
        assert_eq!(enclosure_for_model("Bambu Lab A1 mini"), Enclosure::Open);
        assert_eq!(enclosure_for_model("bambu lab a1 combo"), Enclosure::Open);
        assert_eq!(enclosure_for_model("A1 mini Combo"), Enclosure::Open);
        assert_eq!(enclosure_for_model("Bambu Lab P1P"), Enclosure::Open);
        assert_eq!(enclosure_for_model("Bambu Lab Ender 3"), Enclosure::Unknown);
    }

    #[test]
    fn exclusion_reasons_use_the_friendly_model_name() {
        let raw = PrinterFacts {
            model: Some("N2S".into()),
            enclosure: enclosure_for_model("N2S"),
            nozzle: NozzleKind::Unknown,
        };
        assert_eq!(raw.enclosure, Enclosure::Open);
        let out = fit_for(material(Asa), &[Need::Strong], &raw, None).unwrap_err();
        assert_eq!(out.reasons, ["Needs an enclosed printer; the A1 is open."]);
        let combo = PrinterFacts {
            model: Some("Bambu Lab A1 mini".into()),
            enclosure: Enclosure::Open,
            nozzle: NozzleKind::Unknown,
        };
        let out = fit_for(material(Asa), &[Need::Strong], &combo, None).unwrap_err();
        assert_eq!(
            out.reasons,
            ["Needs an enclosed printer; the A1 mini is open."]
        );
    }

    #[test]
    fn negated_nozzle_words_are_never_hardened() {
        let n = |t: &str| Nozzle {
            nozzle_type: Some(t.to_string()),
            ..Default::default()
        };
        for t in [
            "not_hardened",
            "non-hardened",
            "unhardened",
            "Not Hardened Steel",
        ] {
            assert_eq!(nozzle_kind(&[n(t)]), NozzleKind::NotHardened, "{t}");
        }
        // A name that says both is left unknown.
        assert_eq!(nozzle_kind(&[n("stainless_hardened")]), NozzleKind::Unknown);
        // Words must match whole: this is not "hardened".
        assert_eq!(nozzle_kind(&[n("hardenedish")]), NozzleKind::Unknown);
        assert_eq!(nozzle_kind(&[n("tungsten_carbide")]), NozzleKind::Hardened);
        assert_eq!(nozzle_kind(&[n("brass")]), NozzleKind::NotHardened);
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
        assert!(with.points > without.points);
        assert_eq!(with.total, without.total);
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
