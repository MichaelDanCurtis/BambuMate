use std::collections::HashSet;

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use tracing::debug;

use super::registry::ProfileRegistry;
use super::types::FilamentProfile;

/// Metadata fields that should NOT be inherited from parent profiles.
///
/// During inheritance merge, these fields are skipped from ancestor profiles.
/// The leaf profile's own values for these fields are applied last.
const SKIP_INHERIT_FIELDS: &[&str] = &[
    "inherits",
    "name",
    "type",
    "from",
    "instantiation",
    "filament_id",
    "setting_id",
    "include",
    "description",
    "compatible_printers",
    "compatible_prints",
    "compatible_printers_condition",
    "compatible_prints_condition",
    "filament_settings_id",
];

/// Maximum inheritance depth to prevent infinite loops.
pub(crate) const MAX_INHERITANCE_DEPTH: usize = 10;

/// Resolve the inheritance chain for a profile.
///
/// Walks the `inherits` chain from leaf to root, then merges fields
/// from base (root) to leaf. Metadata fields are skipped during
/// ancestor merge; the leaf profile's own values override everything.
///
/// The string `"nil"` (and arrays of all `"nil"` strings) are treated
/// as "inherit from parent" and do not overwrite parent values.
pub fn resolve_inheritance(
    profile: &FilamentProfile,
    registry: &ProfileRegistry,
) -> Result<FilamentProfile> {
    // Build inheritance chain: leaf -> ... -> root
    let mut chain: Vec<&FilamentProfile> = vec![profile];
    let mut visited: HashSet<String> = HashSet::new();

    if let Some(name) = profile.name() {
        visited.insert(name.to_string());
    }

    // Check for include field (not resolved, just logged)
    if let Some(include) = profile.raw().get("include") {
        debug!(
            "Profile {:?} has include field: {:?} (not resolved in this version)",
            profile.name().unwrap_or("<unnamed>"),
            include
        );
    }

    let mut current = profile;
    while let Some(parent_name) = current.inherits() {
        if parent_name.is_empty() {
            break;
        }

        // Guard against circular inheritance
        if visited.contains(parent_name) {
            bail!(
                "Circular inheritance detected: {:?} already visited in chain",
                parent_name
            );
        }

        // Guard against excessive depth
        if chain.len() >= MAX_INHERITANCE_DEPTH {
            bail!(
                "Inheritance chain exceeds maximum depth of {} for profile {:?}",
                MAX_INHERITANCE_DEPTH,
                profile.name().unwrap_or("<unnamed>")
            );
        }

        let parent = registry.get_by_name(parent_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Parent profile not found: {:?} (referenced by {:?})",
                parent_name,
                current.name().unwrap_or("<unnamed>")
            )
        })?;

        visited.insert(parent_name.to_string());

        // Log include field on parent too
        if let Some(include) = parent.raw().get("include") {
            debug!(
                "Parent profile {:?} has include field: {:?} (not resolved)",
                parent_name, include
            );
        }

        chain.push(parent);
        current = parent;
    }

    // Reverse: base first, leaf last
    chain.reverse();

    // Merge from base to leaf
    let mut resolved = Map::new();

    // Apply ancestor fields (skipping metadata fields)
    for ancestor in &chain[..chain.len().saturating_sub(1)] {
        for (key, value) in ancestor.raw() {
            // Skip metadata fields during ancestor merge
            if SKIP_INHERIT_FIELDS.contains(&key.as_str()) {
                continue;
            }

            // Skip nil values -- they mean "inherit from parent"
            if is_nil_value(value) {
                continue;
            }

            resolved.insert(key.clone(), value.clone());
        }
    }

    // Apply ALL fields from the leaf profile (including metadata)
    // The leaf's identity overrides everything
    for (key, value) in profile.raw() {
        // Even for the leaf, skip nil values so parent values remain
        if is_nil_value(value) {
            continue;
        }
        resolved.insert(key.clone(), value.clone());
    }

    // Third pass: preserve nil-valued fields from the inheritance chain.
    //
    // A field set to `["nil", "nil"]` at every level means "use Bambu Studio's
    // built-in engine default". These fields must still be present in the output
    // JSON — omitting them entirely causes import failures in Bambu Studio because
    // the schema validator expects them to exist.
    //
    // Iterating from root to leaf means we insert the root's nil value first; if a
    // higher-priority level also has nil it is the same value so there is no harm in
    // overwriting. We only ever insert when the key is NOT already resolved to a real
    // value (the `!resolved.contains_key` guard).
    //
    // Forward-compatibility: when Bambu Studio ships a new version and adds a new
    // field to `fdm_filament_common` with an initial nil placeholder, this pass will
    // automatically include it — no code changes required.
    for ancestor in chain.iter() {
        for (key, value) in ancestor.raw() {
            if SKIP_INHERIT_FIELDS.contains(&key.as_str()) {
                continue;
            }
            if is_nil_value(value) && !resolved.contains_key(key) {
                resolved.insert(key.clone(), value.clone());
            }
        }
    }

    Ok(FilamentProfile::from_map(resolved))
}

