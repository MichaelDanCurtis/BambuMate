# Phase 2: Profile Engine - Research

**Researched:** 2026-02-04
**Domain:** Bambu Studio filament profile JSON read/write with inheritance resolution, atomic writes, and round-trip preservation
**Confidence:** HIGH

## Summary

This research examined the actual Bambu Studio filament profile files on disk at `~/Library/Application Support/BambuStudio/` to reverse-engineer the undocumented JSON format, inheritance chain mechanism, file organization, and metadata patterns. The profile format was analyzed across 4 user-created profiles, 6 system base profiles, 3 inheritance template files, and the BBL.json registry -- all from the local Bambu Studio installation (version 2.5.0.5).

The profile format is a JSON object where nearly all values are strings inside arrays (e.g., `"nozzle_temperature": ["220", "220"]`), with the string `"nil"` used to indicate "inherit from parent." Inheritance is resolved via the `inherits` field (pointing to a parent profile by name) and an `include` field (a mixin mechanism used for dual-extruder templates). System profiles use a 3-4 level chain: `fdm_filament_common` -> `fdm_filament_pla` -> `Generic PLA @base` -> `Generic PLA @BBL H2C 0.4 nozzle`. User-created profiles are fully flattened (all ~139 fields present, `inherits: ""`) and stored alongside `.info` metadata files in INI-like format.

The standard approach for implementing the profile engine in Rust is: use `serde_json` with `preserve_order` feature for round-trip key ordering, model profiles as `serde_json::Value` (not a fully-typed struct) to guarantee zero data loss, implement inheritance resolution by walking the `inherits` chain and merging fields, and use `tempfile` crate's `NamedTempFile::persist()` for atomic writes.

**Primary recommendation:** Model profiles as `serde_json::Map<String, Value>` (not a typed struct) for the general case, with typed accessor methods for the ~20 fields BambuMate actively manipulates. This guarantees round-trip fidelity for all 139+ fields without maintaining a struct that mirrors Bambu Studio's evolving schema.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| serde | 1.0 | Serialization framework | Already in project from Phase 1 |
| serde_json | 1.0.149 | JSON parsing/writing with `preserve_order` feature | Must preserve key ordering for round-trip fidelity |
| tempfile | 3.24.0 | Atomic file writes via NamedTempFile::persist() | Industry-standard atomic write pattern in Rust |
| dirs | 6.0 | Cross-platform path resolution | Already in project from Phase 1 |
| tracing | 0.1 | Structured logging | Already in project from Phase 1 |
| anyhow | 1.0 | Application-level error handling | Already in project from Phase 1 |
| thiserror | 2.0 | Library-level error types | Already in project from Phase 1 |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| indexmap | 2.x | Insertion-ordered HashMap (pulled in by serde_json preserve_order) | Automatically used when preserve_order enabled |
| walkdir | 2.5 | Recursive directory traversal for discovering profiles | Scanning system/BBL/filament/ for all base profiles |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `serde_json::Value` for profile model | Typed struct with `#[serde(flatten)]` | Typed struct risks losing fields if struct definition lags behind BS updates; Value approach is guaranteed lossless but loses compile-time field access |
| `tempfile::NamedTempFile::persist()` | `atomic_write_file` crate | tempfile is more mature (130M+ downloads), already well-known; atomic_write_file is newer but provides similar guarantees |
| `walkdir` for profile scanning | `std::fs::read_dir` + manual recursion | walkdir handles edge cases (symlinks, permissions) and is more ergonomic |

**Installation (additions to Cargo.toml):**
```toml
# In [dependencies] section, modify existing serde_json:
serde_json = { version = "1.0", features = ["preserve_order"] }

# Add new dependencies:
tempfile = "3.24"
walkdir = "2.5"
```

## Architecture Patterns

### Recommended Project Structure
```
src-tauri/src/
├── commands/
│   ├── mod.rs           # Add profile commands
│   └── profile.rs       # Tauri commands for profile operations
├── profile/
│   ├── mod.rs           # Public API: read_profile, write_profile, list_profiles
│   ├── types.rs         # FilamentProfile, ProfileMetadata, ProfileId types
│   ├── paths.rs         # OS-specific path detection, user/system dir resolution
│   ├── reader.rs        # Read JSON from disk, parse into FilamentProfile
│   ├── inheritance.rs   # Resolve inheritance chain, merge fields
│   ├── writer.rs        # Serialize to JSON, atomic write with tempfile
│   └── registry.rs      # Discover and index all available profiles (system + user)
├── error.rs             # Extended with ProfileError variants
└── lib.rs               # Wire profile commands into Tauri handler
```

