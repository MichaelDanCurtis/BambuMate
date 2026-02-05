# Phase 4: Profile Generation & Installation - Research

**Researched:** 2026-02-05
**Domain:** Bambu Studio profile generation from scraped specs, profile installation to BS config directory, process detection, Leptos UI for filament search
**Confidence:** HIGH

## Summary

This phase bridges Phase 2 (Profile Engine) and Phase 3 (Filament Scraper) to deliver the core user workflow: search for a filament, view its specs, generate a Bambu Studio profile, and install it. The research examined the existing codebase (both profile engine and scraper modules), real Bambu Studio profile formats on disk, the cloud sync overwrite issue, process detection approaches for BS running detection, and Leptos patterns for the search UI.

The profile generation task is a mapping problem: `FilamentSpecs` (from the scraper) must be transformed into a `FilamentProfile` (the BS JSON format). This involves: (1) loading a base system profile (e.g., "Generic PLA") via the existing `ProfileRegistry`, (2) cloning it and overriding fields with scraped values, (3) generating unique IDs (`filament_id`, `setting_id`, `filament_settings_id`), and (4) writing both the `.json` and `.info` files atomically to the user filament directory. The critical insight is that user profiles in Bambu Studio are **fully flattened** (all ~139 fields present, `inherits: ""`), so the generator must resolve the base profile's full inheritance chain first, then apply overrides.

For process detection (PROF-05), `sysinfo` 0.38 provides cross-platform process name lookup via `processes_by_name()`. However, to avoid adding a heavyweight dependency, `std::process::Command` with `pgrep` on macOS is a lighter alternative. The cloud sync overwrite concern (documented in STATE.md) is addressable by setting the `.info` file's `updated_time` to the current Unix timestamp and leaving `sync_info` empty -- the key finding from community forums is that corrupted `"nil"` values in array fields (not the metadata) are the primary trigger for sync overwrites.

**Primary recommendation:** Build a `ProfileGenerator` struct that takes `FilamentSpecs` + base profile name, resolves the base via `ProfileRegistry` + `resolve_inheritance()`, applies field overrides using the existing `set_string`/`set_string_array` mutators, generates unique IDs, and writes via the existing `write_profile_with_metadata()`. Use `std::process::Command` + `pgrep` for lightweight BS detection on macOS (with architectural hooks for Windows/Linux).

## Standard Stack

### Core (Already in Project)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| serde_json | 1.0 (preserve_order) | Profile JSON manipulation | Already used by profile engine; Map<String, Value> model is the foundation |
| tempfile | 3.24 | Atomic writes for generated profiles | Already used by writer.rs; proven pattern |
| anyhow | 1.0 | Error handling | Already used throughout |
| tracing | 0.1 | Structured logging | Already used throughout |
| dirs | 6.0 | OS-specific path resolution | Already used by paths.rs |
| walkdir | 2.5 | Profile discovery | Already used by registry.rs |
| chrono | 0.4 | Timestamp generation for .info files | Already used by cache; needed for `updated_time` |
| tokio | 1 | Async runtime | Already available; needed for async Tauri commands |

### New Dependencies
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| rand | 0.9 | Generate random hex for filament_id and setting_id | Lightweight, widely used; only need `rand::rng().random::<[u8; N]>()` |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `rand` for ID generation | `uuid` crate | uuid generates full UUIDs (128-bit); we need short hex strings (7-14 chars) matching BS conventions. `rand` is simpler and already sufficient |
| `sysinfo` for process detection | `std::process::Command` + `pgrep` | sysinfo is 0.38 with MSRV 1.88, comprehensive but heavy (pulls in system info); pgrep is zero-dependency, macOS-native, and sufficient for "is process running?" check |
| New frontend framework for search | Existing Leptos 0.8 signals + resources | No new framework needed; Leptos reactive primitives (signals, resources, effects) handle search/debounce/display natively |

**Installation (addition to Cargo.toml):**
```toml
rand = "0.9"
```

## Architecture Patterns

