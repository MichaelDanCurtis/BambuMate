//! The filament ID index: which installed preset a filament ID the printer
//! reports is. It is built from Bambu Studio's system filament presets and
//! the user's own. Each is resolved through `inherits` to its real
//! `filament_id` with the slicer's [`PresetIndex::flatten`], so a variant
//! such as `Bambu PLA Basic @BBL H2C` gets GFA00 from its `@base`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::model_key;
use crate::slicer::settings::{PresetEntry, PresetIndex, PresetKind, PresetSource};

/// The installed preset a slot's filament ID names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detected {
    /// The full name, e.g. `Bambu PETG HF @BBL H2D`. Cards hide the `@…` part.
    pub preset_name: String,
    /// The preset's JSON file. "Use this" hands it to `printer_assign_slot`.
    pub path: String,
    /// A user preset rather than one of Bambu Studio's system presets.
    pub is_user: bool,
    /// Every preset with this ID from the same source that fits this printer
    /// and nozzle (one per nozzle when the nozzle is unknown), by name.
    pub variants: Vec<String>,
}

/// The printer the index picks variants for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrinterTarget {
    /// The model as the user or the printer wrote it: `H2D`, `X1C`,
    /// `Bambu Lab X1 Carbon`, a discovery code. Any spelling is accepted
    /// ([`model_key`]). Empty when unknown.
    pub model: String,
    /// The active nozzle's diameter in mm. When known, only presets for that
    /// nozzle count; when `None`, the standard 0.4 is preferred.
    pub nozzle: Option<f64>,
}

/// What the index knows about one reported filament ID.
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    /// An installed preset for this printer and nozzle.
    Found(Detected),
    /// Presets with this ID are installed for this printer, but none for the
    /// configured nozzle. `nozzles` are the diameters (mm, ascending) they
    /// are made for. Only possible while the nozzle is known.
    OtherNozzle { nozzles: Vec<f64> },
    /// No installed preset with this ID is for this printer (or none exists).
    NotInstalled,
}

/// Filament ID (upper-cased) → the preset chosen for the printer.
#[derive(Debug, Default)]
pub struct FilamentIndex {
    by_id: HashMap<String, Detected>,
    /// IDs with presets for this printer but not for the configured nozzle,
    /// and the nozzles those are for.
    other_nozzle: HashMap<String, Vec<f64>>,
    /// The nozzle (mm) the index was built for. `None` when unknown.
    nozzle: Option<f64>,
}

struct Candidate {
    entry: PresetEntry,
    compatible: Vec<String>,
}

impl FilamentIndex {
    /// Flattens every selectable filament preset in `presets`, already
    /// loaded, and picks one per ID for `target`. CPU-bound: run it on a
    /// blocking thread.
    pub fn build(presets: &PresetIndex, target: &PrinterTarget) -> Self {
        let mut groups: HashMap<String, Vec<Candidate>> = HashMap::new();
        for entry in presets.filament_entries() {
            let Ok(flat) = presets.flatten(PresetKind::Filament, &entry.name) else {
                continue;
            };
            let Some(id) = flat
                .config
                .get("filament_id")
                .and_then(text)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let compatible = presets.compatible_printers(PresetKind::Filament, &entry.name);
            groups
                .entry(id.to_ascii_uppercase())
                .or_default()
                .push(Candidate { entry, compatible });
        }
        // A model no preset names (a code, a typo, nothing) filters nothing.
        let model = model_key(&target.model);
        let model_known = !model.is_empty()
            && groups.values().flatten().any(|c| {
                c.compatible
                    .iter()
                    .any(|p| machine(p).is_some_and(|(m, _)| model_key(m) == model))
            });
        let want = Want {
            model: model_known.then_some(model),
            nozzle: target.nozzle,
        };
        let any_nozzle = Want {
            model: want.model.clone(),
            nozzle: None,
        };
        let mut by_id = HashMap::new();
        let mut other_nozzle = HashMap::new();
        for (id, candidates) in groups {
            match choose(&candidates, &want) {
                Some(d) => {
                    by_id.insert(id, d);
                }
                None if want.nozzle.is_some() => {
                    let nozzles = nozzles_for(&candidates, &any_nozzle);
                    if !nozzles.is_empty() {
                        other_nozzle.insert(id, nozzles);
                    }
                }
                None => {}
            }
        }
        Self {
            by_id,
            other_nozzle,
            nozzle: target.nozzle,
        }
    }

