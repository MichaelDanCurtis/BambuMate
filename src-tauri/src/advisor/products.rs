//! Which products to show for a material, in the spec's order, and the one
//! function every "Where to buy" link goes through.
//!
//! The catalog has no prices or ratings, so products are never ranked on
//! quality: AMS-loaded spools first, then preferred brands, then the rest
//! A–Z (brand, then name).

use serde::Serialize;

use super::materials::MaterialId;
use crate::printer::slots::SlotView;
use crate::scraper::catalog::CatalogEntry;

/// The `preferences.json` key holding the preferred brands, a JSON array.
pub const PREFERRED_BRANDS_KEY: &str = "preferred_brands";
pub const MAX_PREFERRED_BRANDS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductStatus {
    /// In an AMS slot now: "Loaded in A2".
    Loaded,
    /// From a brand chosen in Settings → General.
    Preferred,
    Catalog,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Product {
    pub name: String,
    pub brand: String,
    pub status: ProductStatus,
    /// The AMS slot label, for loaded spools.
    pub slot: Option<String>,
    /// From [`buy_link`].
    pub buy_url: Option<String>,
    /// What "Generate profile" hands to the Search flow.
    pub entry: Option<CatalogEntry>,
}

/// Upper-cased alphanumeric words: "PA6-CF" is `PA6`, `CF`.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_uppercase)
        .collect()
}

/// The material a product's text describes, or `None` for one the advisor
/// has no row for (support, glass fibre, PVA, PCTG, PCL, HIPS, CF blends of
/// ABS, ASA, TPU and PC).
///
/// Matching is on whole words, never substrings: "PCTG", "PCL" and
/// "Panchroma" are not PC or PA, "PETG Platinum" is PETG, and the carbon in
/// "Polycarbonate" is not carbon fibre.
pub fn classify(text: &str) -> Option<MaterialId> {
    let toks = tokens(text);
    let has = |w: &str| toks.iter().any(|t| t == w);
    let digits_after = |t: &str, prefix: &str| {
        t.strip_prefix(prefix)
            .is_some_and(|r| r.chars().all(|c| c.is_ascii_digit()))
    };
    if has("SUPPORT") || has("GLASS") || has("PCTG") || toks.iter().any(|t| digits_after(t, "GF")) {
        return None;
    }
    let carbon_fibre = toks
        .windows(2)
        .any(|w| w[0] == "CARBON" && matches!(w[1].as_str(), "FIBER" | "FIBRE"))
        || has("CARBONFIBER")
        || has("CARBONFIBRE")
        || has("CARBONX");
    let cf = carbon_fibre || toks.iter().any(|t| digits_after(t, "CF"));

    if has("TPU") || has("TPE") {
        return (!cf).then_some(MaterialId::Tpu);
    }
    if has("PLA") {
        let plus = text.to_ascii_uppercase().contains("PLA+")
            || ["PLUS", "PRO", "TOUGH"].iter().any(|k| has(k));
        return Some(if cf {
            MaterialId::PlaCf
        } else if plus {
            MaterialId::PlaPlus
        } else {
            MaterialId::Pla
        });
    }
    if has("PET") || has("PETG") {
        return Some(if cf {
            MaterialId::PetCf
        } else {
            MaterialId::Petg
        });
    }
    if has("ASA") {
        return (!cf).then_some(MaterialId::Asa);
    }
    if has("PC") || has("POLYCARBONATE") {
        return (!cf).then_some(MaterialId::Pc);
    }
    if has("ABS") {
        return (!cf).then_some(MaterialId::Abs);
    }
    if has("NYLON") || has("PAHT") || has("PA") || toks.iter().any(|t| digits_after(t, "PA")) {
        return Some(if cf { MaterialId::PaCf } else { MaterialId::Pa });
    }
    None
}

/// The catalog stores only the slug's first word as `material`, so the
/// name is read too.
pub fn material_of_entry(entry: &CatalogEntry) -> Option<MaterialId> {
    classify(&format!("{} {}", entry.material, entry.name))
}

pub fn material_of_tray(slot: &SlotView) -> Option<MaterialId> {
    let t = &slot.tray;
    if t.empty || t.tray_type.trim().is_empty() {
        return None;
    }
    classify(&format!("{} {}", t.tray_type, t.tray_sub_brands))
}