### Recommended Project Structure
```
src-tauri/src/
  profile/
    mod.rs              # Add: pub mod generator;
    generator.rs        # NEW: FilamentSpecs -> FilamentProfile mapping
    types.rs            # Existing: FilamentProfile, ProfileMetadata
    paths.rs            # Existing: BambuPaths, user_filament_dir()
    reader.rs           # Existing
    writer.rs           # Existing: write_profile_with_metadata()
    inheritance.rs      # Existing: resolve_inheritance()
    registry.rs         # Existing: ProfileRegistry, get_by_name()
  commands/
    profile.rs          # EXTEND: add generate_profile, install_profile, search_and_generate commands
    scraper.rs          # Existing: search_filament command
  scraper/
    types.rs            # Existing: FilamentSpecs, MaterialType
src/
  pages/
    mod.rs              # Add: pub mod filament_search;
    filament_search.rs  # NEW: Search UI page
  components/
    mod.rs              # Add: pub mod filament_card; pub mod profile_preview;
    filament_card.rs    # NEW: Display FilamentSpecs results
    profile_preview.rs  # NEW: Preview generated profile before install
  commands.rs           # EXTEND: add invoke wrappers for new commands
  app.rs                # EXTEND: add /filament route
  components/sidebar.rs # EXTEND: add "Filament Search" nav item
```

### Pattern 1: Profile Generator (FilamentSpecs -> FilamentProfile)
**What:** A function that takes scraped specs and a base profile name, resolves the base profile's full inheritance chain, then overlays the scraped values to produce a fully-flattened user profile.
**When to use:** Every time a user generates a profile from search results.
**Key insight:** The generated profile must be fully flattened (all ~139 fields present, `inherits: ""`) because that is how Bambu Studio stores user profiles. The generator must resolve the base profile's inheritance first, then set identity/override fields.

```rust
// Source: Derived from existing codebase patterns in types.rs, inheritance.rs, registry.rs
use crate::profile::{FilamentProfile, ProfileMetadata, ProfileRegistry, BambuPaths};
use crate::profile::inheritance::resolve_inheritance;
use crate::scraper::types::{FilamentSpecs, MaterialType};
use chrono::Utc;

/// Map a MaterialType to the corresponding Bambu Studio base profile name.
/// These are the "Generic X" profiles that ship with Bambu Studio.
fn base_profile_name(material: &MaterialType) -> &'static str {
    match material {
        MaterialType::PLA => "Generic PLA",
        MaterialType::PETG => "Generic PETG",
        MaterialType::ABS => "Generic ABS",
        MaterialType::ASA => "Generic ASA",
        MaterialType::TPU => "Generic TPU",
        MaterialType::Nylon => "Generic PA",
        MaterialType::PC => "Generic PC",
        MaterialType::PVA => "Generic PVA",
        MaterialType::HIPS => "Generic HIPS",
        MaterialType::Other(_) => "Generic PLA", // Safe fallback
    }
}

/// Generate a random filament_id in the format "P" + 7 hex chars.
fn generate_filament_id() -> String {
    let bytes: [u8; 4] = rand::random();
    format!("P{:07x}", u32::from_be_bytes(bytes) & 0x0FFFFFFF)
}

/// Generate a random setting_id in the format "PFUS" + 14 hex chars.
fn generate_setting_id() -> String {
    let bytes: [u8; 7] = rand::random();
    format!("PFUS{}", hex::encode(bytes))
}

pub fn generate_profile(
    specs: &FilamentSpecs,
    registry: &ProfileRegistry,
    target_printer: Option<&str>,  // e.g., "Bambu Lab H2C 0.4 nozzle"
) -> Result<(FilamentProfile, ProfileMetadata)> {
    let material = MaterialType::from_str(&specs.material);
    let base_name = base_profile_name(&material);

    // 1. Find and resolve the base profile
    let base = registry.get_by_name(base_name)
        .ok_or_else(|| anyhow!("Base profile '{}' not found", base_name))?;
    let mut profile = resolve_inheritance(base, registry)?;

    // 2. Set identity fields
    let profile_name = format!(
        "{} {} @{}",
        specs.brand,
        specs.name,
        target_printer.unwrap_or("Bambu Lab H2C 0.4 nozzle")
    );
    profile.set_string("name", profile_name.clone());
    profile.set_string("inherits", String::new());  // Fully flattened
    profile.set_string("from", "User".to_string());
    profile.set_string("filament_id", generate_filament_id());

    // 3. Override with scraped values (array format, duplicated for dual-extruder)
    if let Some(temp) = specs.nozzle_temp_max {
        let t = temp.to_string();
        profile.set_string_array("nozzle_temperature", vec![t.clone(), t]);
    }
    // ... similar for bed_temperature, fan speeds, retraction, etc.

    // 4. Set display fields
    profile.set_string_array("filament_settings_id",
        vec![profile_name.clone(), profile_name.clone()]);
    profile.set_string_array("filament_type",
        vec![specs.material.clone(), specs.material.clone()]);
    profile.set_string_array("filament_vendor",
        vec![specs.brand.clone(), specs.brand.clone()]);

    // 5. Generate metadata
    let metadata = ProfileMetadata {
        sync_info: String::new(),
        user_id: String::new(),  // Will be filled from BambuPaths.preset_folder
        setting_id: generate_setting_id(),
        base_id: String::new(),
        updated_time: Utc::now().timestamp() as u64,
    };

    Ok((profile, metadata))
}
```