    /// The preset for a reported ID. `None` for an empty ID, or for one no
    /// installed preset for this printer and nozzle has. See
    /// [`FilamentIndex::lookup_outcome`] to tell "wrong nozzle" from "none".
    pub fn lookup(&self, id: &str) -> Option<Detected> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        self.by_id.get(&id.to_ascii_uppercase()).cloned()
    }

    /// Like [`FilamentIndex::lookup`], but says why nothing was found: a
    /// preset exists for this printer, only not for the configured nozzle.
    pub fn lookup_outcome(&self, id: &str) -> Lookup {
        if let Some(d) = self.lookup(id) {
            return Lookup::Found(d);
        }
        match self.other_nozzle.get(&id.trim().to_ascii_uppercase()) {
            Some(nozzles) if !id.trim().is_empty() => Lookup::OtherNozzle {
                nozzles: nozzles.clone(),
            },
            _ => Lookup::NotInstalled,
        }
    }

    /// The nozzle diameter (mm) the index was built for.
    pub fn nozzle(&self) -> Option<f64> {
        self.nozzle
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// A setting as text: a string, or the first of a list.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Array(a) => a.first().and_then(text),
        _ => None,
    }
}

/// `Bambu Lab H2D 0.4 nozzle` → (`H2D`, 0.4).
fn machine(name: &str) -> Option<(&str, f64)> {
    let rest = name.strip_prefix("Bambu Lab ")?.strip_suffix(" nozzle")?;
    let (model, diameter) = rest.rsplit_once(' ')?;
    Some((model, diameter.parse().ok()?))
}

/// What a preset has to fit.
///
/// A known nozzle excludes presets made for another nozzle of the same
/// printer. The one exception: when the printer is unrecognised (`model` is
/// `None`), or a preset's `compatible_printers` names can't be parsed, there
/// is nothing to say whose nozzle list applies, so the nozzle only ranks.
#[derive(Clone)]
struct Want {
    /// The printer's [`model_key`]. `None` when no preset names the printer
    /// (a code, a typo, nothing): the printer then filters nothing.
    model: Option<String>,
    /// The configured nozzle; `None` is read as the standard 0.4 when ranking.
    nozzle: Option<f64>,
}

/// The nozzle every printer ships with.
const DEFAULT_NOZZLE: f64 = 0.4;

/// How well a preset fits: 2 for the nozzle (the configured one, else 0.4),
/// 1 for the configured printer with another nozzle (only while the nozzle
/// is unknown), `None` for another printer, or for a known nozzle the preset
/// isn't made for. A preset for every printer fits at 1.
fn fit(compatible: &[String], want: &Want) -> Option<u8> {
    if compatible.is_empty() {
        return Some(1);
    }
    let diameter = want.nozzle.unwrap_or(DEFAULT_NOZZLE);
    let mut best = None;
    let mut parsed = false;
    for (model, d) in compatible.iter().filter_map(|p| machine(p)) {
        parsed = true;
        if want.model.as_deref().is_some_and(|m| model_key(model) != m) {
            continue;
        }
        if (diameter - d).abs() < 1e-6 {
            best = best.max(Some(2));
        } else if want.model.is_some() && want.nozzle.is_none() {
            best = best.max(Some(1));
        }
    }
    match (want.model.as_ref(), parsed) {
        // The printer is unknown: nothing is excluded, the nozzle only ranks.
        (None, _) => best.or(Some(1)),
        // Names this code can't read (a custom printer): not ours to judge.
        (Some(_), false) => Some(1),
        _ => best,
    }
}