/// The link behind "Where to buy": in v1, the entry's own manufacturer or
/// SpoolScout URL with no affiliate tag, normalised by the URL parser. Only
/// plain `http(s)://host/...` links are offered: no credentials, no
/// whitespace or control characters, and the scheme separator must be
/// written out (the parser alone would accept `https:/x`).
///
/// Affiliate rule, for later: when affiliate tagging is added, links must
/// go through a redirect the user controls (never a third party's
/// tracker), and the page must show a visible disclosure. Products are
/// never ranked by commission. Every buy link goes through this function so
/// that can change here without touching the UI.
pub fn buy_link(entry: &CatalogEntry) -> Option<String> {
    let raw = entry.full_url.trim();
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    let ok = matches!(parsed.scheme(), "https" | "http")
        && parsed.host_str().is_some_and(|h| !h.is_empty())
        && parsed.username().is_empty()
        && parsed.password().is_none();
    ok.then(|| parsed.to_string())
}

/// Whether a catalog entry is the spool an AMS slot reports, going by the
/// tray's own brand text. A Bambu RFID spool is Bambu's. Any other tray must
/// name the entry's brand; a generic tray names none, so it matches nothing
/// (it must never borrow another product's name or link).
fn same_product(e: &CatalogEntry, sub_brands: &str, rfid: bool) -> bool {
    let brand = e.brand.trim().to_lowercase();
    let sub = sub_brands.trim().to_lowercase();
    let rest = if rfid {
        if !brand.contains("bambu") {
            return false;
        }
        sub.clone()
    } else {
        if brand.is_empty() || !sub.contains(&brand) {
            return false;
        }
        sub.replacen(&brand, "", 1).trim().to_string()
    };
    let name = e.name.trim().to_lowercase();
    let material = e.material.trim().to_lowercase();
    [sub.as_str(), rest.as_str()].iter().any(|t| {
        !t.is_empty()
            && (*t == name || *t == format!("{material} {name}") || *t == format!("{brand} {name}"))
    })
}

/// Products for `material`: AMS trays whose material matches, then catalog
/// entries from preferred brands, then the rest of the catalog's matches,
/// A–Z by brand then name.
///
/// A loaded tray links to a catalog entry only when it names that entry's
/// brand (Bambu RFID spools count as Bambu's). Otherwise it is listed under
/// its own tray text with no brand, `buy_url` or `entry`. While the catalog
/// has no Bambu Lab brand, Bambu RFID rows also come out without a
/// `buy_url`, so callers must not assume one on a `Loaded` product.
pub fn products_for(
    material: MaterialId,
    slots: &[SlotView],
    catalog: &[CatalogEntry],
    preferred: &[String],
) -> Vec<Product> {
    let matching: Vec<&CatalogEntry> = catalog
        .iter()
        .filter(|e| material_of_entry(e) == Some(material))
        .collect();
    let mut used: Vec<&CatalogEntry> = Vec::new();
    let mut out = Vec::new();

    for slot in slots
        .iter()
        .filter(|s| material_of_tray(s) == Some(material))
    {
        let sub = slot.tray.tray_sub_brands.trim();
        let name = if sub.is_empty() {
            slot.tray.tray_type.trim()
        } else {
            sub
        };
        let entry = matching
            .iter()
            .copied()
            .find(|e| same_product(e, sub, slot.rfid));
        if let Some(e) = entry {
            used.push(e);
        }
        let brand = match entry {
            Some(e) => e.brand.clone(),
            None if slot.rfid => "Bambu Lab".to_string(),
            None => String::new(),
        };
        out.push(Product {
            name: name.to_string(),
            brand,
            status: ProductStatus::Loaded,
            slot: Some(slot.label.clone()),
            buy_url: entry.and_then(buy_link),
            entry: entry.cloned(),
        });
    }

    let preferred = normalize_brands(preferred);
    let is_preferred = |e: &CatalogEntry| {
        preferred
            .iter()
            .any(|b| b.trim().eq_ignore_ascii_case(e.brand.trim()))
    };
    let mut rest: Vec<&CatalogEntry> = matching
        .into_iter()
        .filter(|e| !used.iter().any(|u| std::ptr::eq(*u, *e)))
        .collect();
    rest.sort_by_key(|e| {
        (
            !is_preferred(e),
            e.brand.trim().to_lowercase(),
            e.name.trim().to_lowercase(),
        )
    });
    for e in rest {
        out.push(Product {
            name: e.name.clone(),
            brand: e.brand.clone(),
            status: if is_preferred(e) {
                ProductStatus::Preferred
            } else {
                ProductStatus::Catalog
            },
            slot: None,
            buy_url: buy_link(e),
            entry: Some(e.clone()),
        });
    }
    out
}

