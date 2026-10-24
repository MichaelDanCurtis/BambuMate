//! Pure formatting for Filament → Advise, tested on the host.

use std::fmt::Write as _;

use super::types::{
    Advice, Excluded, MaterialFit, Need, NeedReason, Product, ProductStatus, Verdict,
};
use crate::components::patterns::Tone;
use crate::viewport::types::{ModelView, RiskKind};

pub const ORDER_RULE: &str = "Loaded in your AMS first, then your preferred brands, then the rest A–Z. Products are never ranked on price, rating or commission.";
pub const AI_FAILED: &str = "Couldn't read the description; pick the needs below.";
pub const AI_OFF: &str = "AI isn't set up, so pick what the part needs below.";
pub const BEFORE_ADVISING: &str = "Describe the part or pick what it needs";
pub const READING: &str = "Reading your description";
/// Shown when `fits` is empty. The same text as the backend's
/// `score::NOTHING_FITS`.
pub const NOTHING_FITS: &str = "Nothing on this printer meets all of that.";
pub const CATALOG_EMPTY: &str = "The filament catalog is empty. Refresh it on the Search tab.";
/// Under a product whose "Where to buy" link the system would not open.
pub const BUY_FAILED: &str = "Couldn't open that link in your browser.";
/// Under the rank pills when the model's geometry changed the order.
pub const GEOMETRY_ORDER: &str = "Order also weighs this model's size and shape.";

/// Toggle one chip. The heat options are one choice: picking one clears
/// the others.
pub fn toggle_need(current: &[Need], need: Need) -> Vec<Need> {
    if current.contains(&need) {
        return current.iter().copied().filter(|n| *n != need).collect();
    }
    let mut out: Vec<Need> = current
        .iter()
        .copied()
        .filter(|n| !(need.is_heat() && n.is_heat()))
        .collect();
    out.push(need);
    out.sort();
    out
}

/// Toggle a preferred brand, ignoring case.
pub fn toggle_brand(current: &[String], brand: &str) -> Vec<String> {
    if current.iter().any(|b| b.eq_ignore_ascii_case(brand)) {
        current
            .iter()
            .filter(|b| !b.eq_ignore_ascii_case(brand))
            .cloned()
            .collect()
    } else {
        let mut out = current.to_vec();
        out.push(brand.to_string());
        out
    }
}

pub fn verdict_mark(v: Verdict) -> &'static str {
    match v {
        Verdict::Met => "✓",
        Verdict::Partly => "~",
        Verdict::NotMet => "✗",
    }
}

/// A reason row's value: `✓ 3/3`.
pub fn reason_value(r: &NeedReason) -> String {
    format!("{} {}", verdict_mark(r.verdict), r.detail)
}

/// `Fit 100% · ✓ Heat ≥80 °C · ✓ Outdoor/UV`: the fit and up to three met needs.
pub fn hero_detail(fit: &MaterialFit) -> String {
    let mut out = format!("Fit {}%", fit.fit_pct);
    for r in fit
        .reasons
        .iter()
        .filter(|r| r.verdict == Verdict::Met)
        .take(3)
    {
        let _ = write!(out, " · ✓ {}", r.need.label());
    }
    out
}

/// A ruled-out material's reasons on one line.
pub fn excluded_reasons(e: &Excluded) -> String {
    e.reasons.join("; ")
}

/// Whether Advise reads the description before scoring: only with AI, only
/// for a non-empty text, and only when it differs from the text last read.
/// Otherwise the chips as they are now are the needs, so an edit to them
/// survives the next press.
pub fn should_interpret(ai: bool, text: &str, last_read: Option<&str>) -> bool {
    let text = text.trim();
    ai && !text.is_empty() && last_read.map(str::trim) != Some(text)
}

/// The hero is signal-coloured only when the best fit misses no need.
/// Colour marks an event, not a default.
pub fn hero_tone(fit: &MaterialFit) -> Tone {
    if fit.reasons.iter().any(|r| r.verdict == Verdict::NotMet) {
        Tone::Neutral
    } else {
        Tone::Signal
    }
}