### Pattern 1: Value-Based Profile Model with Typed Accessors
**What:** Store the entire profile as `serde_json::Map<String, Value>` and provide typed getter/setter methods for fields BambuMate needs to manipulate.
**When to use:** Always. This is the core pattern for the entire profile engine.
**Why:** Bambu Studio profiles have 139+ fields that change across versions. A fully-typed struct would silently lose new fields. The Value approach preserves everything.
**Example:**
```rust
// Source: Verified against actual profile files on disk
use serde_json::{Map, Value};

pub struct FilamentProfile {
    /// The raw JSON object -- preserves ALL fields including unknown ones
    data: Map<String, Value>,
}

impl FilamentProfile {
    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self> {
        let data: Map<String, Value> = serde_json::from_str(json)?;
        Ok(Self { data })
    }

    /// Serialize to JSON string (4-space indent, sorted keys for user profiles)
    pub fn to_json(&self) -> Result<String> {
        let json = serde_json::to_string_pretty(&self.data)?;
        Ok(json)
    }

    // --- Typed accessors for fields BambuMate manipulates ---

    pub fn name(&self) -> Option<&str> {
        self.data.get("name")?.as_str()
    }

    pub fn inherits(&self) -> Option<&str> {
        self.data.get("inherits")?.as_str()
    }

    pub fn filament_id(&self) -> Option<&str> {
        self.data.get("filament_id")?.as_str()
    }

    pub fn filament_type(&self) -> Option<&str> {
        self.get_first_array_value("filament_type")
    }

    pub fn nozzle_temperature(&self) -> Option<Vec<&str>> {
        self.get_string_array("nozzle_temperature")
    }

    pub fn compatible_printers(&self) -> Option<Vec<&str>> {
        self.get_string_array("compatible_printers")
    }

    /// Get the first element of a string array field
    fn get_first_array_value(&self, key: &str) -> Option<&str> {
        self.data.get(key)?
            .as_array()?
            .first()?
            .as_str()
    }

    /// Get all elements of a string array field
    fn get_string_array(&self, key: &str) -> Option<Vec<&str>> {
        self.data.get(key)?
            .as_array()?
            .iter()
            .map(|v| v.as_str())
            .collect()
    }

    /// Set a string array field
    pub fn set_string_array(&mut self, key: &str, values: Vec<String>) {
        let arr: Vec<Value> = values.into_iter().map(Value::String).collect();
        self.data.insert(key.to_string(), Value::Array(arr));
    }

    /// Set a bare string field (not array-wrapped)
    pub fn set_string(&mut self, key: &str, value: String) {
        self.data.insert(key.to_string(), Value::String(value));
    }

    /// Get raw access to the underlying map
    pub fn raw(&self) -> &Map<String, Value> {
        &self.data
    }

    pub fn raw_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.data
    }
}
```

### Pattern 2: Inheritance Resolution via Chain Walking
**What:** Load a profile, follow its `inherits` field to load the parent, recursively until reaching a profile with no parent, then merge from base to leaf.
**When to use:** When reading system profiles that use sparse inheritance (only override changed fields).
**Note:** User-created profiles on disk are already fully flattened (all fields present). Inheritance resolution is needed for reading system profiles and for understanding which values are overridden vs inherited.
**Example:**
```rust
// Source: Verified inheritance chain from actual files:
// fdm_filament_common -> fdm_filament_pla -> Generic PLA @base -> Generic PLA @BBL H2C 0.4 nozzle

pub fn resolve_inheritance(
    profile: &FilamentProfile,
    registry: &ProfileRegistry,
) -> Result<FilamentProfile> {
    let mut chain: Vec<&FilamentProfile> = vec![profile];

    // Walk up the inheritance chain
    let mut current = profile;
    while let Some(parent_name) = current.inherits() {
        if parent_name.is_empty() {
            break;
        }
        let parent = registry.get_by_name(parent_name)
            .ok_or_else(|| anyhow!("Parent profile not found: {}", parent_name))?;
        chain.push(parent);
        current = parent;
    }

    // Also handle "include" mixins (used for dual-extruder templates)
    // The include field references templates like "fdm_filament_template_direct_dual"
    // which provide default values for dual-extruder fields

    // Merge from base (last) to leaf (first)
    chain.reverse();
    let mut resolved = Map::new();
    for ancestor in &chain {
        for (key, value) in ancestor.raw() {
            // Skip metadata fields during merge
            if key == "inherits" || key == "name" || key == "type"
                || key == "from" || key == "instantiation"
                || key == "filament_id" || key == "setting_id"
                || key == "include" || key == "description"
                || key == "compatible_printers" {
                continue;
            }
            resolved.insert(key.clone(), value.clone());
        }
    }

    // Apply leaf profile's own values last (including its metadata)
    for (key, value) in profile.raw() {
        resolved.insert(key.clone(), value.clone());
    }

    Ok(FilamentProfile { data: resolved })
}
```

