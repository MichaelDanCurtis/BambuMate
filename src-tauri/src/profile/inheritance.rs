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
const MAX_INHERITANCE_DEPTH: usize = 10;

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

    Ok(FilamentProfile::from_map(resolved))
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