### Pattern 2: Installation with BS Detection
**What:** Before writing files, check if Bambu Studio is running. If running, warn the user and require confirmation.
**When to use:** Every profile installation.
**Why:** BS may have profiles cached in memory; writing files while BS is running can lead to overwrite on next sync or BS not seeing the changes until restart.

```rust
// Source: macOS pgrep approach (zero external dependencies)
use std::process::Command;

/// Check if Bambu Studio is currently running.
/// Returns true if the process is detected.
#[cfg(target_os = "macos")]
pub fn is_bambu_studio_running() -> bool {
    Command::new("pgrep")
        .arg("-f")
        .arg("BambuStudio")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
pub fn is_bambu_studio_running() -> bool {
    // tasklist /FI "IMAGENAME eq BambuStudio.exe" approach
    false // Stub for now
}

#[cfg(target_os = "linux")]
pub fn is_bambu_studio_running() -> bool {
    Command::new("pgrep")
        .arg("-f")
        .arg("BambuStudio")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
```

### Pattern 3: Tauri Command for End-to-End Flow
**What:** A single Tauri command that searches, generates, and optionally installs in one flow.
**When to use:** When the UI sends a "search and generate" request.
**Why:** Reduces round-trips between frontend and backend; the backend already has all the context.

```rust
#[derive(Serialize)]
pub struct GenerateResult {
    pub profile_name: String,
    pub profile_json_preview: String,  // First N lines for UI preview
    pub field_count: usize,
    pub base_profile_used: String,
    pub specs_used: FilamentSpecs,
    pub warnings: Vec<String>,
    pub bambu_studio_running: bool,
}

#[tauri::command]
pub async fn generate_profile_from_specs(
    app: tauri::AppHandle,
    specs: FilamentSpecs,
    target_printer: Option<String>,
) -> Result<GenerateResult, String> {
    // 1. Detect paths and build registry
    // 2. Generate profile
    // 3. Check if BS is running
    // 4. Return preview for user confirmation
}

#[tauri::command]
pub async fn install_generated_profile(
    app: tauri::AppHandle,
    profile_json: String,     // Full JSON from generate step
    metadata_info: String,    // .info content from generate step
    force: bool,              // Override BS-running warning
) -> Result<String, String> {
    // 1. Re-check BS running status
    // 2. Write profile + metadata atomically
    // 3. Return installed path
}
```

### Pattern 4: Leptos Search UI with Debounced Input
**What:** A search page with text input, debounced search trigger, results display, and generate/install buttons.
**When to use:** The primary user-facing feature of Phase 4.