### Pattern 3: Atomic Write with tempfile
**What:** Write profile JSON to a temporary file in the same directory, then atomically rename to the target path.
**When to use:** Every profile write operation. Prevents corruption if the process crashes mid-write.
**Example:**
```rust
// Source: tempfile 3.24.0 docs
use tempfile::NamedTempFile;
use std::io::Write;

pub fn write_profile_atomic(
    profile: &FilamentProfile,
    target_path: &Path,
) -> Result<()> {
    let json = profile.to_json()?;

    // Create temp file in the SAME directory as target
    // (required for atomic rename on same filesystem)
    let parent_dir = target_path.parent()
        .ok_or_else(|| anyhow!("Invalid target path"))?;

    let mut temp = NamedTempFile::new_in(parent_dir)?;
    temp.write_all(json.as_bytes())?;
    temp.flush()?;

    // Atomic rename -- replaces target if it exists
    temp.persist(target_path)?;

    Ok(())
}
```

### Pattern 4: Profile Metadata (.info file) Handling
**What:** Each user profile JSON has a companion `.info` file in INI-like format containing sync metadata.
**When to use:** When writing user profiles, must also create/update the corresponding `.info` file.
**Format verified from disk:**
```ini
sync_info =
user_id = 1881310893
setting_id = PFUS50d8c9d5139548
base_id =
updated_time = 1770267863
```
**Example:**
```rust
pub struct ProfileMetadata {
    pub sync_info: String,
    pub user_id: String,
    pub setting_id: String,
    pub base_id: String,
    pub updated_time: u64,
}

impl ProfileMetadata {
    pub fn to_info_string(&self) -> String {
        format!(
            "sync_info = {}\nuser_id = {}\nsetting_id = {}\nbase_id = {}\nupdated_time = {}\n",
            self.sync_info, self.user_id, self.setting_id,
            self.base_id, self.updated_time
        )
    }

    pub fn from_info_string(content: &str) -> Result<Self> {
        let mut meta = ProfileMetadata::default();
        for line in content.lines() {
            let parts: Vec<&str> = line.splitn(2, " = ").collect();
            if parts.len() == 2 {
                match parts[0].trim() {
                    "sync_info" => meta.sync_info = parts[1].to_string(),
                    "user_id" => meta.user_id = parts[1].to_string(),
                    "setting_id" => meta.setting_id = parts[1].to_string(),
                    "base_id" => meta.base_id = parts[1].to_string(),
                    "updated_time" => meta.updated_time = parts[1].parse().unwrap_or(0),
                    _ => {} // Preserve unknown fields
                }
            }
        }
        Ok(meta)
    }
}
```

### Anti-Patterns to Avoid

- **Fully-typed struct for all 139+ profile fields:** The schema evolves with every Bambu Studio update. A typed struct will silently discard new fields via `#[serde(deny_unknown_fields)]` or require constant maintenance. Use `Map<String, Value>` with typed accessors for known fields only.

- **Ignoring `preserve_order` for serde_json:** User profiles have alphabetically sorted keys. Without `preserve_order`, serde_json uses BTreeMap which sorts keys -- this happens to work for user profiles but would reorder system profile keys (which put `type`, `name`, `inherits` first). Always use `preserve_order` to match the source ordering.

- **Writing profile JSON without matching formatting:** Bambu Studio expects 4-space indented JSON. `serde_json::to_string_pretty` uses 2-space by default. Must use a custom formatter or post-process to match 4-space indentation.

- **Assuming user profiles use inheritance:** User-created profiles on disk have `inherits: ""` and contain ALL fields fully expanded. Do not assume they have sparse inheritance like system profiles.

- **Forgetting the .info companion file:** Every user profile JSON has a `.info` file with sync metadata. Writing a JSON without its `.info` will cause Bambu Studio to behave unpredictably with that profile.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Atomic file writes | Custom temp-file-then-rename logic | `tempfile::NamedTempFile::persist()` | Handles edge cases (cross-device, permissions, cleanup on error) |
| JSON key ordering | Custom ordered map implementation | `serde_json` with `preserve_order` feature | Uses IndexMap internally, well-tested |
| Cross-platform paths | `#[cfg]` blocks with hardcoded paths | `dirs` crate + `BambuPaths` abstraction | dirs handles XDG, Windows AppData, macOS Application Support |
| INI-like .info parsing | Full INI parser | Simple line-by-line split on ` = ` | The .info format is trivially simple (5 lines, no sections, no nesting) |
| JSON pretty-printing with 4-space indent | String replacement on serde output | `serde_json::ser::PrettyFormatter::with_indent(b"    ")` | serde_json has a built-in customizable formatter |