/// Why a preset couldn't be resolved with [`resolve_with_includes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// A parent named by `inherits` isn't in the registry.
    MissingParent(String),
    /// The `inherits` chain loops back on itself at this name.
    Circular(String),
    /// The chain is deeper than `MAX_INHERITANCE_DEPTH`.
    TooDeep(String),
}

/// A preset flattened the way Bambu Studio flattens it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPreset {
    /// Every key, merged root to leaf. The leaf's own metadata (`name`,
    /// `inherits`, `compatible_printers`, ...) is kept as written.
    pub config: Map<String, Value>,
    /// Ancestor names, nearest parent first.
    pub ancestors: Vec<String>,
}

/// Flattens a machine, process or filament preset exactly as Bambu Studio's
/// `PresetBundle` does before handing it to the slicer: for every level from
/// the root to the leaf, that level's `include` templates are applied first,
/// then the level's own keys. Ancestor metadata (see `SKIP_INHERIT_FIELDS`)
/// is not inherited, except that a leaf without `filament_id` takes its
/// nearest ancestor's, as Bambu Studio does.
///
/// A `"nil"` value never replaces a real one; it is kept only where nothing
/// else set the key, so Bambu Studio still sees the key.
///
/// Unlike [`resolve_inheritance`] this resolves `include`, which system
/// machine presets use for their G-code and filament presets for per-nozzle
/// templates. An `include` that isn't in the registry is skipped with a
/// warning, as Bambu Studio does.
pub fn resolve_with_includes(
    profile: &FilamentProfile,
    registry: &ProfileRegistry,
) -> std::result::Result<ResolvedPreset, ResolveError> {
    let leaf_name = profile.name().unwrap_or("<unnamed>").to_string();
    let mut chain: Vec<&FilamentProfile> = vec![profile];
    let mut visited: HashSet<String> = HashSet::from([leaf_name.clone()]);
    let mut current = profile;
    while let Some(parent_name) = current.inherits().filter(|p| !p.is_empty()) {
        if !visited.insert(parent_name.to_string()) {
            return Err(ResolveError::Circular(parent_name.to_string()));
        }
        if chain.len() >= MAX_INHERITANCE_DEPTH {
            return Err(ResolveError::TooDeep(leaf_name));
        }
        let parent = registry
            .get_by_name(parent_name)
            .ok_or_else(|| ResolveError::MissingParent(parent_name.to_string()))?;
        chain.push(parent);
        current = parent;
    }

    let mut config = Map::new();
    let leaf_index = chain.len() - 1;
    for (i, level) in chain.iter().rev().enumerate() {
        for include in include_names(level.raw().get("include")) {
            match registry.get_by_name(&include) {
                Some(template) => merge_level(&mut config, template.raw(), true),
                None => tracing::warn!(
                    "include {include:?} of preset {:?} not found; skipped",
                    level.name().unwrap_or("<unnamed>")
                ),
            }
        }
        merge_level(&mut config, level.raw(), i != leaf_index);
    }
    if !config.contains_key("filament_id") {
        if let Some(id) = chain[1..].iter().find_map(|p| p.raw().get("filament_id")) {
            config.insert("filament_id".to_string(), id.clone());
        }
    }
    config.remove("include");

    Ok(ResolvedPreset {
        config,
        ancestors: chain[1..]
            .iter()
            .filter_map(|p| p.name().map(str::to_string))
            .collect(),
    })
}

fn include_names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn merge_level(config: &mut Map<String, Value>, src: &Map<String, Value>, skip_metadata: bool) {
    for (key, value) in src {
        if key == "include" || (skip_metadata && SKIP_INHERIT_FIELDS.contains(&key.as_str())) {
            continue;
        }
        if is_nil_value(value) && config.get(key).is_some_and(|v| !is_nil_value(v)) {
            continue;
        }
        config.insert(key.clone(), value.clone());
    }
}

/// Check if a value represents "nil" (inherit from parent).
///
/// Returns true if:
/// - The value is the string `"nil"`
/// - The value is an array where ALL elements are the string `"nil"`
pub fn is_nil_value(value: &Value) -> bool {
    match value {
        Value::String(s) => s == "nil",
        Value::Array(arr) => {
            if arr.is_empty() {
                return false;
            }
            arr.iter().all(|v| v.as_str() == Some("nil"))
        }
        _ => false,
    }
}