```rust
// Source: Leptos 0.8 forms and signals pattern (book.leptos.dev)
use leptos::prelude::*;

#[component]
pub fn FilamentSearchPage() -> impl IntoView {
    let (search_query, set_search_query) = signal(String::new());
    let (is_searching, set_is_searching) = signal(false);

    // Resource that triggers search when query changes (with debounce via Effect)
    let search_results = Resource::new(
        move || search_query.get(),
        |query| async move {
            if query.len() < 3 { return Err("Enter at least 3 characters".to_string()); }
            // invoke search_filament Tauri command
            search_filament_command(&query).await
        }
    );

    view! {
        <div class="page filament-search-page">
            <h2>"Search Filament"</h2>
            <input
                type="text"
                placeholder="e.g., Polymaker PLA Pro"
                prop:value=move || search_query.get()
                on:input=move |ev| set_search_query.set(event_target_value(&ev))
            />
            // Results display with Suspense
            <Suspense fallback=|| view! { <p>"Searching..."</p> }>
                {move || search_results.get().map(|result| match result {
                    Ok(specs) => view! { <FilamentCard specs=specs /> }.into_any(),
                    Err(e) => view! { <p class="error">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}
```

### Anti-Patterns to Avoid

- **Generating profiles with `inherits` set to a base name:** User profiles MUST be fully flattened (`inherits: ""`). If `inherits` is non-empty, Bambu Studio treats it as a system profile derivative and may behave unpredictably.

- **Using system profile ID formats for user profiles:** System profiles use `GFL` + number for `filament_id` and `GFS` + number for `setting_id`. User profiles use `P` + hex and `PFUS` + hex respectively. Using the wrong format may collide with system IDs or confuse BS.

- **Writing profiles to the `default/` user directory:** The `default/` folder is unused. Profiles must go to `user/{preset_folder}/filament/base/`. Always use `BambuPaths.user_filament_dir()` to get the correct path.

- **Forgetting to duplicate values for dual-extruder arrays:** All array fields in user profiles have 2 elements (for dual-extruder compatibility). Generating arrays with 1 element will produce profiles that may show as corrupted or incomplete.

- **Writing profile files while BS is running without warning:** Even though the atomic write ensures no partial files, BS may have cached the old state and overwrite on next sync.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Base profile resolution | Manual field copying from Generic PLA JSON | `resolve_inheritance()` from inheritance.rs | Handles 3-4 level chains, nil skipping, metadata exclusion correctly |
| Atomic file writes | Manual temp-file-rename pattern | `write_profile_with_metadata()` from writer.rs | Already proven, handles .json + .info atomically |
| Material type mapping | String matching with if/else | `MaterialType::from_str()` from scraper/types.rs | Priority-ordered, handles variants (PLA+, PA-CF, etc.) |
| Path detection | Hardcoded OS paths | `BambuPaths::detect()` + `user_filament_dir()` | Reads preset_folder from BambuStudio.conf, falls back to directory scan |
| Profile name formatting | Custom string formatting | Follow existing naming convention: `"{Brand} {Material} {Name} @{Printer}"` | Matches how BS names profiles for UI display |
| JSON 4-space formatting | Custom indentation | `FilamentProfile::to_json_4space()` | Already correct, matches BS format exactly |

**Key insight:** Phase 2 and Phase 3 built almost all the infrastructure Phase 4 needs. The generator is primarily a mapping layer between `FilamentSpecs` and `FilamentProfile`, using existing read/write/resolve infrastructure. The new code is the mapping logic, ID generation, process detection, and UI.

## Common Pitfalls