/// The nozzle diameters (mm, ascending) of the presets that fit `want`'s
/// printer, whatever the nozzle.
fn nozzles_for(candidates: &[Candidate], want: &Want) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for c in candidates {
        if fit(&c.compatible, want).is_none() {
            continue;
        }
        for (model, d) in c.compatible.iter().filter_map(|p| machine(p)) {
            if want.model.as_deref().is_none_or(|m| model_key(model) == m)
                && !out.iter().any(|o| (o - d).abs() < 1e-6)
            {
                out.push(d);
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

/// System presets first, user presets only when no system preset fits.
/// Within a source: the best fit, then the first by name. `variants` are the
/// presets of that source that fit at all.
fn choose(candidates: &[Candidate], want: &Want) -> Option<Detected> {
    let mut candidates: Vec<&Candidate> = candidates.iter().collect();
    candidates.sort_by(|a, b| a.entry.name.cmp(&b.entry.name));
    for source in [PresetSource::System, PresetSource::User] {
        let fitting: Vec<(u8, &Candidate)> = candidates
            .iter()
            .copied()
            .filter(|c| c.entry.source == source)
            .filter_map(|c| fit(&c.compatible, want).map(|f| (f, c)))
            .collect();
        let best = fitting
            .iter()
            .max_by(|(fa, a), (fb, b)| fa.cmp(fb).then_with(|| b.entry.name.cmp(&a.entry.name)));
        if let Some((_, c)) = best {
            return Some(Detected {
                preset_name: c.entry.name.clone(),
                path: c.entry.path.to_string_lossy().into_owned(),
                is_user: source == PresetSource::User,
                variants: fitting.iter().map(|(_, c)| c.entry.name.clone()).collect(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::settings::tests::fake_bambu_root;
    use serde_json::json;
    use std::path::Path;

    const USER: &str = "1881310893";

    fn write(root: &Path, rel: &str, v: Value) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }

    fn index(root: &Path, model: &str, nozzle: Option<f64>) -> FilamentIndex {
        let presets = PresetIndex::load(root, Some(USER));
        FilamentIndex::build(
            &presets,
            &PrinterTarget {
                model: model.into(),
                nozzle,
            },
        )
    }

    #[test]
    fn a_system_variant_gets_its_id_through_inherits() {
        let root = fake_bambu_root();
        let idx = index(root.path(), "H2C", Some(0.4));
        let d = idx.lookup("GFA00").unwrap();
        assert_eq!(d.preset_name, "Bambu PLA Basic @BBL H2C");
        assert!(!d.is_user);
        assert!(
            d.path.ends_with("Bambu PLA Basic @BBL H2C.json"),
            "{}",
            d.path
        );
        assert_eq!(
            d.variants,
            vec!["Bambu PLA Basic @BBL H2C"],
            "the H2S variant is for another printer"
        );
        assert_eq!(
            idx.lookup(" gfa00 "),
            Some(d),
            "IDs compare without case or spaces"
        );
    }

    #[test]
    fn the_variant_for_the_configured_printer_and_nozzle_is_chosen() {
        let root = fake_bambu_root();
        write(
            root.path(),
            "system/BBL/filament/Bambu PLA Basic @BBL H2C 0.2 nozzle.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @BBL H2C 0.2 nozzle",
                "inherits":"Bambu PLA Basic @base","from":"system","instantiation":"true",
                "compatible_printers":["Bambu Lab H2C 0.2 nozzle"]
            }),
        );
        let name = |model: &str, nozzle: Option<f64>| {
            index(root.path(), model, nozzle)
                .lookup("GFA00")
                .unwrap()
                .preset_name
        };
        assert_eq!(name("H2S", Some(0.4)), "Bambu PLA Basic @BBL H2S");
        assert_eq!(name("H2C", Some(0.4)), "Bambu PLA Basic @BBL H2C");
        assert_eq!(
            name("h2c", Some(0.2)),
            "Bambu PLA Basic @BBL H2C 0.2 nozzle",
            "the nozzle's own variant wins"
        );
        assert_eq!(name("H2C", None), "Bambu PLA Basic @BBL H2C");
    }

    /// A system filament preset made for the listed machines.
    fn sys(root: &Path, name: &str, id: &str, compatible: &[&str]) {
        write(
            root,
            &format!("system/BBL/filament/{name}.json"),
            json!({
                "type":"filament","name":name,"from":"system","instantiation":"true",
                "filament_id":id,"filament_type":["PLA"],"compatible_printers":compatible
            }),
        );
    }

    fn picked(idx: &FilamentIndex, id: &str) -> Option<String> {
        idx.lookup(id).map(|d| d.preset_name)
    }

    #[test]
    fn a_known_nozzle_without_a_variant_is_not_installed() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Bambu TPU 85A @BBL H2C",
            "GFU04",
            &["Bambu Lab H2C 0.6 nozzle", "Bambu Lab H2C 0.8 nozzle"],
        );
        assert_eq!(picked(&index(root.path(), "H2C", Some(0.4)), "GFU04"), None);
        assert_eq!(
            picked(&index(root.path(), "H2C", Some(0.6)), "GFU04"),
            Some("Bambu TPU 85A @BBL H2C".into())
        );
        assert_eq!(
            picked(&index(root.path(), "H2C", None), "GFU04"),
            Some("Bambu TPU 85A @BBL H2C".into()),
            "an unknown nozzle excludes nothing"
        );
        let idx = index(root.path(), "H2C", Some(0.6));
        assert_eq!(
            picked(&idx, "GFA00"),
            None,
            "the 0.4-only PLA is not installed for a 0.6 nozzle"
        );
    }

    #[test]
    fn an_unknown_nozzle_prefers_the_standard_04_over_a_smaller_name() {
        let root = fake_bambu_root();
        for (name, nozzle) in [
            ("Aaa PETG @BBL H2D 0.2 nozzle", "0.2"),
            ("Bbb PETG @BBL H2D 0.6 nozzle", "0.6"),
            ("Ccc PETG @BBL H2D", "0.4"),
            ("Ddd PETG @BBL H2D 0.8 nozzle", "0.8"),
        ] {
            sys(
                root.path(),
                name,
                "GFG00",
                &[&format!("Bambu Lab H2D {nozzle} nozzle")],
            );
        }
        let idx = index(root.path(), "H2D", None);
        let d = idx.lookup("GFG00").unwrap();
        assert_eq!(d.preset_name, "Ccc PETG @BBL H2D");
        assert_eq!(
            d.variants.len(),
            4,
            "every nozzle's variant fits while the nozzle is unknown"
        );
        assert_eq!(
            picked(&index(root.path(), "H2D", Some(0.6)), "GFG00"),
            Some("Bbb PETG @BBL H2D 0.6 nozzle".into())
        );
    }

    #[test]
    fn only_variants_that_fit_the_printer_and_nozzle_are_listed() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Bambu PLA Basic @BBL H2C 0.2 nozzle",
            "GFA00",
            &["Bambu Lab H2C 0.2 nozzle"],
        );
        let d = index(root.path(), "H2C", Some(0.4))
            .lookup("GFA00")
            .unwrap();
        assert_eq!(d.variants, vec!["Bambu PLA Basic @BBL H2C"]);
        let d = index(root.path(), "H2C", Some(0.2))
            .lookup("GFA00")
            .unwrap();
        assert_eq!(d.variants, vec!["Bambu PLA Basic @BBL H2C 0.2 nozzle"]);
    }

    #[test]
    fn the_nozzle_beats_the_name_order_for_04_too() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Aaa PLA @BBL H2C 0.2 nozzle",
            "GFA00",
            &["Bambu Lab H2C 0.2 nozzle"],
        );
        assert_eq!(
            picked(&index(root.path(), "H2C", Some(0.4)), "GFA00"),
            Some("Bambu PLA Basic @BBL H2C".into())
        );
        assert_eq!(
            picked(&index(root.path(), "H2C", Some(0.2)), "GFA00"),
            Some("Aaa PLA @BBL H2C 0.2 nozzle".into())
        );
    }

    #[test]
    fn a_nozzle_matches_inside_a_list_of_machines() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Bambu ABS @BBL X1C",
            "GFB00",
            &[
                "Bambu Lab X1 Carbon 0.4 nozzle",
                "Bambu Lab X1E 0.4 nozzle",
                "Bambu Lab X1 Carbon 0.6 nozzle",
            ],
        );
        sys(
            root.path(),
            "Aaa ABS @BBL X1C 0.2 nozzle",
            "GFB00",
            &["Bambu Lab X1 Carbon 0.2 nozzle"],
        );
        for (model, nozzle, want) in [
            ("X1 Carbon", Some(0.6), Some("Bambu ABS @BBL X1C")),
            ("X1E", Some(0.4), Some("Bambu ABS @BBL X1C")),
            ("X1E", Some(0.6), None),
            ("X1C", Some(0.2), Some("Aaa ABS @BBL X1C 0.2 nozzle")),
        ] {
            assert_eq!(
                picked(&index(root.path(), model, nozzle), "GFB00").as_deref(),
                want,
                "{model} {nozzle:?}"
            );
        }
    }

    #[test]
    fn any_spelling_of_the_model_finds_its_presets() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "PLA @X1C",
            "GFA01",
            &["Bambu Lab X1 Carbon 0.4 nozzle"],
        );
        sys(
            root.path(),
            "PLA @A1M",
            "GFA01",
            &["Bambu Lab A1 mini 0.4 nozzle"],
        );
        sys(
            root.path(),
            "PLA @H2D",
            "GFA01",
            &["Bambu Lab H2D 0.4 nozzle"],
        );
        sys(
            root.path(),
            "PLA @H2DP",
            "GFA01",
            &["Bambu Lab H2D Pro 0.4 nozzle"],
        );
        for (model, want) in [
            ("X1C", "PLA @X1C"),
            ("X1 Carbon", "PLA @X1C"),
            ("Bambu Lab X1 Carbon", "PLA @X1C"),
            ("x1c combo", "PLA @X1C"),
            ("A1 mini", "PLA @A1M"),
            ("A1M", "PLA @A1M"),
            ("Bambu Lab A1 mini", "PLA @A1M"),
            ("N1", "PLA @A1M"),
            ("H2D", "PLA @H2D"),
            ("Bambu Lab H2D", "PLA @H2D"),
            ("H2D Pro", "PLA @H2DP"),
            ("H2DP", "PLA @H2DP"),
            ("O1E", "PLA @H2DP"),
            ("Bambu Lab H2D Pro 0.4 nozzle", "PLA @H2DP"),
        ] {
            assert_eq!(
                picked(&index(root.path(), model, Some(0.4)), "GFA01").as_deref(),
                Some(want),
                "{model}"
            );
        }
        for model in ["Bambu Lab H2C", "H2C Combo"] {
            assert_eq!(
                index(root.path(), model, Some(0.4))
                    .lookup("GFA00")
                    .unwrap()
                    .preset_name,
                "Bambu PLA Basic @BBL H2C",
                "{model}"
            );
        }
        assert_eq!(
            picked(&index(root.path(), "H2S", Some(0.4)), "GFA00"),
            Some("Bambu PLA Basic @BBL H2S".into())
        );
    }

    #[test]
    fn a_user_preset_stands_in_when_no_system_preset_fits_the_printer() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Bambu PETG @BBL H2S",
            "GFG09",
            &["Bambu Lab H2S 0.4 nozzle"],
        );
        write(
            root.path(),
            &format!("user/{USER}/filament/My PETG @Bambu Lab H2C 0.4 nozzle.json"),
            json!({
                "name":"My PETG @Bambu Lab H2C 0.4 nozzle","inherits":"","from":"User",
                "filament_id":"GFG09","filament_type":["PETG"],
                "compatible_printers":["Bambu Lab H2C 0.4 nozzle"]
            }),
        );
        let d = index(root.path(), "H2C", Some(0.4))
            .lookup("GFG09")
            .unwrap();
        assert_eq!(d.preset_name, "My PETG @Bambu Lab H2C 0.4 nozzle");
        assert!(d.is_user);
        assert_eq!(d.variants, vec!["My PETG @Bambu Lab H2C 0.4 nozzle"]);
        let d = index(root.path(), "H2S", Some(0.4))
            .lookup("GFG09")
            .unwrap();
        assert_eq!(d.preset_name, "Bambu PETG @BBL H2S");
        assert!(!d.is_user);
    }

    #[test]
    fn several_user_variants_of_one_id_pick_the_one_for_the_printer_and_nozzle() {
        let root = fake_bambu_root();
        for (name, machine) in [
            ("Acme PLA @Bambu Lab H2C 0.2 nozzle", "H2C 0.2"),
            ("Acme PLA @Bambu Lab H2C 0.4 nozzle", "H2C 0.4"),
            ("Acme PLA @Bambu Lab H2S 0.4 nozzle", "H2S 0.4"),
            ("Acme PLA @Bambu Lab H2C 0.6 nozzle", "H2C 0.6"),
        ] {
            write(
                root.path(),
                &format!("user/{USER}/filament/base/{name}.json"),
                json!({
                    "name":name,"inherits":"","from":"User","filament_id":"P1a2b3c4",
                    "filament_type":["PLA"],
                    "compatible_printers":[format!("Bambu Lab {machine} nozzle")]
                }),
            );
        }
        let h2c = |nozzle| {
            index(root.path(), "H2C", nozzle)
                .lookup("p1a2b3c4")
                .unwrap()
        };
        assert_eq!(
            h2c(Some(0.4)).preset_name,
            "Acme PLA @Bambu Lab H2C 0.4 nozzle"
        );
        assert_eq!(
            h2c(Some(0.6)).preset_name,
            "Acme PLA @Bambu Lab H2C 0.6 nozzle"
        );
        assert_eq!(h2c(None).preset_name, "Acme PLA @Bambu Lab H2C 0.4 nozzle");
        assert_eq!(
            h2c(Some(0.4)).variants,
            vec!["Acme PLA @Bambu Lab H2C 0.4 nozzle"]
        );
        assert_eq!(h2c(None).variants.len(), 3, "the H2S one is left out");
        assert_eq!(
            picked(&index(root.path(), "H2S", Some(0.4)), "P1A2B3C4"),
            Some("Acme PLA @Bambu Lab H2S 0.4 nozzle".into())
        );
    }

    #[test]
    fn a_user_preset_resolves_to_itself_and_never_shadows_a_system_preset() {
        let root = fake_bambu_root();
        write(
            root.path(),
            &format!("user/{USER}/filament/base/Acme Matte PLA @Bambu Lab H2C 0.4 nozzle.json"),
            json!({
                "name":"Acme Matte PLA @Bambu Lab H2C 0.4 nozzle","inherits":"","from":"User",
                "filament_id":"P4d6ae04","filament_type":["PLA"],
                "compatible_printers":["Bambu Lab H2C 0.4 nozzle"]
            }),
        );
        write(
            root.path(),
            &format!("user/{USER}/filament/My PLA.json"),
            json!({
                "type":"filament","name":"My PLA","inherits":"Bambu PLA Basic @BBL H2C","from":"User"
            }),
        );
        let idx = index(root.path(), "H2C", Some(0.4));
        let mine = idx.lookup("P4d6ae04").unwrap();
        assert_eq!(mine.preset_name, "Acme Matte PLA @Bambu Lab H2C 0.4 nozzle");
        assert!(mine.is_user);
        assert_eq!(
            mine.variants,
            vec!["Acme Matte PLA @Bambu Lab H2C 0.4 nozzle"]
        );
        assert_eq!(
            idx.lookup("GFA00").unwrap().preset_name,
            "Bambu PLA Basic @BBL H2C",
            "My PLA inherits GFA00, but the system preset comes first"
        );
        assert!(idx.lookup("HSPETG04").unwrap().is_user);
    }

    #[test]
    fn a_calibrated_copy_sharing_the_system_id_never_displaces_the_system_preset() {
        // Calibration's copy of a system preset keeps its filament_id: a
        // flattened user preset with the same id, printer and nozzle.
        let root = fake_bambu_root();
        write(
            root.path(),
            &format!("user/{USER}/filament/base/Bambu PLA Basic (calibrated) @BBL H2C.json"),
            json!({
                "name":"Bambu PLA Basic (calibrated) @BBL H2C","inherits":"","from":"User",
                "instantiation":"true","filament_id":"GFA00","filament_type":["PLA"],
                "compatible_printers":["Bambu Lab H2C 0.4 nozzle"]
            }),
        );
        let first = index(root.path(), "H2C", Some(0.4))
            .lookup("GFA00")
            .unwrap();
        assert_eq!(first.preset_name, "Bambu PLA Basic @BBL H2C");
        assert!(!first.is_user, "the system preset comes first");
        assert_eq!(first.variants, vec!["Bambu PLA Basic @BBL H2C"]);
        // Rebuilt (every preset change rebuilds it), the pick stays put.
        for _ in 0..5 {
            assert_eq!(
                index(root.path(), "H2C", Some(0.4)).lookup("GFA00"),
                Some(first.clone())
            );
        }
    }

    #[test]
    fn an_unknown_or_missing_id_gives_none() {
        let root = fake_bambu_root();
        let idx = index(root.path(), "H2C", Some(0.4));
        assert_eq!(idx.lookup("GFL99"), None);
        assert_eq!(idx.lookup(""), None);
        assert_eq!(idx.lookup("   "), None);
    }

    #[test]
    fn a_preset_made_only_for_another_printer_is_not_installed_for_this_one() {
        let root = fake_bambu_root();
        write(
            root.path(),
            "system/BBL/filament/Bambu PETG HF @BBL H2S.json",
            json!({
                "type":"filament","name":"Bambu PETG HF @BBL H2S","from":"system",
                "instantiation":"true","filament_id":"GFG02","filament_type":["PETG"],
                "compatible_printers":["Bambu Lab H2S 0.4 nozzle"]
            }),
        );
        assert_eq!(index(root.path(), "H2C", Some(0.4)).lookup("GFG02"), None);
        assert_eq!(
            index(root.path(), "H2S", Some(0.4))
                .lookup("GFG02")
                .unwrap()
                .preset_name,
            "Bambu PETG HF @BBL H2S"
        );
    }

    #[test]
    fn a_model_no_preset_names_filters_nothing() {
        let root = fake_bambu_root();
        for model in ["Z9", ""] {
            assert_eq!(
                index(root.path(), model, Some(0.4))
                    .lookup("GFA00")
                    .unwrap()
                    .preset_name,
                "Bambu PLA Basic @BBL H2C",
                "{model:?}"
            );
        }
    }

    #[test]
    fn machine_names_parse_into_model_and_nozzle() {
        assert_eq!(machine("Bambu Lab H2D 0.4 nozzle"), Some(("H2D", 0.4)));
        assert_eq!(
            machine("Bambu Lab X1 Carbon 0.6 nozzle"),
            Some(("X1 Carbon", 0.6))
        );
        assert_eq!(
            machine("Bambu Lab H2D Pro 0.4 nozzle"),
            Some(("H2D Pro", 0.4))
        );
        assert_eq!(machine("My printer"), None);
        assert_eq!(machine("Bambu Lab H2D nozzle"), None);
    }

    #[test]
    fn no_presets_make_an_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        assert!(index(dir.path(), "H2D", Some(0.4)).is_empty());
    }

    #[test]
    fn lookup_outcome_tells_a_wrong_nozzle_from_nothing_installed() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Bambu TPU 85A @BBL H2C",
            "GFU04",
            &["Bambu Lab H2C 0.8 nozzle", "Bambu Lab H2C 0.6 nozzle"],
        );
        sys(
            root.path(),
            "Bambu TPU 85A @BBL H2S",
            "GFU04",
            &["Bambu Lab H2S 0.4 nozzle"],
        );
        let h2c = index(root.path(), "H2C", Some(0.4));
        assert_eq!(
            h2c.lookup_outcome("GFU04"),
            Lookup::OtherNozzle {
                nozzles: vec![0.6, 0.8]
            },
            "the H2S 0.4 preset is for another printer and is not counted"
        );
        assert_eq!(h2c.lookup("GFU04"), None, "lookup is unchanged");
        assert_eq!(h2c.lookup_outcome(" gfu04 "), h2c.lookup_outcome("GFU04"));
        assert_eq!(h2c.lookup_outcome("GFL99"), Lookup::NotInstalled);
        assert_eq!(h2c.lookup_outcome(""), Lookup::NotInstalled);
        assert_eq!(h2c.lookup_outcome("  "), Lookup::NotInstalled);
        assert!(matches!(h2c.lookup_outcome("GFA00"), Lookup::Found(d)
            if d.preset_name == "Bambu PLA Basic @BBL H2C"));
        // Only for another printer: not installed, whatever the nozzle.
        let s2 = index(root.path(), "H2S", Some(0.6));
        assert_eq!(
            s2.lookup_outcome("GFU04"),
            Lookup::OtherNozzle { nozzles: vec![0.4] }
        );
        sys(
            root.path(),
            "PLA @H2D",
            "GFA01",
            &["Bambu Lab H2D 0.4 nozzle"],
        );
        let h2d = index(root.path(), "H2D", Some(0.4));
        assert_eq!(h2d.lookup_outcome("GFU04"), Lookup::NotInstalled);
    }

    #[test]
    fn with_an_unknown_nozzle_a_preset_is_found_so_no_other_nozzle_is_reported() {
        let root = fake_bambu_root();
        sys(
            root.path(),
            "Bambu TPU 85A @BBL H2C",
            "GFU04",
            &["Bambu Lab H2C 0.6 nozzle"],
        );
        let idx = index(root.path(), "H2C", None);
        assert!(matches!(idx.lookup_outcome("GFU04"), Lookup::Found(_)));
    }

    #[test]
    fn a_user_preset_for_another_nozzle_is_reported_too() {
        let root = fake_bambu_root();
        let idx = index(root.path(), "H2C", Some(0.6));
        assert_eq!(
            idx.lookup_outcome("HSPETG04"),
            Lookup::OtherNozzle { nozzles: vec![0.4] },
            "the SUNLU 0.4 preset, with a 0.6 nozzle fitted"
        );
    }

    #[test]
    fn hand_typed_lower_case_discovery_codes_find_their_presets() {
        let root = fake_bambu_root();
        for model in ["o1c2", "O1C2", "o1s"] {
            let want = if model.eq_ignore_ascii_case("O1S") {
                "Bambu PLA Basic @BBL H2S"
            } else {
                "Bambu PLA Basic @BBL H2C"
            };
            assert_eq!(
                picked(&index(root.path(), model, Some(0.4)), "GFA00").as_deref(),
                Some(want),
                "{model}"
            );
        }
        sys(
            root.path(),
            "PLA @A1M",
            "GFA01",
            &["Bambu Lab A1 mini 0.4 nozzle"],
        );
        sys(
            root.path(),
            "PLA @P1S",
            "GFA01",
            &["Bambu Lab P1S 0.4 nozzle"],
        );
        assert_eq!(
            picked(&index(root.path(), "n1", Some(0.4)), "GFA01").as_deref(),
            Some("PLA @A1M")
        );
        assert_eq!(
            picked(&index(root.path(), "c12", Some(0.4)), "GFA01").as_deref(),
            Some("PLA @P1S")
        );
    }

    #[test]
    fn h2s_and_h2c_are_different_printers() {
        let root = fake_bambu_root();
        for (model, want) in [
            ("H2S", "Bambu PLA Basic @BBL H2S"),
            ("Bambu Lab H2S", "Bambu PLA Basic @BBL H2S"),
            ("H2C", "Bambu PLA Basic @BBL H2C"),
        ] {
            assert_eq!(
                picked(&index(root.path(), model, Some(0.4)), "GFA00").as_deref(),
                Some(want),
                "{model}"
            );
        }
    }

    #[test]
    fn an_unrecognised_model_or_unreadable_names_let_the_nozzle_rank_but_not_filter() {
        let root = fake_bambu_root();
        // A model no preset names: the 0.4-only PLA still shows for a 0.6 nozzle.
        assert_eq!(
            picked(&index(root.path(), "Z9", Some(0.6)), "GFA00").as_deref(),
            Some("Bambu PLA Basic @BBL H2C")
        );
        assert_eq!(
            picked(&index(root.path(), "", Some(0.6)), "GFA00").as_deref(),
            Some("Bambu PLA Basic @BBL H2C")
        );
        // ...but the nozzle still ranks: the 0.6 variant beats the earlier name.
        sys(
            root.path(),
            "Zzz PLA @BBL H2C 0.6 nozzle",
            "GFA00",
            &["Bambu Lab H2C 0.6 nozzle"],
        );
        assert_eq!(
            picked(&index(root.path(), "Z9", Some(0.6)), "GFA00").as_deref(),
            Some("Zzz PLA @BBL H2C 0.6 nozzle")
        );
        // Names that don't parse: not ours to judge, even for a known model.
        sys(root.path(), "Custom PLA", "GFA77", &["My Printer"]);
        let idx = index(root.path(), "H2C", Some(0.6));
        assert_eq!(picked(&idx, "GFA77").as_deref(), Some("Custom PLA"));
    }
}
