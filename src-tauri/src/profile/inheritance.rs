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

/// The filament keys Bambu Studio keeps one element per nozzle variant
/// (`filament_options_with_variant`, `src/libslic3r/PrintConfig.cpp`).
/// A user preset's array for one of these is merged into its parent's
/// element by element, matched by variant name, and a "nil" element keeps
/// the parent's. The keys in [`FILAMENT_OVERRIDE_KEYS`] are the exception.
pub(crate) const FILAMENT_VARIANT_KEYS: &[&str] = &[
    "filament_flow_ratio",
    "filament_max_volumetric_speed",
    "filament_ramming_volumetric_speed",
    "filament_pre_cooling_temperature",
    "filament_ramming_travel_time",
    "filament_ramming_volumetric_speed_nc",
    "filament_pre_cooling_temperature_nc",
    "filament_ramming_travel_time_nc",
    "filament_extruder_id",
    "filament_extruder_variant",
    "filament_retraction_length",
    "filament_retract_length_nc",
    "filament_z_hop",
    "filament_z_hop_types",
    "filament_retract_restart_extra",
    "filament_retraction_speed",
    "filament_deretraction_speed",
    "filament_retraction_minimum_travel",
    "filament_retract_when_changing_layer",
    "filament_wipe",
    "filament_wipe_distance",
    "filament_retract_before_wipe",
    "filament_long_retractions_when_cut",
    "filament_retraction_distances_when_cut",
    "long_retractions_when_ec",
    "retraction_distances_when_ec",
    "nozzle_temperature_initial_layer",
    "nozzle_temperature",
    "filament_flush_volumetric_speed",
    "filament_flush_temp",
    "filament_flush_temp_fast",
    "filament_enable_overhang_speed",
    "filament_bridge_speed",
    "filament_overhang_1_4_speed",
    "filament_overhang_2_4_speed",
    "filament_overhang_3_4_speed",
    "filament_overhang_4_4_speed",
    "filament_overhang_totally_speed",
    "override_process_overhang_speed",
    "volumetric_speed_coefficients",
    "filament_adaptive_volumetric_speed",
    "filament_preheat_temperature_delta",
    "filament_cooling_before_tower",
    "slow_down_min_speed",
];

/// The filament keys that override the printer's extruder settings
/// (`is_filament_extruder_override_key`, `src/libslic3r/PrintConfig.cpp`).
/// A preset that sets one replaces its parent's array whole, and a "nil"
/// element means "the printer's value".
pub(crate) const FILAMENT_OVERRIDE_KEYS: &[&str] = &[
    "filament_retraction_length",
    "filament_z_hop",
    "filament_z_hop_types",
    "filament_retract_lift_above",
    "filament_retract_lift_below",
    "filament_retraction_speed",
    "filament_deretraction_speed",
    "filament_retract_restart_extra",
    "filament_retraction_minimum_travel",
    "filament_wipe_distance",
    "filament_retract_when_changing_layer",
    "filament_wipe",
    "filament_retract_before_wipe",
    "filament_long_retractions_when_cut",
    "filament_retraction_distances_when_cut",
    "filament_retract_length_nc",
];

/// The key naming a filament preset's nozzle variants.
const FILAMENT_VARIANT_NAMES: &str = "filament_extruder_variant";

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
///
/// Bambu Studio merges a level into its parent two ways, and so does
/// [`resolve_with_includes`]:
/// - A system preset, and every `include` template, replaces each key it
///   sets whole, "nil" elements included (`PresetBundle::
///   load_vendor_configs_from_json`: `apply_only` for includes, then
///   `apply`).
/// - A user preset (`"from": "User"`) over a parent is merged as
///   `PresetCollection::load_presets` merges it, through
///   `DynamicPrintConfig::update_diff_values_to_child_config`:
///   - a [`FILAMENT_VARIANT_KEYS`] array is merged element by element,
///     matched by variant name: a "nil" element, or a variant the preset
///     doesn't list, keeps the parent's element, and the parent's variant
///     list stays;
///   - a [`FILAMENT_OVERRIDE_KEYS`] array, or a scalar, replaces the
///     parent's whole ("nil" is the printer's value);
///   - any other key keeps BambuMate's earlier rule: an all-"nil" array
///     leaves the parent's real value, anything else replaces it. Process
///     and machine presets have variant sets of their own, which this
///     doesn't model per element.
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
        let user_over_parent = i > 0
            && level
                .raw()
                .get("from")
                .and_then(Value::as_str)
                .is_some_and(|f| f.eq_ignore_ascii_case("user"));
        if user_over_parent {
            merge_user_level(&mut config, level.raw(), i != leaf_index);
        } else {
            merge_level(&mut config, level.raw(), i != leaf_index);
        }
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

/// A system level or an include: each key replaces the one before it whole.
fn merge_level(config: &mut Map<String, Value>, src: &Map<String, Value>, skip_metadata: bool) {
    for (key, value) in src {
        if key == "include" || (skip_metadata && SKIP_INHERIT_FIELDS.contains(&key.as_str())) {
            continue;
        }
        config.insert(key.clone(), value.clone());
    }
}