### Pitfall 1: Array Fields Must Be Dual-Element
**What goes wrong:** Generated profile has single-element arrays for temperature, speed, etc. Profile shows "corrupted" or "unsupported" in BS.
**Why it happens:** The base profile after inheritance resolution has 2-element arrays (for H2C dual-extruder). If the generator sets `nozzle_temperature: ["210"]` instead of `["210", "210"]`, it breaks the format.
**How to avoid:** Always generate 2-element arrays for all array fields. The resolved base profile will show the correct element count; when overriding, match the element count from the base.
**Warning signs:** Profile appears in BS but shows incorrect values or "unsupported filament" warning.

### Pitfall 2: filament_id/setting_id Collisions
**What goes wrong:** Two generated profiles have the same `filament_id` or `setting_id`. BS confuses them or one overwrites the other.
**Why it happens:** Using predictable ID generation (hash of name, sequential numbers) instead of random hex.
**How to avoid:** Use `rand::random::<[u8; N]>()` for truly random IDs. The ID space is large enough (7 hex chars = 268M combinations) that collisions are negligible.
**Warning signs:** Installing a new profile causes an existing profile to disappear or change.

### Pitfall 3: Cloud Sync Overwrites Locally Written Profiles
**What goes wrong:** Profile is installed, BS syncs with cloud, and the profile is either overwritten or removed.
**Why it happens:** The cloud sync mechanism downloads profiles and can overwrite local ones. The primary trigger is corrupted `"nil"` values in array fields, but any mismatch between cloud and local state can cause issues.
**How to avoid:** (1) Generate clean profiles without inconsistent `"nil"` values -- either use `"nil"` in all positions or real values in all positions, never mix. (2) Set `updated_time` to current timestamp in `.info` file. (3) Leave `sync_info` empty. (4) Warn user that BS restart is required and that cloud sync may interact with the profile.
**Warning signs:** Profile disappears or reverts after BS restart with cloud sync enabled.

### Pitfall 4: Base Profile Not Found
**What goes wrong:** `ProfileRegistry.get_by_name("Generic PLA")` returns None because the system profile name does not match exactly.
**Why it happens:** System profiles have names like `"Generic PLA"`, `"Generic PLA @base"`, `"Generic PLA @BBL H2C 0.4 nozzle"`. The exact name depends on which level in the hierarchy you want.
**How to avoid:** Load the system profile registry from `BambuPaths.system_filament_dir()`, then look up the correct base. The `@base` variants are intermediate (not directly usable); the named variants like `"Generic PLA"` (without `@`) are the printer-specific instantiated profiles. For generation, use the `@base` variant and resolve its inheritance, OR use a fully instantiated variant matching the target printer.
**Warning signs:** "Base profile not found" error; user can't generate any profiles.

### Pitfall 5: Bambu Studio Not Installed
**What goes wrong:** Generation fails because system profiles can't be loaded; installation fails because target directory doesn't exist.
**Why it happens:** User runs BambuMate before installing BS, or BS was uninstalled.
**How to avoid:** The existing `BambuPaths::detect()` returns an error and `list_profiles` returns empty vec. Follow the same pattern: if BS is not detected, return a clear error message guiding the user to install BS first. Do not crash.
**Warning signs:** Panic or unclear error on fresh system without BS.

### Pitfall 6: Missing filament_vendor and filament_type Array Fields
**What goes wrong:** Generated profile shows in BS but filament type shows as "unknown" or vendor is blank.
**Why it happens:** The `filament_vendor` and `filament_type` fields are array fields that must be present in user profiles. If the generator only sets `filament_type` as a bare string (not array), BS can't read it.
**How to avoid:** Always set these as 2-element arrays: `"filament_vendor": ["Polymaker", "Polymaker"]`, `"filament_type": ["PLA", "PLA"]`.
**Warning signs:** Profile appears in BS with blank vendor or type columns.

## Code Examples

### Field Mapping: FilamentSpecs to Profile Overrides