/// Trimmed, without blanks or case-insensitive repeats (the first spelling
/// wins), in the order given, at most [`MAX_PREFERRED_BRANDS`].
pub fn normalize_brands(brands: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in brands {
        let t = b.trim();
        if t.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            continue;
        }
        out.push(t.to_string());
        if out.len() == MAX_PREFERRED_BRANDS {
            break;
        }
    }
    out
}

/// The stored value as brands. Anything but an array of strings reads as none.
pub fn brands_from_value(value: Option<&serde_json::Value>) -> Vec<String> {
    let list: Vec<String> = value
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    normalize_brands(&list)
}

pub fn brands_to_value(brands: &[String]) -> serde_json::Value {
    serde_json::json!(normalize_brands(brands))
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::printer::slots::{SlotStatus, SlotView};
    use crate::printer::state::Tray;
    use crate::scraper::catalog::CatalogEntry;

    pub fn slot(label: &str, tray_type: &str, sub: &str, rfid: bool) -> SlotView {
        SlotView {
            ams_id: 0,
            tray_id: 0,
            label: label.to_string(),
            tray: Tray {
                tray_type: tray_type.to_string(),
                tray_sub_brands: sub.to_string(),
                empty: tray_type.is_empty(),
                ..Default::default()
            },
            rfid,
            assigned_preset: None,
            assigned_filament_id: None,
            status: SlotStatus::Unassigned,
            needs_cloud_sync: false,
            preset_has_no_id: false,
        }
    }

    pub fn entry(brand: &str, material: &str, name: &str) -> CatalogEntry {
        let slug = |s: &str| s.to_lowercase().replace(' ', "-");
        CatalogEntry {
            brand: brand.to_string(),
            name: name.to_string(),
            material: material.to_string(),
            url_slug: slug(&format!("{material} {name}")),
            full_url: format!(
                "https://www.spoolscout.com/data-sheets/{}/{}",
                slug(brand),
                slug(name)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{entry, slot};
    use super::ProductStatus::*;
    use super::*;
    use crate::advisor::materials::MaterialId::*;

    #[test]
    fn product_names_classify_to_materials() {
        let cases = [
            ("PLA High Speed PLA", Some(Pla)),
            ("PLA Carbon Fiber", Some(PlaCf)),
            ("PLA+ Silk", Some(PlaPlus)),
            ("PLA Tough", Some(PlaPlus)),
            ("PETG-CF", Some(PetCf)),
            ("PET-CF", Some(PetCf)),
            ("PETG Basic", Some(Petg)),
            ("ABS", Some(Abs)),
            ("ASA Aero", Some(Asa)),
            ("TPU 95A", Some(Tpu)),
            ("PA6-CF", Some(PaCf)),
            ("PAHT-CF", Some(PaCf)),
            ("Nylon", Some(Pa)),
            ("PC", Some(Pc)),
            ("PLA-S Support for PLA", None),
            ("PA6-GF", None),
            ("PVA", None),
            ("PCTG", None),
            ("ABS-CF", None),
            ("Polycarbonate", Some(Pc)),
            ("PC Polycarbonate", Some(Pc)),
            ("PCL", None),
            ("Pcl", None),
            ("PC-ABS", Some(Pc)),
            ("PC-CF", None),
            ("PolyCast", None),
            ("Spectrum", None),
            ("Panchroma", None),
            ("Pastel", None),
            ("PLA Sparkle", Some(Pla)),
            ("Silk PLA", Some(Pla)),
            ("PLA+", Some(PlaPlus)),
            ("PLA Pro", Some(PlaPlus)),
            ("PLA Carbon Fiber", Some(PlaCf)),
            ("PETG HF", Some(Petg)),
            ("PETG Platinum", Some(Petg)),
            ("ABS Plain", Some(Abs)),
            ("PA12", Some(Pa)),
            ("PA12-CF", Some(PaCf)),
            ("ABS-GF", None),
            ("ASA-CF", None),
            ("ASA Aero", Some(Asa)),
            ("TPU for AMS", Some(Tpu)),
            ("Support for PLA", None),
            ("HIPS", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(classify(text), want, "{text}");
        }
    }

    #[test]
    fn ams_then_preferred_brands_then_the_rest_a_to_z() {
        let catalog = vec![
            entry("Polymaker", "PLA", "PolyLite PLA"),
            entry("Acme", "PLA", "Zeta"),
            entry("Bambu Lab", "PLA", "Basic"),
            entry("Acme", "PLA", "Alpha"),
            entry("Sunlu", "PLA", "Carbon Fiber"),
            entry("Polymaker", "PETG", "PolyLite PETG"),
        ];
        let slots = vec![
            slot("A1", "PLA-S", "Support for PLA", true),
            slot("A2", "PLA", "PLA Basic", true),
            slot("A3", "PETG", "", false),
            slot("B4", "", "", false),
        ];
        let got = products_for(Pla, &slots, &catalog, &[" polymaker ".to_string()]);
        let summary: Vec<(&str, &str, ProductStatus, Option<&str>)> = got
            .iter()
            .map(|p| {
                (
                    p.brand.as_str(),
                    p.name.as_str(),
                    p.status,
                    p.slot.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Bambu Lab", "PLA Basic", Loaded, Some("A2")),
                ("Polymaker", "PolyLite PLA", Preferred, None),
                ("Acme", "Alpha", Catalog, None),
                ("Acme", "Zeta", Catalog, None),
            ]
        );
        // The loaded spool matched its catalog entry, so it can be bought
        // and profiled, and that entry is not listed twice.
        assert_eq!(
            got[0].buy_url.as_deref(),
            Some("https://www.spoolscout.com/data-sheets/bambu-lab/basic")
        );
        assert_eq!(
            got[0].entry.as_ref().map(|e| e.brand.as_str()),
            Some("Bambu Lab")
        );
    }

    #[test]
    fn an_unmatched_ams_spool_is_still_listed_first() {
        let catalog = vec![entry("Polymaker", "PETG", "PolyLite PETG")];
        let got = products_for(Petg, &[slot("A3", "PETG", "", false)], &catalog, &[]);
        assert_eq!(got.len(), 2);
        assert_eq!(
            (got[0].status, got[0].slot.as_deref()),
            (Loaded, Some("A3"))
        );
        assert_eq!((got[0].name.as_str(), got[0].brand.as_str()), ("PETG", ""));
        assert_eq!(got[0].buy_url, None);
        assert_eq!(got[0].entry, None);
        assert_eq!(got[1].status, Catalog);
    }

    #[test]
    fn buy_links_are_the_plain_url() {
        let e = entry("Polymaker", "PLA", "PolyLite PLA");
        assert_eq!(buy_link(&e).as_deref(), Some(e.full_url.as_str()));
        for bad in [
            "",
            "javascript:alert(1)",
            "ftp://example.com/x",
            "not a url",
        ] {
            let e = crate::scraper::catalog::CatalogEntry {
                full_url: bad.to_string(),
                ..e.clone()
            };
            assert_eq!(buy_link(&e), None, "{bad}");
        }
    }

    #[test]
    fn buy_links_are_normalised_and_trimmed() {
        let e = entry("A", "PLA", "B");
        let with = |u: &str| crate::scraper::catalog::CatalogEntry {
            full_url: u.to_string(),
            ..e.clone()
        };
        assert_eq!(
            buy_link(&with("  HTTPS://Example.com/a?x=1 ")).as_deref(),
            Some("https://example.com/a?x=1")
        );
        assert_eq!(
            buy_link(&with("http://example.com")).as_deref(),
            Some("http://example.com/")
        );
    }

    #[test]
    fn a_generic_tray_never_borrows_a_catalog_product() {
        // The catalog has entries literally named after the material.
        let catalog = vec![
            entry("3d Fuel", "PETG", "PETG"),
            entry("Elegoo", "PETG", "PETG"),
        ];
        let got = products_for(Petg, &[slot("A1", "PETG", "", false)], &catalog, &[]);
        assert_eq!(got.len(), 3);
        assert_eq!(
            (got[0].status, got[0].slot.as_deref()),
            (Loaded, Some("A1"))
        );
        assert_eq!((got[0].name.as_str(), got[0].brand.as_str()), ("PETG", ""));
        assert_eq!((got[0].buy_url.clone(), got[0].entry.clone()), (None, None));
        // Both catalog entries are still listed.
        assert_eq!(got[1].brand, "3d Fuel");
        assert_eq!(got[2].brand, "Elegoo");

        // A named sub-brand that is not in the catalog also claims nothing.
        let got = products_for(
            Petg,
            &[slot("A1", "PETG", "Hatchbox PETG", false)],
            &catalog,
            &[],
        );
        assert_eq!((got[0].brand.as_str(), got[0].entry.clone()), ("", None));
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn a_tray_that_names_the_brand_links_to_it() {
        let catalog = vec![
            entry("Polymaker", "PLA", "PolyLite PLA"),
            entry("Elegoo", "PLA", "PLA"),
        ];
        let got = products_for(
            Pla,
            &[slot("A1", "PLA", "Polymaker PolyLite PLA", false)],
            &catalog,
            &[],
        );
        assert_eq!(got.len(), 2);
        assert_eq!(
            (got[0].status, got[0].brand.as_str()),
            (Loaded, "Polymaker")
        );
        assert!(got[0].buy_url.is_some() && got[0].entry.is_some());
        assert_eq!(got[1].brand, "Elegoo");
    }

    #[test]
    fn a_blank_preferred_brand_matches_nothing() {
        let catalog = vec![entry("", "PLA", "Mystery"), entry("Acme", "PLA", "Alpha")];
        let got = products_for(Pla, &[], &catalog, &["".to_string(), "   ".to_string()]);
        assert!(got.iter().all(|p| p.status == Catalog));
    }

    #[test]
    fn preferred_brands_order_among_themselves_and_ignore_case() {
        let catalog = vec![
            entry("acme", "PLA", "b"),
            entry("Zed", "PLA", "z"),
            entry("Yarn", "PLA", "y"),
            entry("Acme", "PLA", "a"),
            entry("Mid", "PLA", "m"),
        ];
        let got = products_for(Pla, &[], &catalog, &["zed".to_string(), "YARN".to_string()]);
        let order: Vec<(&str, ProductStatus)> =
            got.iter().map(|p| (p.name.as_str(), p.status)).collect();
        assert_eq!(
            order,
            vec![
                ("y", Preferred),
                ("z", Preferred),
                ("a", Catalog),
                ("b", Catalog),
                ("m", Catalog)
            ]
        );
    }

    #[test]
    fn two_slots_with_the_same_spool_are_both_listed_once_each() {
        let catalog = vec![entry("Bambu Lab", "PLA", "Basic")];
        let slots = [
            slot("A1", "PLA", "PLA Basic", true),
            slot("A2", "PLA", "PLA Basic", true),
        ];
        let got = products_for(Pla, &slots, &catalog, &[]);
        let labels: Vec<_> = got.iter().map(|p| p.slot.as_deref()).collect();
        assert_eq!(labels, vec![Some("A1"), Some("A2")]);
        assert!(got.iter().all(|p| p.status == Loaded && p.entry.is_some()));
    }

    #[test]
    fn blank_and_unknown_trays_are_skipped() {
        let slots = [
            slot("A1", "   ", "", false),
            slot("A2", "Unknown", "", false),
            slot("A3", "GENERIC", "", false),
        ];
        for s in &slots {
            assert_eq!(material_of_tray(s), None, "{}", s.label);
        }
        assert!(products_for(Pla, &slots, &[], &[]).is_empty());
    }

    #[test]
    fn preferred_brands_are_stored_and_read_back() {
        let input: Vec<String> = ["  Polymaker ", "polymaker", "", "Bambu Lab", "Prusament"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let stored = brands_to_value(&input);
        assert_eq!(
            stored,
            serde_json::json!(["Polymaker", "Bambu Lab", "Prusament"])
        );
        assert_eq!(
            brands_from_value(Some(&stored)),
            ["Polymaker", "Bambu Lab", "Prusament"]
        );
        assert!(brands_from_value(None).is_empty());
        assert!(brands_from_value(Some(&serde_json::json!("Polymaker"))).is_empty());
        let many: Vec<String> = (0..80).map(|i| format!("Brand {i}")).collect();
        assert_eq!(normalize_brands(&many).len(), MAX_PREFERRED_BRANDS);
    }
}