/// A user preset over its parent: see [`ResolvedPreset`].
fn merge_user_level(
    config: &mut Map<String, Value>,
    src: &Map<String, Value>,
    skip_metadata: bool,
) {
    let parent_variants = text_array(config.get(FILAMENT_VARIANT_NAMES));
    // Studio gives a preset without its own variant list its parent's.
    let own_variants = match src.get(FILAMENT_VARIANT_NAMES) {
        Some(v) => text_array(Some(v)),
        None => parent_variants.clone(),
    };
    for (key, value) in src {
        if key == "include" || (skip_metadata && SKIP_INHERIT_FIELDS.contains(&key.as_str())) {
            continue;
        }
        let k = key.as_str();
        if k == FILAMENT_VARIANT_NAMES {
            // The parent's variants stay; a preset without a parent list
            // brings its own.
            if parent_variants.is_none() {
                config.insert(key.clone(), value.clone());
            }
            continue;
        }
        let per_variant =
            FILAMENT_VARIANT_KEYS.contains(&k) && !FILAMENT_OVERRIDE_KEYS.contains(&k);
        match (per_variant, value, config.get(key)) {
            (true, Value::Array(own), Some(Value::Array(parent))) => {
                let merged = merge_by_variant(
                    key,
                    parent,
                    own,
                    parent_variants.as_deref().unwrap_or(&[]),
                    own_variants.as_deref().unwrap_or(&[]),
                );
                config.insert(key.clone(), Value::Array(merged));
            }
            (true, _, _) => {
                config.insert(key.clone(), value.clone());
            }
            (false, _, _) => {
                let overrides = FILAMENT_OVERRIDE_KEYS.contains(&k);
                if !overrides
                    && is_nil_value(value)
                    && config.get(key).is_some_and(|v| !is_nil_value(v))
                {
                    continue;
                }
                config.insert(key.clone(), value.clone());
            }
        }
    }
}

/// `own` merged into `parent` as Bambu Studio's `set_only_diff` merges a
/// variant array: each parent element takes the element of the same
/// variant name in `own`, unless that one is "nil" or missing. `own` is
/// first fitted to its variant list (cut, or widened with its first
/// element, as `extend_default_config_length` does). A parent array that
/// doesn't match its own variant list is kept as it is, as Studio keeps
/// it after refusing the merge.
fn merge_by_variant(
    key: &str,
    parent: &[Value],
    own: &[Value],
    parent_variants: &[String],
    own_variants: &[String],
) -> Vec<Value> {
    let Some(first) = own.first() else {
        return parent.to_vec();
    };
    let mut own = own.to_vec();
    let own_len = own_variants.len().max(1);
    own.resize(own_len, first.clone());
    // Parent element i takes own element map[i].
    let map: Vec<Option<usize>> = if parent_variants.is_empty() || own_variants.is_empty() {
        let mut m = vec![None; parent_variants.len().max(1)];
        m[0] = Some(0);
        m
    } else {
        parent_variants
            .iter()
            .map(|p| own_variants.iter().position(|o| o == p))
            .collect()
    };
    if parent.len() != map.len() {
        tracing::warn!(
            "{key}: the parent has {} elements for {} variants; its value is kept",
            parent.len(),
            map.len()
        );
        return parent.to_vec();
    }
    let mut merged = parent.to_vec();
    for (slot, j) in merged.iter_mut().zip(map) {
        if let Some(v) = j.and_then(|j| own.get(j)) {
            if v.as_str() != Some("nil") {
                *slot = v.clone();
            }
        }
    }
    merged
}