```rust
// Source: Derived from sample_profile.json fixture and FilamentSpecs struct
// This maps each scraped spec field to the corresponding BS profile field(s).

fn apply_specs_to_profile(profile: &mut FilamentProfile, specs: &FilamentSpecs) {
    // Helper: set a 2-element array from a value
    let set_dual = |p: &mut FilamentProfile, key: &str, val: String| {
        p.set_string_array(key, vec![val.clone(), val]);
    };

    // Nozzle temperature: use max as primary, min for range_low
    if let Some(temp_max) = specs.nozzle_temp_max {
        set_dual(profile, "nozzle_temperature", temp_max.to_string());
        // Initial layer: +5C is common convention
        set_dual(profile, "nozzle_temperature_initial_layer",
            (temp_max + 5).to_string());
    }
    if let Some(temp_max) = specs.nozzle_temp_max {
        // Range high: max + 10 for safety margin
        set_dual(profile, "nozzle_temperature_range_high",
            (temp_max + 20).to_string());
    }
    if let Some(temp_min) = specs.nozzle_temp_min {
        set_dual(profile, "nozzle_temperature_range_low", temp_min.to_string());
    }

    // Bed temperature: use max as primary
    if let Some(bed_max) = specs.bed_temp_max {
        set_dual(profile, "bed_temperature", bed_max.to_string());
        set_dual(profile, "bed_temperature_initial_layer", bed_max.to_string());
        set_dual(profile, "hot_plate_temp", bed_max.to_string());
        set_dual(profile, "hot_plate_temp_initial_layer", bed_max.to_string());
        // Engineering plate and textured plate: same as bed temp for simplicity
        set_dual(profile, "eng_plate_temp", bed_max.to_string());
        set_dual(profile, "eng_plate_temp_initial_layer", bed_max.to_string());
    }
    if let Some(bed_min) = specs.bed_temp_min {
        // Cool plate: use min bed temp
        set_dual(profile, "cool_plate_temp", bed_min.to_string());
        set_dual(profile, "cool_plate_temp_initial_layer", bed_min.to_string());
        // Textured plate: slightly lower
        let textured = if bed_min >= 5 { bed_min - 5 } else { bed_min };
        set_dual(profile, "textured_plate_temp", textured.to_string());
        set_dual(profile, "textured_plate_temp_initial_layer", textured.to_string());
    }

    // Fan speed: convert percentage to Bambu format
    if let Some(fan) = specs.fan_speed_percent {
        set_dual(profile, "fan_max_speed", format!("{}%", fan));
        // Min speed: 60% of max is a reasonable default
        let fan_min = (fan as f32 * 0.6) as u8;
        set_dual(profile, "fan_min_speed", format!("{}%", fan_min));
    }

    // Retraction
    if let Some(dist) = specs.retraction_distance_mm {
        set_dual(profile, "filament_retraction_length", format!("{:.1}", dist));
    }
    if let Some(speed) = specs.retraction_speed_mm_s {
        set_dual(profile, "filament_retraction_speed", speed.to_string());
    }

    // Density
    if let Some(density) = specs.density_g_cm3 {
        set_dual(profile, "filament_density", format!("{:.2}", density));
    }

    // Material identity
    set_dual(profile, "filament_type",
        specs.material.clone());
    set_dual(profile, "filament_vendor",
        specs.brand.clone());
}
```

### ID Generation Matching Bambu Studio Conventions

```rust
// Source: Observed from actual user profiles on disk
// User filament_id pattern: "P" + 7 hex chars (e.g., "Pe7b385c")
// User setting_id pattern: "PFUS" + 14 hex chars (e.g., "PFUS50d8c9d5139548")

fn generate_filament_id() -> String {
    let bytes: [u8; 4] = rand::random();
    format!("P{:07x}", u32::from_be_bytes(bytes) & 0x0FFFFFFF)
}

fn generate_setting_id() -> String {
    let bytes: [u8; 7] = rand::random();
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("PFUS{}", hex)
}

// The user_id in .info comes from BambuPaths.preset_folder
fn generate_metadata(paths: &BambuPaths) -> ProfileMetadata {
    ProfileMetadata {
        sync_info: String::new(),
        user_id: paths.preset_folder.clone().unwrap_or_default(),
        setting_id: generate_setting_id(),
        base_id: String::new(),
        updated_time: chrono::Utc::now().timestamp() as u64,
    }
}
```

