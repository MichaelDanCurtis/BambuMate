//! The filament ID index: which installed preset a filament ID the printer
//! reports is. It is built from Bambu Studio's system filament presets and
//! the user's own. Each is resolved through `inherits` to its real
//! `filament_id` with the slicer's [`PresetIndex::flatten`], so a variant
//! such as `Bambu PLA Basic @BBL H2C` gets GFA00 from its `@base`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Every installed preset with this ID from the same source (one per
    /// printer or nozzle), by name.
    pub variants: Vec<String>,
}

/// The printer the index picks variants for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrinterTarget {
    /// `H2D`, `X1 Carbon`: the model without "Bambu Lab". Empty when unknown.
    pub model: String,
    /// The active nozzle's diameter in mm.
    pub nozzle: Option<f64>,
}

/// Filament ID (upper-cased) → the preset chosen for the printer.
#[derive(Debug, Default)]
pub struct FilamentIndex {
    by_id: HashMap<String, Detected>,
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
        let model_known = !target.model.is_empty()
            && groups.values().flatten().any(|c| {
                c.compatible
                    .iter()
                    .any(|p| machine(p).is_some_and(|(m, _)| m.eq_ignore_ascii_case(&target.model)))
            });
        let by_id = groups
            .into_iter()
            .filter_map(|(id, c)| choose(c, target, model_known).map(|d| (id, d)))
            .collect();
        Self { by_id }
    }

    /// The preset for a reported ID. `None` for an empty ID, or for one no
    /// installed preset for this printer has.
    pub fn lookup(&self, id: &str) -> Option<Detected> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        self.by_id.get(&id.to_ascii_uppercase()).cloned()
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

/// How well a preset fits: 2 for the configured nozzle, 1 for the
/// configured printer (or every printer), `None` for another printer.
fn fit(compatible: &[String], target: &PrinterTarget, model_known: bool) -> Option<u8> {
    if compatible.is_empty() || !model_known {
        return Some(1);
    }
    let mut best = None;
    for (model, diameter) in compatible.iter().filter_map(|p| machine(p)) {
        if !model.eq_ignore_ascii_case(&target.model) {
            continue;
        }
        let nozzle = target.nozzle.is_some_and(|n| (n - diameter).abs() < 1e-6);
        best = best.max(Some(if nozzle { 2 } else { 1 }));
    }
    best
}

/// System presets first, user presets only when no system preset fits.
/// Within a source: the best fit, then the first by name.
fn choose(
    mut candidates: Vec<Candidate>,
    target: &PrinterTarget,
    model_known: bool,
) -> Option<Detected> {
    candidates.sort_by(|a, b| a.entry.name.cmp(&b.entry.name));
    for source in [PresetSource::System, PresetSource::User] {
        let tier: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.entry.source == source)
            .collect();
        let best = tier
            .iter()
            .filter_map(|c| fit(&c.compatible, target, model_known).map(|f| (f, *c)))
            .max_by(|(fa, a), (fb, b)| fa.cmp(fb).then_with(|| b.entry.name.cmp(&a.entry.name)));
        if let Some((_, c)) = best {
            return Some(Detected {
                preset_name: c.entry.name.clone(),
                path: c.entry.path.to_string_lossy().into_owned(),
                is_user: source == PresetSource::User,
                variants: tier.iter().map(|c| c.entry.name.clone()).collect(),
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
            vec!["Bambu PLA Basic @BBL H2C", "Bambu PLA Basic @BBL H2S"]
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
        assert_eq!(
            name("H2C", Some(0.6)),
            "Bambu PLA Basic @BBL H2C",
            "no variant for the nozzle: the first by name for the printer"
        );
        assert_eq!(name("H2C", None), "Bambu PLA Basic @BBL H2C");
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
}