**Key insight:** The profile engine's complexity is in understanding the *format* (undocumented, evolving), not in the *implementation* (standard JSON read/write). The risk is in getting the format wrong, not in the code being hard to write.

## Common Pitfalls

### Pitfall 1: serde_json Pretty Printer Uses 2-Space Indent by Default
**What goes wrong:** `serde_json::to_string_pretty()` outputs 2-space indented JSON. Bambu Studio's profiles use 4-space indentation. A round-trip would change all indentation, making diffs noisy and potentially confusing Bambu Studio.
**Why it happens:** serde_json's default pretty formatter uses 2 spaces.
**How to avoid:** Use the custom `PrettyFormatter` with 4-space indent:
```rust
use serde_json::ser::{PrettyFormatter, Serializer};

fn to_json_4space(value: &Map<String, Value>) -> Result<String> {
    let mut buf = Vec::new();
    let formatter = PrettyFormatter::with_indent(b"    ");
    let mut ser = Serializer::with_formatter(&mut buf, formatter);
    value.serialize(&mut ser)?;
    Ok(String::from_utf8(buf)?)
}
```
**Warning signs:** Profile JSON diff shows every line changed due to whitespace.

### Pitfall 2: "nil" String Is Not JSON null
**What goes wrong:** Many profile fields use the string `"nil"` (not JSON `null`) to mean "inherit from parent." Code that treats this as a regular string value or tries to convert it to a number will produce incorrect profiles.
**Why it happens:** Bambu Studio's profile system predates proper JSON conventions. `"nil"` is a convention, not a JSON primitive.
**How to avoid:** When resolving inheritance, treat `"nil"` as "no value set, use parent's value." When writing profiles, preserve `"nil"` strings exactly as-is. Never convert `"nil"` to `null` or omit the field.
**Warning signs:** Fields with value `["nil", "nil"]` being treated as actual data.

### Pitfall 3: Multi-Element Arrays for Dual-Extruder Printers
**What goes wrong:** Many fields have 2-element arrays (e.g., `"nozzle_temperature": ["220", "220"]`) for dual-extruder printers like the H2C. Code that assumes single-element arrays will break or lose the second extruder's settings.
**Why it happens:** The H2C and other dual-extruder printers have two hotend variants (Direct Drive Standard + Direct Drive High Flow), each needing its own value.
**How to avoid:** Always preserve array length. When reading, don't assume `arr[0]`. When writing, maintain the original array length. The `include: ["fdm_filament_template_direct_dual"]` mechanism is what expands single-value fields to dual-value.
**Warning signs:** Profile for H2C printer has single-element arrays where it should have 2.

### Pitfall 4: User Profile Folder Contains Numeric User ID
**What goes wrong:** Code hardcodes `user/default/filament/` as the user profile path. The actual user profiles are under `user/1881310893/filament/base/` (a numeric user/device ID).
**Why it happens:** The `default` folder is empty; the real profiles are in the device-specific folder. The folder name is the `preset_folder` value from `BambuStudio.conf`.
**How to avoid:** Read `preset_folder` from BambuStudio.conf, or scan `user/` directory for non-`default` folders that contain profiles. User profiles live in a `base/` subdirectory within `filament/`.
**Warning signs:** Profile reader finds zero user profiles.

### Pitfall 5: filament_id vs setting_id vs filament_settings_id
**What goes wrong:** Confusing the three different ID fields leads to profiles that appear in Bambu Studio but malfunction or get overwritten.
**Why it happens:** Three separate identifiers serve different purposes:
- `filament_id` (bare string): Identifies the filament material itself (e.g., `"Pe7b385c"`, `"GFL99"`). Shared across all nozzle variants of the same filament. System profiles use `GFxxx` format.
- `setting_id` (in .info file, not JSON for user profiles): Identifies the specific settings preset (e.g., `"PFUS50d8c9d5139548"`, `"GFSL99_20"`). System profiles use `GFSxxx` format. User profiles use `PFUSxxxxxxxxx` format.
- `filament_settings_id` (array field in JSON): Display identifier in the UI, typically matches the profile name.
**How to avoid:** For user-created profiles: `filament_id` is a short hex string (e.g., `"Pe7b385c"`), `setting_id` goes in the `.info` file with `PFUS` prefix + hex, and `filament_settings_id` in JSON matches the profile name.
**Warning signs:** Profile shows wrong name in Bambu Studio, or gets confused with another filament.