/// What the results' live region announces once an answer arrives.
pub fn result_summary(advice: &Advice) -> String {
    let Some((top, rest)) = advice.fits.split_first() else {
        return NOTHING_FITS.to_string();
    };
    let mut out = format!("Best fit {}, fit {}%.", top.name, top.fit_pct);
    if !rest.is_empty() {
        let names: Vec<&str> = rest.iter().map(|f| f.name.as_str()).collect();
        let _ = write!(out, " Also {}.", names.join(", "));
    }
    out
}

/// A ruled-out list's heading, with how many it holds.
pub fn ruled_out_heading(count: usize) -> String {
    format!("Ruled out ({count})")
}

/// A rank pill: `1 · PETG`.
pub fn rank_pill(index: usize, name: &str) -> String {
    format!("{} · {name}", index + 1)
}

/// The backend's material keys in its table order, the tie-break order
/// when two materials fit the same. A copy of the declaration order of
/// `MaterialId` in src-tauri/src/advisor/materials.rs (whose test
/// `every_material_is_listed_once_in_id_order` pins the table to it): the
/// backend sends neither the order nor the ranking points, and the two
/// crates share no code, so keep this in step by hand.
const MATERIAL_ORDER: &[&str] = &[
    "pla", "pla_plus", "pla_cf", "petg", "pet_cf", "abs", "asa", "tpu", "pa", "pa_cf", "pc",
];

/// Whether the shown order differs from the order by fit alone (the
/// needs-only `total`, ties in the table's order). Only a model's geometry
/// moves a material, so the page then says the order weighs it.
pub fn geometry_reordered(fits: &[MaterialFit]) -> bool {
    let place = |key: &str| MATERIAL_ORDER.iter().position(|m| *m == key);
    fits.windows(2).any(|pair| {
        let (a, b) = (&pair[0], &pair[1]);
        a.total < b.total
            || (a.total == b.total
                && matches!((place(&a.material), place(&b.material)), (Some(x), Some(y)) if x > y))
    })
}

/// A material's notes in its panel: the printer's, then the model's
/// geometry, then the material's own.
pub fn detail_notes(fit: &MaterialFit) -> Vec<String> {
    fit.printer_notes
        .iter()
        .chain(&fit.geometry_notes)
        .chain(&fit.material_notes)
        .cloned()
        .collect()
}

pub fn status_tag(p: &Product) -> Option<String> {
    match p.status {
        ProductStatus::Loaded => Some(match &p.slot {
            Some(slot) => format!("Loaded in {slot}"),
            None => "Loaded".to_string(),
        }),
        ProductStatus::Preferred => Some("Preferred".to_string()),
        ProductStatus::Catalog => None,
    }
}

/// One decimal under 10 (`6.0`), whole above (`12`), as the risk list does.
fn ratio(v: f32) -> String {
    if v < 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    }
}

/// `10 × 10 mm · Overhang 64 mm² · Tall and thin 6.0:1`: the footprint, the
/// overhang area summed over its regions, then the other risks.
pub fn model_line(view: &ModelView) -> String {
    let size = |i: usize| (view.bounds_max[i] - view.bounds_min[i]).max(0.0);
    let mut parts = vec![format!("{:.0} × {:.0} mm", size(0), size(1))];
    let overhangs: Vec<f32> = view
        .risks
        .iter()
        .filter(|r| r.kind == RiskKind::Overhang)
        .map(|r| r.value)
        .collect();
    if !overhangs.is_empty() {
        parts.push(format!("Overhang {:.0} mm²", overhangs.iter().sum::<f32>()));
    }
    for r in &view.risks {
        match r.kind {
            RiskKind::SmallBedContact => {
                parts.push(format!("Small bed contact {:.0} mm²", r.value))
            }
            RiskKind::TallThin => parts.push(format!("Tall and thin {}:1", ratio(r.value))),
            RiskKind::Overhang => {}
        }
    }
    parts.join(" · ")
}