### Profile Filename Convention

```rust
// Source: Observed from actual profiles on disk
// Pattern: "{Vendor} {Type} {Name} @{Printer Nozzle}"
// User profiles use full printer name: "Bambu Lab H2C 0.4 nozzle"
// System profiles use abbreviated: "BBL H2C 0.4 nozzle"

fn generate_profile_filename(specs: &FilamentSpecs, printer: &str) -> String {
    format!("{} {} {} @{}.json",
        specs.brand,
        specs.material,
        specs.name,
        printer
    )
}

fn generate_profile_name(specs: &FilamentSpecs, printer: &str) -> String {
    format!("{} {} {} @{}",
        specs.brand,
        specs.material,
        specs.name,
        printer
    )
}
```

### Process Detection (Zero Dependencies)

```rust
// Source: macOS pgrep, verified approach
use std::process::Command;

pub fn is_bambu_studio_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("pgrep")
            .arg("-f")
            .arg("BambuStudio")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "windows")]
    {
        // tasklist /FI "IMAGENAME eq bambu_studio.exe" 2>nul
        Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq bambu_studio.exe", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("bambu_studio"))
            .unwrap_or(false)
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("pgrep")
            .arg("-f")
            .arg("BambuStudio")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}
```

### Leptos Frontend Command Wrappers