/// An array of strings, as text; `None` when `value` isn't one.
fn text_array(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::Array(items) => Some(
            items
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect(),
        ),
        _ => None,
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

    /// Bambu Studio's vendor loader (`PresetBundle::load_vendor_configs_from_json`)
    /// copies the parent's config, applies each include's keys
    /// (`apply_only`), then the preset's own (`apply`): every key a system
    /// level or an include sets replaces the one before it whole, "nil"
    /// elements included. The real H2D templates set the retraction
    /// overrides to `["nil","nil","nil"]` this way, which means "the
    /// printer's value" for every variant.
    #[test]
    fn system_levels_and_includes_replace_whole_keys_nil_included() {
        let registry = registry_of(vec![
            mk(json!({"name":"base","from":"system","a":["5"],"b":["nil"],"c":["1","2"]})),
            mk(json!({"name":"tmpl","a":["nil","nil","nil"],"b":["nil","nil","nil"]})),
        ]);
        let leaf = mk(json!({
            "name":"leaf","inherits":"base","from":"system","include":["tmpl"],"c":["nil","3"]
        }));
        let r = resolve_with_includes(&leaf, &registry).unwrap();
        assert_eq!(r.config["a"], json!(["nil", "nil", "nil"]));
        assert_eq!(r.config["b"], json!(["nil", "nil", "nil"]));
        assert_eq!(r.config["c"], json!(["nil", "3"]));
    }

    /// A parent shaped like `Bambu PLA Basic @BBL H2D`, for the user-preset
    /// merge (`PresetCollection::load_presets` then
    /// `DynamicPrintConfig::update_diff_values_to_child_config`).
    fn h2d_parent() -> ProfileRegistry {
        registry_of(vec![mk(json!({
            "name":"parent","from":"system","instantiation":"true",
            "filament_extruder_variant":["Standard","High Flow","E3D"],
            "filament_flow_ratio":["0.98","0.97","0.96"],
            "nozzle_temperature":["220","225","230"],
            "filament_retraction_length":["0.4","0.4","1"],
            "layer_gap":["0.2","0.2"]
        }))])
    }

    fn user_leaf(own: serde_json::Value) -> FilamentProfile {
        let mut map = own.as_object().unwrap().clone();
        map.insert("name".into(), json!("mine"));
        map.insert("inherits".into(), json!("parent"));
        map.insert("from".into(), json!("User"));
        FilamentProfile::from_map(map)
    }

    #[test]
    fn a_user_presets_nil_variant_element_keeps_the_parents_element() {
        let leaf = user_leaf(json!({"filament_flow_ratio":["1.029","nil","nil"]}));
        let r = resolve_with_includes(&leaf, &h2d_parent()).unwrap();
        assert_eq!(
            r.config["filament_flow_ratio"],
            json!(["1.029", "0.97", "0.96"])
        );
        let all_nil = user_leaf(json!({"nozzle_temperature":["nil","nil","nil"]}));
        let r = resolve_with_includes(&all_nil, &h2d_parent()).unwrap();
        assert_eq!(r.config["nozzle_temperature"], json!(["220", "225", "230"]));
    }

    #[test]
    fn a_user_presets_variant_elements_are_matched_by_name() {
        let leaf = user_leaf(json!({
            "filament_extruder_variant":["High Flow","Standard"],
            "filament_flow_ratio":["0.9","nil"]
        }));
        let r = resolve_with_includes(&leaf, &h2d_parent()).unwrap();
        assert_eq!(
            r.config["filament_flow_ratio"],
            json!(["0.98", "0.9", "0.96"]),
            "Standard is nil, High Flow is 0.9, E3D isn't in the preset"
        );
        assert_eq!(
            r.config["filament_extruder_variant"],
            json!(["Standard", "High Flow", "E3D"]),
            "the parent's variants stay"
        );
    }

    #[test]
    fn a_user_presets_short_variant_array_is_widened_from_its_first_element() {
        let leaf = user_leaf(json!({"nozzle_temperature":["240"]}));
        let r = resolve_with_includes(&leaf, &h2d_parent()).unwrap();
        assert_eq!(r.config["nozzle_temperature"], json!(["240", "240", "240"]));
    }

    #[test]
    fn a_user_presets_override_key_replaces_the_parents_whole_nil_meaning_the_printer() {
        let leaf = user_leaf(json!({"filament_retraction_length":["nil","nil","nil"]}));
        let r = resolve_with_includes(&leaf, &h2d_parent()).unwrap();
        assert_eq!(
            r.config["filament_retraction_length"],
            json!(["nil", "nil", "nil"])
        );
        let leaf = user_leaf(json!({"filament_retraction_length":["0.6","nil","nil"]}));
        let r = resolve_with_includes(&leaf, &h2d_parent()).unwrap();
        assert_eq!(
            r.config["filament_retraction_length"],
            json!(["0.6", "nil", "nil"])
        );
    }

    #[test]
    fn a_parent_whose_array_doesnt_match_its_variants_keeps_its_value() {
        let registry = registry_of(vec![mk(json!({
            "name":"parent","from":"system",
            "filament_extruder_variant":["Standard","High Flow"],
            "filament_flow_ratio":["0.98"]
        }))]);
        let leaf = user_leaf(json!({"filament_flow_ratio":["1.01","1.02"]}));
        let r = resolve_with_includes(&leaf, &registry).unwrap();
        assert_eq!(r.config["filament_flow_ratio"], json!(["0.98"]));
    }

    /// Keys outside Bambu Studio's filament variant set keep BambuMate's
    /// earlier rule at a user level: an all-"nil" array leaves the parent's
    /// value. Process and machine presets have variant sets of their own
    /// that BambuMate doesn't model per element yet, and Studio writes
    /// "nil" there for every element equal to the parent's.
    #[test]
    fn other_keys_at_a_user_level_keep_the_parents_value_for_all_nil() {
        let leaf = user_leaf(json!({"layer_gap":["nil","nil"]}));
        let r = resolve_with_includes(&leaf, &h2d_parent()).unwrap();
        assert_eq!(r.config["layer_gap"], json!(["0.2", "0.2"]));
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