/// The Advise route with `model` set, as Slice's "Suggest filament" links it.
pub fn advise_href(path: &str) -> String {
    let mut out = String::from("/filament/advise?model=");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advisor::types::{
        Advice, Excluded, MaterialFit, Need::*, NeedReason, Product, ProductStatus, Verdict,
    };
    use crate::components::patterns::Tone;

    fn reason(need: Need, verdict: Verdict, detail: &str) -> NeedReason {
        NeedReason {
            need,
            verdict,
            detail: detail.into(),
        }
    }
    fn fit(pct: u8, reasons: Vec<NeedReason>) -> MaterialFit {
        MaterialFit {
            material: "pa_cf".into(),
            name: "PA-CF".into(),
            total: 5,
            fit_pct: pct,
            reasons,
            printer_notes: vec![],
            geometry_notes: vec![],
            material_notes: vec![],
            products: vec![],
        }
    }
    fn product(status: ProductStatus, slot: Option<&str>) -> Product {
        Product {
            name: "ASA".into(),
            brand: "Bambu Lab".into(),
            status,
            slot: slot.map(str::to_string),
            buy_url: None,
            entry: None,
        }
    }

    #[test]
    fn the_hero_detail_gives_the_fit_and_the_ticks() {
        let f = fit(
            83,
            vec![
                reason(Heat80, Verdict::Met, "heat deflection ≈120 °C"),
                reason(OutdoorUv, Verdict::Partly, "2/3"),
            ],
        );
        assert_eq!(hero_detail(&f), "Fit 83% · ✓ Heat ≥80 °C");
        let both = fit(
            100,
            vec![
                reason(Heat80, Verdict::Met, "x"),
                reason(OutdoorUv, Verdict::Met, "3/3"),
            ],
        );
        assert_eq!(
            hero_detail(&both),
            "Fit 100% · ✓ Heat ≥80 °C · ✓ Outdoor/UV"
        );
        assert_eq!(
            hero_detail(&fit(0, vec![reason(Flexible, Verdict::NotMet, "0/3")])),
            "Fit 0%"
        );
    }

    #[test]
    fn reasons_read_as_a_mark_and_a_detail() {
        assert_eq!(
            reason_value(&reason(OutdoorUv, Verdict::Met, "3/3")),
            "✓ 3/3"
        );
        assert_eq!(
            reason_value(&reason(OutdoorUv, Verdict::Partly, "2/3")),
            "~ 2/3"
        );
        assert_eq!(
            reason_value(&reason(Flexible, Verdict::NotMet, "0/3")),
            "✗ 0/3"
        );
    }

    #[test]
    fn heat_chips_exclude_each_other() {
        let n = toggle_need(&[], Strong);
        assert_eq!(n, [Strong]);
        let n = toggle_need(&toggle_need(&n, Heat60), Heat100);
        assert_eq!(n, [Strong, Heat100]);
        assert_eq!(toggle_need(&n, Heat100), [Strong]);
        assert_eq!(toggle_need(&[OutdoorUv], Heat80), [Heat80, OutdoorUv]);
    }

    #[test]
    fn brands_toggle_ignoring_case() {
        let b = toggle_brand(&[], "Polymaker");
        assert_eq!(b, ["Polymaker"]);
        assert!(toggle_brand(&b, "polymaker").is_empty());
    }

    #[test]
    fn status_tags_name_the_slot() {
        assert_eq!(
            status_tag(&product(ProductStatus::Loaded, Some("A2"))).as_deref(),
            Some("Loaded in A2")
        );
        assert_eq!(
            status_tag(&product(ProductStatus::Preferred, None)).as_deref(),
            Some("Preferred")
        );
        assert_eq!(status_tag(&product(ProductStatus::Catalog, None)), None);
    }

    #[test]
    fn ruled_out_reasons_join_with_semicolons() {
        let e = |reasons: &[&str]| Excluded {
            material: "pc".into(),
            name: "PC".into(),
            reasons: reasons.iter().map(|r| r.to_string()).collect(),
        };
        assert_eq!(
            excluded_reasons(&e(&[
                "Needs an enclosed printer; the A1 is open.",
                "Softens around 110 °C"
            ])),
            "Needs an enclosed printer; the A1 is open.; Softens around 110 °C"
        );
        assert_eq!(
            excluded_reasons(&e(&["Softens around 55 °C"])),
            "Softens around 55 °C"
        );
        assert_eq!(excluded_reasons(&e(&[])), "");
    }

    #[test]
    fn rank_pills_number_the_material() {
        assert_eq!(rank_pill(0, "PETG"), "1 · PETG");
        assert_eq!(rank_pill(2, "PLA"), "3 · PLA");
    }

    fn ranked(material: &str, total: u32) -> MaterialFit {
        MaterialFit {
            material: material.into(),
            total,
            fit_pct: (total * 100 / 6) as u8,
            ..fit(0, vec![])
        }
    }

    #[test]
    fn the_order_note_shows_only_when_geometry_moved_a_material() {
        // By fit, ties in the table's order: as the backend ranks without a model.
        assert!(!geometry_reordered(&[
            ranked("asa", 6),
            ranked("pla", 4),
            ranked("abs", 4)
        ]));
        assert!(!geometry_reordered(&[
            ranked("pla", 4),
            ranked("abs", 4),
            ranked("pc", 4)
        ]));
        assert!(!geometry_reordered(&[]));
        assert!(!geometry_reordered(&[ranked("tpu", 3)]));
        // A lower fit ranked above a higher one.
        assert!(geometry_reordered(&[ranked("petg", 3), ranked("abs", 5)]));
        // An equal fit out of the table's order: ABS comes before PC.
        assert!(geometry_reordered(&[ranked("pc", 4), ranked("abs", 4)]));
        assert!(geometry_reordered(&[
            ranked("asa", 6),
            ranked("pa_cf", 4),
            ranked("pla_plus", 4)
        ]));
        assert_eq!(
            GEOMETRY_ORDER,
            "Order also weighs this model's size and shape."
        );
    }

    #[test]
    fn a_panel_lists_printer_then_geometry_then_material_notes() {
        let f = MaterialFit {
            printer_notes: vec!["Needs a hardened nozzle".into()],
            geometry_notes: vec!["Supports likely, whatever the material".into()],
            material_notes: vec!["Dry it first".into()],
            ..fit(50, vec![])
        };
        assert_eq!(
            detail_notes(&f),
            [
                "Needs a hardened nozzle",
                "Supports likely, whatever the material",
                "Dry it first"
            ]
        );
        assert!(detail_notes(&fit(50, vec![])).is_empty());
    }

    #[test]
    fn advise_reads_the_description_only_when_it_is_new() {
        // First press with a description and AI: read it.
        assert!(should_interpret(true, "bike rack bracket", None));
        // The same text again (spaces aside): score the chips as they are.
        assert!(!should_interpret(
            true,
            "  bike rack bracket ",
            Some("bike rack bracket")
        ));
        // An edited description is read again.
        assert!(should_interpret(
            true,
            "bike rack bracket, outdoors",
            Some("bike rack bracket")
        ));
        // No text, or no AI: never.
        assert!(!should_interpret(true, "   ", None));
        assert!(!should_interpret(false, "bike rack bracket", None));
    }

    #[test]
    fn the_hero_is_signal_only_with_no_unmet_need() {
        let all_met = fit(
            100,
            vec![
                reason(Strong, Verdict::Met, "3/3"),
                reason(OutdoorUv, Verdict::Partly, "2/3"),
            ],
        );
        assert_eq!(hero_tone(&all_met), Tone::Signal);
        let one_missed = fit(
            56,
            vec![
                reason(Strong, Verdict::Met, "3/3"),
                reason(Flexible, Verdict::NotMet, "0/3"),
            ],
        );
        assert_eq!(hero_tone(&one_missed), Tone::Neutral);
    }

    #[test]
    fn the_result_summary_names_the_picks() {
        let advice = |fits: Vec<MaterialFit>| Advice {
            needs: vec![],
            fits,
            ruled_out: vec![],
            printer_line: None,
            model_error: None,
            catalog_empty: false,
        };
        let named = |name: &str, pct: u8| MaterialFit {
            name: name.into(),
            ..fit(pct, vec![])
        };
        assert_eq!(result_summary(&advice(vec![])), NOTHING_FITS);
        assert_eq!(
            result_summary(&advice(vec![named("ASA", 56)])),
            "Best fit ASA, fit 56%."
        );
        assert_eq!(
            result_summary(&advice(vec![
                named("ASA", 56),
                named("PETG", 44),
                named("PC", 56)
            ])),
            "Best fit ASA, fit 56%. Also PETG, PC."
        );
    }

    #[test]
    fn the_ruled_out_heading_counts_them() {
        assert_eq!(ruled_out_heading(3), "Ruled out (3)");
        assert_eq!(ruled_out_heading(1), "Ruled out (1)");
    }

    #[test]
    fn the_model_line_shows_the_footprint_and_the_risks() {
        let view: ModelView = serde_json::from_str(
            r#"{"name":"cube.stl","triangle_count":12,"bounds_min":[0,0,0],"bounds_max":[10,10,10],
                "dots":"","dot_normals":"","edges":"",
                "risks":[{"kind":"overhang","position":[0,0,0],"value":40,"message":"m"},
                         {"kind":"overhang","position":[0,0,0],"value":24,"message":"m"},
                         {"kind":"tall_thin","position":[0,0,0],"value":6,"message":"m"}]}"#,
        )
        .unwrap();
        assert_eq!(
            model_line(&view),
            "10 × 10 mm · Overhang 64 mm² · Tall and thin 6.0:1"
        );
        let wide: ModelView = serde_json::from_str(
            r#"{"name":"plate.stl","triangle_count":12,"bounds_min":[0,0,0],"bounds_max":[200,120,2],
                "dots":"","dot_normals":"","edges":"",
                "risks":[{"kind":"small_bed_contact","position":[0,0,0],"value":3.4,"message":"m"},
                         {"kind":"tall_thin","position":[0,0,0],"value":12,"message":"m"}]}"#,
        )
        .unwrap();
        assert_eq!(
            model_line(&wide),
            "200 × 120 mm · Small bed contact 3 mm² · Tall and thin 12:1"
        );
    }

    #[test]
    fn advise_links_encode_the_path() {
        assert_eq!(
            advise_href("/Users/me/My Parts/bracket #2.stl"),
            "/filament/advise?model=%2FUsers%2Fme%2FMy%20Parts%2Fbracket%20%232.stl"
        );
        assert_eq!(
            advise_href("C:\\Parts\\é.3mf"),
            "/filament/advise?model=C%3A%5CParts%5C%C3%A9.3mf"
        );
    }

    /// Percent-decode the way the router's query reader does, to prove the
    /// link round-trips.
    fn decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn advise_links_survive_reserved_characters() {
        for path in [
            "/a b/c&d=e.stl",
            "/p#1/100%.stl",
            "/q?x=1/%41.3mf",
            "/plus+sign/a;b,c.stl",
            "/Users/me/Parts & Bits/100% #1.stl",
        ] {
            let href = advise_href(path);
            let query = href.strip_prefix("/filament/advise?model=").unwrap();
            // Nothing that could end or split the query value stays raw.
            assert!(
                !query.contains(['&', '#', '?', ' ', '+', '=', ';', ',', '/']),
                "{query}"
            );
            assert_eq!(decode(query), path);
        }
        assert_eq!(
            advise_href("/a b/c&d=e.stl"),
            "/filament/advise?model=%2Fa%20b%2Fc%26d%3De.stl"
        );
        assert_eq!(
            advise_href("/p#1/100%.stl"),
            "/filament/advise?model=%2Fp%231%2F100%25.stl"
        );
    }

    #[test]
    fn the_copy_matches_the_spec() {
        assert_eq!(NOTHING_FITS, "Nothing on this printer meets all of that.");
        assert_eq!(BEFORE_ADVISING, "Describe the part or pick what it needs");
        assert_eq!(READING, "Reading your description");
        assert_eq!(
            AI_FAILED,
            "Couldn't read the description; pick the needs below."
        );
        assert!(ORDER_RULE.starts_with(
            "Loaded in your AMS first, then your preferred brands, then the rest A–Z."
        ));
    }
}