```rust
// Source: Matching existing commands.rs pattern
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilamentSpecs {
    pub name: String,
    pub brand: String,
    pub material: String,
    pub nozzle_temp_min: Option<u16>,
    pub nozzle_temp_max: Option<u16>,
    pub bed_temp_min: Option<u16>,
    pub bed_temp_max: Option<u16>,
    pub max_speed_mm_s: Option<u16>,
    pub fan_speed_percent: Option<u8>,
    pub retraction_distance_mm: Option<f32>,
    pub retraction_speed_mm_s: Option<u16>,
    pub density_g_cm3: Option<f32>,
    pub diameter_mm: Option<f32>,
    pub source_url: String,
    pub extraction_confidence: f32,
}

#[derive(Serialize)]
struct SearchFilamentArgs {
    filament_name: String,
}

pub async fn search_filament(name: &str) -> Result<FilamentSpecs, String> {
    let args = serde_wasm_bindgen::to_value(&SearchFilamentArgs {
        filament_name: name.to_string(),
    }).map_err(|e| e.to_string())?;

    let result = invoke("search_filament", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual profile creation in BS UI | Programmatic generation from scraped specs | This phase | Users skip manual settings research entirely |
| Profiles inherit from base (sparse) | User profiles fully flattened | BS standard | Generator must resolve inheritance before writing |
| Single-extruder arrays | Dual-extruder arrays (2 elements) | H2C/H2D release | All array fields must have 2 elements |
| pgrep for process detection | sysinfo crate (alternative) | 2025+ | sysinfo is more portable but heavier; pgrep is sufficient for macOS-first |

**Deprecated/outdated:**
- None specific to this phase. All profile format conventions from Phase 2 research remain current.

## Open Questions

1. **Optimal base profile selection strategy**
   - What we know: System profiles have printer-specific variants (e.g., "Generic PLA @BBL H2C 0.4 nozzle"). Using the `@base` variant requires inheritance resolution; using a specific variant gives a fully-resolved starting point.
   - What's unclear: Whether to let the user choose their printer (and select the matching variant) or default to a generic option and let BS handle printer matching via `compatible_printers`.
   - Recommendation: Start with the `@base` variant + `resolve_inheritance()` for maximum compatibility. Allow the user to optionally select a target printer. Set `compatible_printers` to an empty array to make the profile universal (BS shows it for all printers).

2. **Profile overwrite behavior**
   - What we know: If a profile with the same filename already exists in the user directory, our atomic write will replace it.
   - What's unclear: Whether BS tracks profiles by filename, by `filament_id`, or by `setting_id`. Overwriting a file with a different `filament_id` might confuse BS.
   - Recommendation: Check for existing file before writing. If exists, warn the user and offer to overwrite or create a new filename.

3. **Cloud sync interaction with new profiles**
   - What we know: Cloud sync can overwrite locally-written profiles. Primary trigger is corrupted nil values.
   - What's unclear: Whether a freshly-written profile with clean values and current `updated_time` is safe from sync overwrites.
   - Recommendation: Warn users that cloud sync may interact with generated profiles. Generate clean profiles (no mixed nil/value arrays). Set `updated_time` to current timestamp. Document in UI that BS restart is required.

4. **compatible_printers field for generated profiles**
   - What we know: System profiles list specific printers. User profiles observed on disk also have specific printers listed.
   - What's unclear: What happens if `compatible_printers` is empty -- does BS show the filament for all printers, or hide it?
   - Recommendation: Default to empty array `[]` which in testing appears to make the profile universal. If the user selected a target printer, populate with that printer's string.

5. **hex encoding for setting_id**
   - What we know: `rand` crate generates random bytes. We need to encode them as hex.
   - What's unclear: Whether to add a hex crate or use format macros.
   - Recommendation: Use `format!("{:02x}", byte)` for each byte -- no additional dependency needed. The `hex` crate would be cleaner but is unnecessary for 7 bytes.

## Sources

### Primary (HIGH confidence)
- Existing codebase: `src-tauri/src/profile/` (types.rs, paths.rs, writer.rs, inheritance.rs, registry.rs) -- all verified by reading source
- Existing codebase: `src-tauri/src/scraper/types.rs` -- FilamentSpecs struct definition
- Existing codebase: `src-tauri/tests/fixtures/sample_profile.json` -- real profile format with all field patterns
- Existing codebase: `src-tauri/tests/fixtures/sample_profile.info` -- real metadata format
- Phase 2 Research: `.planning/phases/02-profile-engine/02-RESEARCH.md` -- profile format, inheritance, ID patterns (HIGH confidence, verified from disk)
- sysinfo 0.38 docs: https://docs.rs/sysinfo/latest/sysinfo/struct.System.html -- `processes_by_name()` and `processes_by_exact_name()` method signatures

### Secondary (MEDIUM confidence)
- Bambu Lab community forum: https://forum.bambulab.com/t/cloud-sync-overwriting-local-profiles-root-cause-and-fix/207065 -- cloud sync overwrite root cause analysis
- Bambu Lab community forum: https://forum.bambulab.com/t/how-is-bambu-studio-setup-to-handle-filaments/171590 -- profile types and storage
- Bambu Lab Wiki: https://wiki.bambulab.com/en/bambu-studio/submit-preset -- filament_id starts with "GF" (for system profiles)
- Bambu Lab Wiki: https://wiki.bambulab.com/en/bambu-studio/create-filament -- custom filament creation process

### Tertiary (LOW confidence)
- `compatible_printers: []` universality behavior -- inferred, not confirmed via BS source code
- Cloud sync safety with current `updated_time` -- community reports suggest it helps, not officially documented
- Exact process name for BS on macOS ("BambuStudio" vs "bambu_studio") -- needs validation on actual running BS instance

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries already in project except `rand`, which is well-established
- Architecture: HIGH -- generator pattern derived directly from existing codebase infrastructure (inheritance, writer, registry)
- Profile format: HIGH -- verified from sample_profile.json fixture and Phase 2 research
- Field mapping (specs -> profile): MEDIUM -- logical mapping but some fields (cool_plate_temp, textured_plate_temp) are best guesses for material-specific values
- Process detection: MEDIUM -- pgrep approach is standard but exact BS process name needs runtime validation
- Cloud sync behavior: LOW -- community reports only, no official documentation
- Pitfalls: HIGH -- derived from Phase 2 research and codebase analysis

**Research date:** 2026-02-05
**Valid until:** 2026-03-07 (30 days; profile format is stable unless BS updates)