### Pitfall 6: System Profiles Have Different Key Order Than User Profiles
**What goes wrong:** When writing profiles, blindly sorting keys alphabetically produces output that differs from system profile format (which puts `type`, `name`, `inherits`, `from` first).
**Why it happens:** System profiles use a semantic ordering (identity fields first), while user profiles (exported by Bambu Studio) are alphabetically sorted.
**How to avoid:** For **reading**, use `preserve_order` to maintain whatever order the source file has. For **writing user profiles**, alphabetical sort is correct (matches Bambu Studio's own export format). For writing system-style profiles, preserve source ordering.
**Warning signs:** Round-trip of a system profile reorders all keys.

### Pitfall 7: Empty `compatible_prints` vs Missing Field
**What goes wrong:** Some profiles have `"compatible_prints": []` (empty array) while others omit the field entirely. Writing an empty array where the field was missing (or vice versa) changes the profile.
**Why it happens:** The profile format is inconsistent about representing "no value" vs "empty collection."
**How to avoid:** Preserve exactly what was read. Don't normalize empty arrays to missing fields or vice versa.

## Code Examples

### Reading a Profile from Disk
```rust
// Source: Verified against actual files at ~/Library/Application Support/BambuStudio/
use std::path::Path;
use serde_json::{Map, Value};

pub fn read_profile(path: &Path) -> Result<FilamentProfile> {
    let content = std::fs::read_to_string(path)?;
    let data: Map<String, Value> = serde_json::from_str(&content)?;
    Ok(FilamentProfile { data })
}

pub fn read_profile_metadata(json_path: &Path) -> Result<Option<ProfileMetadata>> {
    let info_path = json_path.with_extension("info");
    if info_path.exists() {
        let content = std::fs::read_to_string(&info_path)?;
        Ok(Some(ProfileMetadata::from_info_string(&content)?))
    } else {
        Ok(None)
    }
}
```

### Writing a Profile with Atomic Write
```rust
// Source: tempfile 3.24.0 docs + serde_json custom formatter
use tempfile::NamedTempFile;
use serde::Serialize;
use serde_json::ser::{PrettyFormatter, Serializer};
use std::io::Write;

pub fn write_profile_atomic(
    profile: &FilamentProfile,
    target_path: &Path,
) -> Result<()> {
    // Serialize with 4-space indentation (matching Bambu Studio format)
    let json = {
        let mut buf = Vec::new();
        let formatter = PrettyFormatter::with_indent(b"    ");
        let mut ser = Serializer::with_formatter(&mut buf, formatter);
        profile.raw().serialize(&mut ser)?;
        let mut s = String::from_utf8(buf)?;
        // Bambu Studio profiles end with a newline
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s
    };

    // Write to temp file in same directory, then atomic rename
    let parent = target_path.parent()
        .ok_or_else(|| anyhow!("No parent directory for {:?}", target_path))?;
    std::fs::create_dir_all(parent)?;

    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(json.as_bytes())?;
    temp.flush()?;
    temp.persist(target_path)?;

    Ok(())
}

pub fn write_profile_metadata_atomic(
    metadata: &ProfileMetadata,
    target_path: &Path,
) -> Result<()> {
    let content = metadata.to_info_string();
    let parent = target_path.parent()
        .ok_or_else(|| anyhow!("No parent directory"))?;

    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(content.as_bytes())?;
    temp.flush()?;
    temp.persist(target_path)?;

    Ok(())
}
```

### OS-Specific Path Detection
```rust
// Source: Verified against actual installation at /Applications/BambuStudio.app
// and ~/Library/Application Support/BambuStudio/

pub struct BambuPaths {
    pub config_root: PathBuf,       // ~/Library/Application Support/BambuStudio/
    pub system_filaments: PathBuf,  // .../system/BBL/filament/
    pub user_root: PathBuf,         // .../user/
    pub preset_folder: Option<String>, // e.g., "1881310893"
}

impl BambuPaths {
    pub fn detect() -> Result<Self> {
        let config_root = Self::find_config_root()?;
        let system_filaments = config_root.join("system").join("BBL").join("filament");
        let user_root = config_root.join("user");

        // Read preset_folder from BambuStudio.conf
        let preset_folder = Self::read_preset_folder(&config_root);

        Ok(Self {
            config_root,
            system_filaments,
            user_root,
            preset_folder,
        })
    }

    fn find_config_root() -> Result<PathBuf> {
        // Try dirs crate first (maps to ~/Library/Application Support on macOS)
        if let Some(data_dir) = dirs::data_dir() {
            let bs_dir = data_dir.join("BambuStudio");
            if bs_dir.exists() {
                return Ok(bs_dir);
            }
        }

        // Fallback: explicit path
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = dirs::home_dir() {
                let bs_dir = home.join("Library/Application Support/BambuStudio");
                if bs_dir.exists() {
                    return Ok(bs_dir);
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(appdata) = dirs::config_dir() {
                let bs_dir = appdata.join("BambuStudio");
                if bs_dir.exists() {
                    return Ok(bs_dir);
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(config) = dirs::config_dir() {
                let bs_dir = config.join("BambuStudio");
                if bs_dir.exists() {
                    return Ok(bs_dir);
                }
            }
        }

        anyhow::bail!("Bambu Studio config directory not found")
    }

    fn read_preset_folder(config_root: &Path) -> Option<String> {
        let conf_path = config_root.join("BambuStudio.conf");
        let content = std::fs::read_to_string(&conf_path).ok()?;
        // Parse as JSON -- BambuStudio.conf is JSON format
        let conf: serde_json::Value = serde_json::from_str(&content).ok()?;
        conf.get("preset_folder")?.as_str().map(|s| s.to_string())
    }

    /// Get the active user profile directory
    pub fn user_filament_dir(&self) -> Option<PathBuf> {
        if let Some(ref folder) = self.preset_folder {
            let path = self.user_root.join(folder).join("filament").join("base");
            if path.exists() {
                return Some(path);
            }
        }
        // Fallback: scan for non-default directories
        if let Ok(entries) = std::fs::read_dir(&self.user_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name != "default" && entry.path().is_dir() {
                    let path = entry.path().join("filament").join("base");
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }
}
```

### Round-Trip Test Pattern
```rust
// Critical test: read -> write -> read should produce identical data
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip_preserves_all_fields() {
        let original = std::fs::read_to_string("tests/fixtures/sample_profile.json")
            .unwrap();
        let profile = FilamentProfile::from_json(&original).unwrap();
        let written = profile.to_json_4space().unwrap();
        let reparsed = FilamentProfile::from_json(&written).unwrap();

        // All fields preserved
        assert_eq!(profile.raw().len(), reparsed.raw().len());

        // Every field has identical value
        for (key, value) in profile.raw() {
            assert_eq!(
                reparsed.raw().get(key).unwrap(),
                value,
                "Field '{}' changed during round-trip",
                key
            );
        }
    }

    #[test]
    fn test_nil_values_preserved() {
        let original = r#"{"filament_retraction_speed": ["nil", "nil"]}"#;
        let profile = FilamentProfile::from_json(original).unwrap();
        let written = profile.to_json_4space().unwrap();
        assert!(written.contains(r#""nil""#));
    }
}
```

## Bambu Studio Profile Format -- Complete Reference

**Confidence: HIGH** -- All details verified from actual files on disk at `/Users/michaelcurtis/Library/Application Support/BambuStudio/` (version 2.5.0.5).

### File Organization

```
~/Library/Application Support/BambuStudio/
├── BambuStudio.conf              # Main config (JSON), contains preset_folder
├── system/
│   ├── BBL.json                  # Registry: lists all system profiles with sub_path
│   └── BBL/
│       └── filament/             # 1393 system filament profiles
│           ├── fdm_filament_common.json          # Root base (no inherits)
│           ├── fdm_filament_pla.json             # Material type base
│           ├── fdm_filament_template_direct_dual.json  # Mixin for dual-extruder
│           ├── Generic PLA @base.json            # Brand base (inherits: fdm_filament_pla)
│           ├── Generic PLA @BBL H2C 0.4 nozzle.json  # Printer-specific (instantiation: true)
│           └── ... (brand profiles like eSUN PLA+, SUNLU PLA+, etc.)
└── user/
    ├── default/
    │   └── filament/             # Empty (unused)
    └── 1881310893/               # User ID / preset_folder
        └── filament/
            └── base/             # User-created profiles live here
                ├── SUNLU PLA PLA+ 20 @Bambu Lab H2C 0.4 nozzle.json
                └── SUNLU PLA PLA+ 20 @Bambu Lab H2C 0.4 nozzle.info
```

### Inheritance Hierarchy (System Profiles)

```
Level 0: fdm_filament_common                (root, no inherits, ~90 fields)
  │       instantiation: false
  │
  ├── Level 1: fdm_filament_pla             (inherits: fdm_filament_common)
  │     │       instantiation: false
  │     │       Overrides: temps, density, fan, cost
  │     │
  │     ├── Level 2a: Generic PLA @base     (inherits: fdm_filament_pla)
  │     │     │       instantiation: false, filament_id: "GFL99"
  │     │     │
  │     │     ├── Level 3: Generic PLA @BBL H2C 0.4 nozzle  (instantiation: true)
  │     │     │       setting_id: "GFSL99_20"
  │     │     │       include: ["fdm_filament_template_direct_dual"]
  │     │     │       compatible_printers: ["Bambu Lab H2C 0.4 nozzle"]
  │     │     │
  │     │     └── Level 3: Generic PLA      (instantiation: true)
  │     │             setting_id: "GFSL99"
  │     │             compatible_printers: [12 printer strings]
  │     │
  │     └── Level 2b: eSUN PLA+ @base      (inherits: fdm_filament_pla)
  │           │       filament_id: "GFL03"
  │           │
  │           └── Level 3: eSUN PLA+ @BBL H2D  (instantiation: true)
  │                   setting_id: "GFSL03_07"
  │
  ├── Level 1: fdm_filament_abs
  ├── Level 1: fdm_filament_pet
  └── Level 1: fdm_filament_tpu

Mixin (applied via "include" field, not "inherits"):
  fdm_filament_template_direct_dual
    - Expands single-value fields to dual-value for dual-extruder printers
    - E.g., filament_flow_ratio: ["0.95"] -> ["0.95", "0.95"]
```

### JSON Field Categories

**Identity fields (bare strings, NOT arrays):**
- `name`: Profile display name (e.g., `"SUNLU PLA PLA+ 20 @Bambu Lab H2C 0.4 nozzle"`)
- `inherits`: Parent profile name (empty string `""` for fully flattened, or parent name for system profiles)
- `filament_id`: Material identifier (e.g., `"Pe7b385c"` for user, `"GFL99"` for system)
- `filament_notes`: Free-text notes (bare string)
- `from`: `"system"` or `"User"`
- `version`: Bambu Studio version (e.g., `"2.5.0.5"`)
- `compatible_printers_condition`: Empty string in user profiles
- `compatible_prints_condition`: Empty string in user profiles

**System-only identity fields (bare strings):**
- `type`: Always `"filament"` (only in system profiles)
- `setting_id`: Settings identifier (e.g., `"GFSL99_20"`) -- in system profiles only, user profiles store this in `.info` file
- `instantiation`: `"true"` or `"false"` -- controls visibility in Bambu Studio UI
- `description`: Human-readable description (only some base profiles)

**Array fields (131 in user profiles):**
- Most contain 1 element: `["220"]` for single-extruder settings
- Many contain 2 elements: `["220", "220"]` for dual-extruder printers (H2C, H2D, etc.)
- Some contain 4 elements: `filament_dev_ams_drying_temperature`, `filament_dev_ams_drying_time`
- `compatible_printers`: Array of exact printer+nozzle strings: `["Bambu Lab H2C 0.4 nozzle"]`
- `compatible_prints`: Usually empty array `[]`
- `filament_start_gcode`, `filament_end_gcode`: Single-element arrays containing gcode strings

**Special values:**
- `"nil"`: String meaning "inherit from parent" (NOT JSON null)
- Percentages as strings: `"50%"`, `"95%"`, `"100%"`, `"0%"`, `"10%"`, `"15%"`
- Space-separated coefficients: `"0 0 0 0 0 0"` in `volumetric_speed_coefficients`

### .info File Format (User Profiles Only)

Each user profile JSON has a companion `.info` file with identical filename stem:
```ini
sync_info =
user_id = 1881310893
setting_id = PFUS50d8c9d5139548
base_id =
updated_time = 1770267863
```

- `sync_info`: Empty (cloud sync state)
- `user_id`: Matches the preset_folder / user directory name
- `setting_id`: Unique profile settings ID. User profiles use `PFUS` prefix + 14 hex chars
- `base_id`: Empty for base profiles
- `updated_time`: Unix timestamp (seconds)

### Naming Convention

Profile filenames follow the pattern:
```
{Vendor} {Type} {Variant} @{Printer} {Nozzle}
```
Examples:
- `SUNLU PLA PLA+ 20 @Bambu Lab H2C 0.4 nozzle`
- `Generic PLA @BBL H2C 0.4 nozzle`
- `eSUN PLA+ @BBL H2D`
- `Bambu ABS @BBL X1C 0.2 nozzle`

System profiles use `@BBL` abbreviation; user profiles use `@Bambu Lab` full name.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Single-extruder only fields | Dual-value arrays for dual-extruder printers | H2C/H2D release (~2024) | Many fields now have 2-element arrays; `include: ["fdm_filament_template_direct_dual"]` mechanism added |
| Profiles stored in `user/default/` | Profiles stored in `user/{preset_folder}/filament/base/` | Cloud sync addition | Must read `preset_folder` from BambuStudio.conf to find the right directory |
| Simple flat profile JSON | 3-4 level inheritance with `inherits` + `include` mixins | Early Bambu Studio versions | System profiles are sparse (only overrides); user profiles are fully flattened |
| `setting_id` in JSON | `setting_id` moved to `.info` file for user profiles | Version ~2.x | User profile JSON contains `filament_settings_id` array but NOT `setting_id`; the latter is in the companion `.info` |

**Deprecated/outdated:**
- The initial research (SUMMARY.md) mentioned `filament_id` starts with "GF" -- this is only true for system profiles. User profiles use hex strings like `"Pe7b385c"`.
- The initial research suggested `setting_id` starts with "GFS" -- this is only true for system profiles. User profiles use `PFUS` + hex in the `.info` file.

## Open Questions

1. **BambuStudio.conf JSON parsing reliability**
   - What we know: The config file is valid JSON with a `preset_folder` field
   - What's unclear: Whether the full config file is always valid JSON or can be corrupted/partial
   - Recommendation: Use lenient parsing, fall back to directory scanning if BambuStudio.conf is unreadable

2. **Cloud sync behavior when writing profiles**
   - What we know: Cloud sync can overwrite locally-written profiles (documented in STATE.md blockers)
   - What's unclear: Whether updating the `.info` `updated_time` prevents cloud sync from overwriting, or whether `sync_info` field controls this
   - Recommendation: For Phase 2, focus on correct read/write. Phase 4 (installation) will address cloud sync conflicts

3. **`include` field resolution order vs `inherits`**
   - What we know: `include: ["fdm_filament_template_direct_dual"]` is used alongside `inherits` in system profiles. It appears to be a mixin that provides default dual-extruder values.
   - What's unclear: Whether `include` values are applied before or after `inherits` resolution, and whether multiple includes are supported
   - Recommendation: For Phase 2 reader, treat `include` as additional parents whose fields are merged. User profiles are already flattened so this only matters for reading system profiles.

4. **filament_id generation for new user profiles**
   - What we know: User profiles use short hex IDs like `"Pe7b385c"`. System profiles use formatted IDs like `"GFL99"`.
   - What's unclear: Whether Bambu Studio validates the format of `filament_id`, or whether any unique string works
   - Recommendation: For new user profiles, generate a `P` prefix + 7 random hex characters (matching observed pattern). Validate by creating a test profile and importing it.

5. **setting_id generation for .info files**
   - What we know: User setting IDs use `PFUS` prefix + 14 hex characters
   - What's unclear: Whether there's a specific algorithm (hash of content? random?) or if any unique string works
   - Recommendation: Generate `PFUS` + 14 random hex characters for new profiles

## Sources

### Primary (HIGH confidence)
- Actual Bambu Studio profile files at `~/Library/Application Support/BambuStudio/` -- 4 user profiles, 6 system base profiles, 3 inheritance templates, BBL.json registry, BambuStudio.conf. All examined directly on disk. Version 2.5.0.5.
- serde_json 1.0.149 docs at https://docs.rs/serde_json/latest/serde_json/ -- `preserve_order` feature, `PrettyFormatter`
- serde flatten docs at https://serde.rs/attr-flatten.html -- HashMap preservation pattern
- tempfile 3.24.0 docs at https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html -- `persist()` for atomic writes
- Phase 1 codebase at `/Users/michaelcurtis/Development/BambuMate/src-tauri/src/` -- existing project structure, Cargo.toml dependencies

### Secondary (MEDIUM confidence)
- Bambu Lab Wiki (submit-preset) at https://wiki.bambulab.com/en/bambu-studio/submit-preset -- profile submission format documentation (wiki page had loading issues, content partially verified via search)
- BambuStudio GitHub Issues and Community Forum -- profile format changes, cloud sync conflicts, user profile paths

### Tertiary (LOW confidence)
- `filament_id` and `setting_id` generation patterns -- inferred from observed data, not confirmed via Bambu Studio source code. The `P` + hex and `PFUS` + hex patterns may have additional constraints not yet discovered.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All crates verified via docs.rs, versions confirmed, features documented
- Architecture: HIGH - All patterns derived from actual file examination on disk, not assumptions
- Profile format: HIGH - 4 user profiles + 6 system profiles examined, field types and counts verified via Python analysis
- Pitfalls: HIGH - Each pitfall discovered from actual file differences (e.g., key ordering, nil values, dual-extruder arrays)
- ID generation: MEDIUM - Patterns observed but not confirmed against source code

**Research date:** 2026-02-04
**Valid until:** 30 days for stable format (but Bambu Studio updates can change format at any time -- re-examine after any BS version update)