/// Check if a profile is fully flattened (no inheritance to resolve).
///
/// Returns true if the `inherits` field is empty or missing.
/// User profiles exported by Bambu Studio are typically fully flattened.
pub fn is_fully_flattened(profile: &FilamentProfile) -> bool {
    match profile.inherits() {
        None => true,
        Some(s) => s.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};

    fn make_profile(
        name: &str,
        inherits: Option<&str>,
        extras: &[(&str, serde_json::Value)],
    ) -> FilamentProfile {
        let mut map = Map::new();
        map.insert("name".into(), json!(name));
        if let Some(p) = inherits {
            map.insert("inherits".into(), json!(p));
        }
        for (k, v) in extras {
            map.insert(k.to_string(), v.clone());
        }
        FilamentProfile::from_map(map)
    }

    fn registry_of(profiles: Vec<FilamentProfile>) -> ProfileRegistry {
        let mut r = ProfileRegistry::new();
        for p in profiles {
            r.insert(p);
        }
        r
    }

    // -- resolve_with_includes --

    fn mk(json: serde_json::Value) -> FilamentProfile {
        FilamentProfile::from_map(json.as_object().unwrap().clone())
    }

    /// Mirrors `Bambu Lab H2D 0.4 nozzle`: its G-code lives only in `include`
    /// templates, and its parent carries a different (generic) G-code.
    #[test]
    fn includes_apply_after_the_parent_and_before_own_keys() {
        let registry = registry_of(vec![
            mk(
                json!({"name":"common","machine_start_gcode":"G28 ; generic","retraction_length":["0.8"]}),
            ),
            mk(
                json!({"name":"tmpl start","instantiation":"false","machine_start_gcode":";===== machine: H2D"}),
            ),
            mk(
                json!({"name":"tmpl flow","filament_flow_ratio":["0.95"],"filament_cooling_before_tower":["0"]}),
            ),
        ]);
        let leaf = mk(json!({
            "name":"Bambu Lab H2D 0.4 nozzle","inherits":"common","from":"system",
            "include":["tmpl start","tmpl flow"],
            "filament_cooling_before_tower":["10"]
        }));
        let r = resolve_with_includes(&leaf, &registry).unwrap();
        assert_eq!(
            r.config["machine_start_gcode"],
            json!(";===== machine: H2D")
        );
        assert_eq!(r.config["filament_flow_ratio"], json!(["0.95"]));
        assert_eq!(
            r.config["filament_cooling_before_tower"],
            json!(["10"]),
            "own keys win over includes"
        );
        assert_eq!(r.config["retraction_length"], json!(["0.8"]));
        assert_eq!(r.config["name"], json!("Bambu Lab H2D 0.4 nozzle"));
        assert!(!r.config.contains_key("include"));
        assert_eq!(r.ancestors, vec!["common".to_string()]);
    }

    #[test]
    fn ancestor_metadata_is_not_inherited_but_filament_id_is() {
        let registry = registry_of(vec![mk(json!({
            "name":"Bambu PLA Basic @base","filament_id":"GFA00","setting_id":"GFSA00",
            "compatible_printers":["x"],"filament_density":["1.26"]
        }))]);
        let leaf = mk(
            json!({"name":"Bambu PLA Basic @BBL H2C","inherits":"Bambu PLA Basic @base","compatible_printers":["Bambu Lab H2C 0.4 nozzle"]}),
        );
        let r = resolve_with_includes(&leaf, &registry).unwrap();
        assert_eq!(r.config["filament_id"], json!("GFA00"));
        assert!(!r.config.contains_key("setting_id"));
        assert_eq!(
            r.config["compatible_printers"],
            json!(["Bambu Lab H2C 0.4 nozzle"])
        );
        assert_eq!(r.config["filament_density"], json!(["1.26"]));
    }

    #[test]
    fn nil_never_replaces_a_real_value_but_replaces_nil() {
        let registry = registry_of(vec![
            mk(json!({"name":"base","a":["5"],"b":["nil"]})),
            mk(json!({"name":"tmpl","a":["nil","nil","nil"],"b":["nil","nil","nil"]})),
        ]);
        let leaf = mk(json!({"name":"leaf","inherits":"base","include":["tmpl"]}));
        let r = resolve_with_includes(&leaf, &registry).unwrap();
        assert_eq!(r.config["a"], json!(["5"]));
        assert_eq!(r.config["b"], json!(["nil", "nil", "nil"]));
    }

    #[test]
    fn missing_parent_and_loops_are_reported_by_name() {
        let leaf = mk(json!({"name":"leaf","inherits":"gone"}));
        assert_eq!(
            resolve_with_includes(&leaf, &ProfileRegistry::new()),
            Err(ResolveError::MissingParent("gone".into()))
        );
        let registry = registry_of(vec![
            mk(json!({"name":"a","inherits":"b"})),
            mk(json!({"name":"b","inherits":"a"})),
        ]);
        let start = mk(json!({"name":"start","inherits":"a"}));
        assert_eq!(
            resolve_with_includes(&start, &registry),
            Err(ResolveError::Circular("a".into()))
        );
    }

    #[test]
    fn missing_include_is_skipped() {
        let leaf = mk(json!({"name":"leaf","include":["nowhere"],"k":"v"}));
        let r = resolve_with_includes(&leaf, &ProfileRegistry::new()).unwrap();
        assert_eq!(r.config["k"], json!("v"));
    }

    /// A chain one level longer than `MAX_INHERITANCE_DEPTH` allows is
    /// refused, naming the preset being resolved; one level shorter resolves.
    #[test]
    fn chains_deeper_than_the_limit_are_too_deep() {
        let chain = |ancestors: usize| {
            let mut profiles = Vec::new();
            for i in 0..ancestors {
                let mut p = json!({"name": format!("p{i}")});
                if i + 1 < ancestors {
                    p["inherits"] = json!(format!("p{}", i + 1));
                }
                profiles.push(mk(p));
            }
            registry_of(profiles)
        };
        let leaf = mk(json!({"name":"leaf","inherits":"p0"}));
        assert_eq!(
            resolve_with_includes(&leaf, &chain(MAX_INHERITANCE_DEPTH)),
            Err(ResolveError::TooDeep("leaf".into()))
        );
        let r = resolve_with_includes(&leaf, &chain(MAX_INHERITANCE_DEPTH - 1)).unwrap();
        assert_eq!(r.ancestors.len(), MAX_INHERITANCE_DEPTH - 1);
    }

    // -- nil preservation --

    /// Fields that are nil at every level in the chain must still appear in the
    /// resolved output so Bambu Studio doesn't reject the profile.
    #[test]
    fn nil_field_preserved_when_all_levels_nil() {
        let base = make_profile(
            "base",
            None,
            &[
                ("real_field", json!(["200", "200"])),
                ("nil_field", json!(["nil", "nil"])),
            ],
        );
        let leaf = make_profile("leaf", Some("base"), &[]);

        let registry = registry_of(vec![base]);
        let resolved = resolve_inheritance(&leaf, &registry).unwrap();

        assert!(
            resolved.raw().contains_key("nil_field"),
            "nil-only field must be present in resolved output"
        );
        assert_eq!(resolved.raw()["nil_field"], json!(["nil", "nil"]));
        assert_eq!(resolved.raw()["real_field"], json!(["200", "200"]));
    }

    /// A leaf with nil defers to the ancestor's real value (nil means
    /// "use parent"). The ancestor value must not be replaced by nil.
    #[test]
    fn nil_in_leaf_does_not_overwrite_ancestor_real_value() {
        let base = make_profile("base", None, &[("temp", json!(["220", "220"]))]);
        let leaf = make_profile("leaf", Some("base"), &[("temp", json!(["nil", "nil"]))]);

        let registry = registry_of(vec![base]);
        let resolved = resolve_inheritance(&leaf, &registry).unwrap();

        assert_eq!(
            resolved.raw()["temp"],
            json!(["220", "220"]),
            "ancestor real value must survive leaf nil"
        );
    }

    /// A leaf with a real value must override the ancestor's value.
    #[test]
    fn leaf_real_value_overrides_ancestor() {
        let base = make_profile("base", None, &[("temp", json!(["200", "200"]))]);
        let leaf = make_profile("leaf", Some("base"), &[("temp", json!(["240", "240"]))]);

        let registry = registry_of(vec![base]);
        let resolved = resolve_inheritance(&leaf, &registry).unwrap();

        assert_eq!(resolved.raw()["temp"], json!(["240", "240"]));
    }

    /// Fields that only exist on the leaf (not the base) and carry nil must
    /// also appear in the output.
    #[test]
    fn nil_field_only_on_leaf_is_preserved() {
        let base = make_profile("base", None, &[]);
        let leaf = make_profile(
            "leaf",
            Some("base"),
            &[("leaf_only_nil", json!(["nil", "nil"]))],
        );

        let registry = registry_of(vec![base]);
        let resolved = resolve_inheritance(&leaf, &registry).unwrap();

        assert!(
            resolved.raw().contains_key("leaf_only_nil"),
            "nil field present only on the leaf must be preserved"
        );
    }
}
