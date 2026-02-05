# Architecture Research

**Domain:** Rust CLI tool -- 3D print profile management, AI analysis, web scraping
**Researched:** 2026-02-04
**Confidence:** HIGH (verified against local Bambu Studio installation, BambuStudio GitHub wiki, Rust ecosystem docs)

## System Overview

```
                            ┌─────────────────────────┐
                            │     CLI Entry Point      │
                            │   (clap subcommands)     │
                            │                          │
                            │  lookup | analyze |      │
                            │  generate | apply |      │
                            │  launch                  │
                            └────────────┬────────────┘
                                         │
                    ┌────────────────────┬┴──────────────────────┐
                    │                    │                       │
           ┌────────▼────────┐  ┌────────▼────────┐  ┌──────────▼────────┐
           │   Web Scraper   │  │    AI Client     │  │  Profile Manager  │
           │                 │  │                  │  │                   │
           │ reqwest+scraper │  │ Claude/OpenAI    │  │ Read/Write/       │
           │ Manufacturer    │  │ Vision analysis  │  │ Validate JSON     │
           │ spec extraction │  │ of print photos  │  │ OS-aware paths    │
           └────────┬────────┘  └────────┬────────┘  └────────┬──────────┘
                    │                    │                     │
                    └────────────┬───────┘                     │
                                 │                             │
                    ┌────────────▼────────────┐                │
                    │    Defect Mapper        │                │
                    │                         │                │
                    │  AI defects → setting   │◄───────────────┘
                    │  parameter adjustments  │
                    │  (rule engine)          │
                    └────────────┬────────────┘
                                 │
              ┌──────────────────┼──────────────────┐
              │                  │                   │
     ┌────────▼────────┐  ┌─────▼──────┐  ┌────────▼────────┐
     │  Profile Writer  │  │  Bambu     │  │  OpenSCAD       │
     │                  │  │  Studio    │  │  Studio         │
     │  Generate JSON   │  │  Launcher  │  │  Integration    │
     │  with inherits   │  │            │  │                 │
     │  & overrides     │  │  open CLI  │  │  Receive STLs   │
     └─────────┬────────┘  │  with args │  │  via file path  │
               │           └────────────┘  └─────────────────┘
               ▼
     ┌─────────────────┐
     │  File System     │
     │                  │
     │  ~/Library/      │
     │  Application     │
     │  Support/        │
     │  BambuStudio/    │
     │  user/           │
     └─────────────────┘
```

## Component Responsibilities

| Component | Responsibility | Communicates With | Key Crate(s) |
|-----------|----------------|-------------------|--------------|
| **CLI Entry** | Parse subcommands, route to handlers, manage config | All components | `clap` (derive) |
| **Web Scraper** | Fetch filament specs from manufacturer websites | Profile Manager (provides scraped data), AI Client (for unstructured extraction) | `reqwest`, `scraper`, `tokio` |
| **AI Client** | Send images/text to Claude/OpenAI APIs, parse structured responses | Defect Mapper (provides defect list) | `reqwest`, `serde_json`, `base64` |
| **Profile Manager** | Read/write/validate Bambu Studio JSON profiles, resolve inheritance chains | File system, Defect Mapper | `serde`, `serde_json`, `dirs` |
| **Defect Mapper** | Map AI-detected defects to specific Bambu Studio setting changes | Profile Manager (reads current values, writes adjustments) | Pure Rust (rule engine, no external deps) |
| **Bambu Studio Launcher** | Launch Bambu Studio with correct CLI args | OS process spawning | `std::process::Command` |
| **OpenSCAD Studio Integration** | Accept STL file paths, pass through pipeline | Profile Manager, Launcher | File path handling only |

## Recommended Project Structure

```
bambumate/
├── Cargo.toml
├── src/
│   ├── main.rs                 # Entry point, clap setup
│   ├── cli.rs                  # Clap derive structs, subcommand routing
│   ├── config.rs               # App config (~/.bambumate/config.toml)
│   ├── error.rs                # Unified error types (thiserror)
│   │
│   ├── scraper/
│   │   ├── mod.rs              # Scraper trait + factory
│   │   ├── client.rs           # HTTP client wrapper (reqwest)
│   │   ├── extractors.rs       # HTML parsing (scraper crate)
│   │   └── manufacturers.rs    # Manufacturer-specific extraction rules
│   │
│   ├── ai/
│   │   ├── mod.rs              # AI client trait
│   │   ├── claude.rs           # Claude API implementation
│   │   ├── openai.rs           # OpenAI API implementation
│   │   ├── prompt.rs           # System prompts for analysis
│   │   └── types.rs            # DefectReport, AnalysisResult
│   │
│   ├── profile/
│   │   ├── mod.rs              # Profile manager public API
│   │   ├── schema.rs           # Bambu Studio JSON types (serde)
│   │   ├── paths.rs            # OS-specific config path resolution
│   │   ├── inheritance.rs      # Resolve inherits chains
│   │   ├── reader.rs           # Read + resolve profiles from disk
│   │   └── writer.rs           # Generate + write profile JSON
│   │
│   ├── mapper/
│   │   ├── mod.rs              # Defect-to-setting mapping engine
│   │   ├── rules.rs            # Mapping rules (defect → params)
│   │   └── adjustments.rs      # Parameter adjustment logic
│   │
│   └── launcher/
│       ├── mod.rs              # Bambu Studio launcher
│       └── detect.rs           # Find Bambu Studio installation
│
├── tests/
│   ├── fixtures/               # Sample profile JSONs for testing
│   │   ├── fdm_filament_common.json
│   │   ├── fdm_filament_pla.json
│   │   └── generic_pla.json
│   └── integration/
│       ├── profile_test.rs
│       └── scraper_test.rs
│
└── config/
    └── defect_rules.toml       # Defect-to-setting mapping rules (data, not code)
```

### Structure Rationale

- **`scraper/`**: Isolated because it owns all HTTP I/O and HTML parsing. Different manufacturer sites need different extraction strategies, so this module is designed for extensibility.
- **`ai/`**: Trait-based to swap AI providers. Claude and OpenAI have different APIs but the same output contract (defect analysis). This is the most likely module to gain new backends.
- **`profile/`**: The core domain module. It must understand Bambu Studio's inheritance system (base profile -> material type -> specific filament -> nozzle variant). This complexity deserves its own module with clear subcomponents.
- **`mapper/`**: Pure business logic with no I/O. Takes defect reports in, produces setting adjustment recommendations out. Rules are data-driven (loaded from TOML), not hardcoded.
- **`launcher/`**: Thin wrapper around `std::process::Command`. OS-specific detection of Bambu Studio installation path.

## Data Flow

### Flow 1: `bambumate lookup <filament>`

```
User runs: bambumate lookup "Polymaker PLA Pro"
    │
    ▼
CLI parses args (clap)
    │
    ▼
Scraper searches manufacturer website
    │ reqwest GET → parse HTML with scraper crate
    │ OR: AI-assisted extraction for unstructured pages
    ▼
Returns structured filament specs:
    {
      nozzle_temp: 190-230,
      bed_temp: 25-60,
      print_speed: "recommended 60mm/s",
      density: 1.24,
      drying: "55C for 4h"
    }
    │
    ▼
Display to user (formatted table)
```

### Flow 2: `bambumate analyze <image>`

```
User runs: bambumate analyze photo.jpg --filament "PLA"
    │
    ▼
CLI validates image path exists
    │
    ▼
AI Client encodes image (base64)
    │
    ▼
AI Client sends to Claude/OpenAI with structured prompt:
    "Analyze this 3D print. Identify defects from this list:
     [stringing, layer_adhesion, warping, z_banding, ...]
     Return JSON with severity scores."
    │
    ▼
AI returns DefectReport:
    {
      defects: [
        { type: "stringing", severity: 0.7, confidence: 0.89 },
        { type: "elephant_foot", severity: 0.3, confidence: 0.75 }
      ]
    }
    │
    ▼
Defect Mapper applies rules:
    stringing (0.7) → retraction_length +0.4mm, nozzle_temp -5C
    elephant_foot (0.3) → bed_temp -5C, initial_layer_speed -10mm/s
    │
    ▼
Display recommendations to user
    (optionally: feed into "generate" flow)
```

### Flow 3: `bambumate generate <filament> --printer <model>`

```
User runs: bambumate generate "Polymaker PLA Pro" --printer "P1S" --nozzle 0.4
    │
    ▼
Scraper fetches filament specs (or uses cache)
    │
    ▼
Profile Manager finds base profile to inherit from:
    1. Scan system/BBL/filament/ for "Generic PLA" matching printer
    2. Resolve inheritance: Generic PLA @BBL P1S → fdm_filament_pla → fdm_filament_common
    │
    ▼
Profile Manager creates new JSON:
    {
      "type": "filament",
      "filament_id": "GFU99_POLYMAKER_001",
      "name": "Polymaker PLA Pro @BBL P1S 0.4 nozzle",
      "inherits": "Generic PLA @BBL P1S",
      "instantiation": "true",
      "nozzle_temperature": ["215"],
      "nozzle_temperature_initial_layer": ["220"],
      "cool_plate_temp": ["60"],
      "filament_density": ["1.24"],
      "filament_vendor": ["Polymaker"],
      "compatible_printers": ["Bambu Lab P1S 0.4 nozzle"]
    }
    │
    ▼
Profile Writer validates JSON against known schema
    │
    ▼
Write to stdout (default) or --output file
```

### Flow 4: `bambumate apply <profile.json>`

```
User runs: bambumate apply polymaker_pla_pro.json
    │
    ▼
Profile Manager validates JSON structure
    │
    ▼
Profile Manager resolves target path:
    macOS:  ~/Library/Application Support/BambuStudio/user/<device_id>/filament/
    Win:    %APPDATA%/BambuStudio/user/<device_id>/filament/
    Linux:  ~/.config/BambuStudio/user/<device_id>/filament/
    │
    ▼
Copy file to target directory
    │
    ▼
Display: "Profile installed. Restart Bambu Studio to see it."
```

### Flow 5: `bambumate launch [file.stl] [--profile <name>]`

```
User runs: bambumate launch model.stl --profile "Polymaker PLA Pro"
    │
    ▼
Launcher detects Bambu Studio installation:
    macOS:  /Applications/BambuStudio.app/Contents/MacOS/BambuStudio
    Win:    C:\Program Files\BambuStudio\bambu-studio.exe
    Linux:  /usr/bin/bambu-studio (or AppImage path)
    │
    ▼
Profile Manager locates profile JSON on disk
    │
    ▼
Launcher builds command:
    bambu-studio --load-filaments "path/to/profile.json" model.stl
    │
    ▼
std::process::Command::new(bambu_path)
    .args([...])
    .spawn()
```

## Bambu Studio Profile Schema (Verified from Local Installation)

**Confidence: HIGH** -- Verified against actual files at `/Users/michaelcurtis/Library/Application Support/BambuStudio/`

### Profile Inheritance Chain

Bambu Studio uses a 3-level inheritance hierarchy for filament profiles:

```
Level 0: fdm_filament_common          (base for ALL filaments)
  │       ~50 fields, instantiation: false
  │
  ├── Level 1: fdm_filament_pla       (material type defaults)
  │     │       inherits: fdm_filament_common
  │     │       Overrides: density, temp ranges, fan speeds
  │     │       instantiation: false
  │     │
  │     ├── Level 2: Generic PLA @BBL P1S    (printer-specific)
  │     │       inherits: fdm_filament_pla (or intermediate)
  │     │       Overrides: compatible_printers, fine-tuned temps
  │     │       instantiation: true  ← VISIBLE in Bambu Studio
  │     │
  │     └── Level 2: Polymaker PLA Pro @BBL P1S  (custom user)
  │             inherits: Generic PLA @BBL P1S
  │             Overrides: vendor, specific temps, flow ratio
  │             instantiation: true
  │
  ├── Level 1: fdm_filament_abs
  ├── Level 1: fdm_filament_pet
  └── Level 1: fdm_filament_tpu
```

### Key JSON Fields (from actual files)

**Identity fields:**
- `type`: Always `"filament"`
- `name`: Display name (e.g., `"Generic PLA @Creality"`)
- `filament_id`: Unique ID (e.g., `"GFL99"`, `"Pe7b385c"`)
- `setting_id`: Settings identifier (e.g., `"GFSL99_CREALITY_00"`)
- `from`: `"system"` or `"User"`
- `instantiation`: `"true"` = visible in UI, `"false"` = template only
- `inherits`: Parent profile name (can be empty string for fully self-contained)

**Critical for generation:**
- `compatible_printers`: Array of exact printer+nozzle strings (e.g., `"Bambu Lab P1S 0.4 nozzle"`)
- `nozzle_temperature`: Array of strings (e.g., `["215"]` or `["220", "220"]` for multi-extruder)
- `nozzle_temperature_initial_layer`: Same format
- `nozzle_temperature_range_low` / `_high`: Validation bounds
- `cool_plate_temp`, `hot_plate_temp`, `eng_plate_temp`, `textured_plate_temp`: Bed temps per plate type
- `filament_flow_ratio`: Flow calibration (e.g., `["0.98"]`)
- `filament_retraction_length`: Retraction distance
- `filament_density`: Material density
- `filament_type`: `"PLA"`, `"ABS"`, `"PETG"`, etc.
- `filament_vendor`: Manufacturer name
- `pressure_advance`: PA value (e.g., `["0.02"]`)

**Important quirk:** Most numeric values are stored as **strings inside single-element arrays** (e.g., `"nozzle_temperature": ["215"]`). Multi-extruder printers use multi-element arrays (e.g., `["220", "220"]`). The value `"nil"` means "inherit from parent."

### OS-Specific Paths (Verified)

| OS | Config Root | User Profiles | System Profiles |
|----|-------------|---------------|-----------------|
| **macOS** | `~/Library/Application Support/BambuStudio/` | `user/<device_id>/filament/` and `user/default/filament/` | `system/BBL/filament/` |
| **Windows** | `%APPDATA%\BambuStudio\` | `user\<device_id>\filament\` | `system\BBL\filament\` |
| **Linux** | `~/.config/BambuStudio/` | `user/<device_id>/filament/` | `system/BBL/filament/` |

The `<device_id>` is a numeric string (e.g., `1881310893`). There can be both a `default/` and a device-specific directory. User-created profiles live under `user/<device_id>/filament/base/`.

### Bambu Studio Application Path

| OS | Path |
|----|------|
| **macOS** | `/Applications/BambuStudio.app` (binary at `Contents/MacOS/BambuStudio`) |
| **Windows** | `C:\Program Files\BambuStudio\bambu-studio.exe` |
| **Linux** | AppImage or `/usr/bin/bambu-studio` |

### Bambu Studio CLI Arguments (Verified from GitHub wiki)

```
bambu-studio [OPTIONS] [file.3mf/file.stl ...]

Key options:
  --load-filaments "f1.json;f2.json;..."    Load filament settings
  --load-settings "machine.json;process.json"  Load machine/process settings
  --slice <plate_index>                      Slice (0=all, N=specific plate)
  --export-3mf <output.3mf>                 Export project
  --export-settings <settings.json>          Export settings to file
  --outputdir <dir>                          Output directory
  --arrange <0|1>                            Arrange model on plate
  --uptodate                                 Update 3mf configs to latest
  --debug <level>                            Log verbosity (0-5)
  --key=value                                Override any setting
```

Settings priority: CLI args > loaded files > values from 3MF file.

## Architectural Patterns

### Pattern 1: Trait-Based Provider Abstraction

**What:** Define traits for components that have multiple implementations (AI providers, manufacturer scrapers).
**When to use:** Any module that needs swappable backends.
**Trade-offs:** Adds a layer of indirection but enables testing with mocks and future extensibility.

```rust
// ai/mod.rs
#[async_trait]
pub trait AiAnalyzer: Send + Sync {
    async fn analyze_print(&self, image: &[u8], context: &AnalysisContext)
        -> Result<DefectReport>;
}

// ai/claude.rs
pub struct ClaudeAnalyzer { client: reqwest::Client, api_key: String }

impl AiAnalyzer for ClaudeAnalyzer {
    async fn analyze_print(&self, image: &[u8], context: &AnalysisContext)
        -> Result<DefectReport> {
        // Base64 encode image, send to Claude API with structured prompt
        // Parse structured JSON response into DefectReport
    }
}
```

### Pattern 2: Flatten + HashMap for Forward-Compatible Profiles

**What:** Use `serde(flatten)` with `HashMap<String, Value>` to preserve unknown JSON fields when reading/writing profiles.
**When to use:** Profile schema handling, because Bambu Studio adds new fields in updates and we must not discard them.
**Trade-offs:** Slightly more complex deserialization, but prevents data loss when round-tripping profiles.

```rust
// profile/schema.rs
#[derive(Serialize, Deserialize)]
pub struct FilamentProfile {
    #[serde(rename = "type")]
    pub profile_type: String,
    pub name: String,
    pub inherits: Option<String>,
    pub instantiation: Option<String>,
    pub filament_id: Option<String>,
    pub setting_id: Option<String>,
    pub compatible_printers: Option<Vec<String>>,
    pub nozzle_temperature: Option<Vec<String>>,
    pub filament_flow_ratio: Option<Vec<String>>,
    // ... known fields we actively use ...

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,  // Preserve unknown fields
}
```

### Pattern 3: Data-Driven Rule Engine

**What:** Defect-to-setting mappings stored as configuration data (TOML/JSON), not hardcoded in Rust.
**When to use:** The mapper module. Rules change as we learn from user feedback. Data files are easier to iterate on than recompiling.
**Trade-offs:** Slight runtime overhead for rule loading, but dramatically easier to update without code changes.

```toml
# config/defect_rules.toml
[[rules]]
defect = "stringing"
severity_threshold = 0.3

[[rules.adjustments]]
setting = "filament_retraction_length"
operation = "increase"
amount = "0.4"

[[rules.adjustments]]
setting = "nozzle_temperature"
operation = "decrease"
amount = "5"

[[rules]]
defect = "elephant_foot"
severity_threshold = 0.3

[[rules.adjustments]]
setting = "cool_plate_temp"
operation = "decrease"
amount = "5"
```

### Pattern 4: OS-Aware Path Resolution

**What:** Centralize all OS-specific path logic in a single module that returns paths based on the current platform.
**When to use:** Every interaction with Bambu Studio's file system (reading system profiles, writing user profiles, finding the executable).

```rust
// profile/paths.rs
pub struct BambuPaths {
    pub config_root: PathBuf,     // ~/Library/Application Support/BambuStudio/
    pub system_profiles: PathBuf, // .../system/BBL/filament/
    pub user_profiles: PathBuf,   // .../user/<device_id>/filament/
    pub bambu_executable: PathBuf,
}

impl BambuPaths {
    pub fn detect() -> Result<Self> {
        #[cfg(target_os = "macos")]
        { /* ~/Library/Application Support/BambuStudio/ */ }

        #[cfg(target_os = "windows")]
        { /* %APPDATA%/BambuStudio/ */ }

        #[cfg(target_os = "linux")]
        { /* ~/.config/BambuStudio/ */ }
    }
}
```

## Anti-Patterns

### Anti-Pattern 1: Typed Structs for Full Profile Schema

**What people do:** Define a Rust struct with all 100+ Bambu Studio profile fields as typed fields.
**Why it's wrong:** The schema evolves with every Bambu Studio update. New fields get added, old ones get deprecated. A rigid struct will silently discard unknown fields, corrupting profiles on round-trip read/write.
**Do this instead:** Use `serde(flatten)` with `HashMap<String, Value>` for unknown fields. Only type the fields you actively manipulate. Round-trip everything else unmodified.

### Anti-Pattern 2: Blocking HTTP in CLI

**What people do:** Use `reqwest::blocking` for simplicity, then hit issues when adding concurrent scraping or AI requests.
**Why it's wrong:** The scraper and AI client both do HTTP I/O. Sequential execution for multiple lookups or parallel analysis wastes time.
**Do this instead:** Use `tokio` runtime from the start. The CLI entry point wraps `#[tokio::main]`, and all I/O modules use async. This pays off immediately when scraping multiple manufacturer pages or when a user wants to analyze + lookup + generate in one pipeline.

### Anti-Pattern 3: Hardcoding Defect-to-Setting Mappings

**What people do:** Write `match defect { "stringing" => { retraction += 0.4; temp -= 5; } }` directly in code.
**Why it's wrong:** Mapping rules are domain knowledge that changes frequently based on user feedback, new filament types, and community input. Code changes require recompilation and a new release.
**Do this instead:** Store rules in a TOML/JSON config file. Load at runtime. Users can even customize their own rule overrides.

### Anti-Pattern 4: Monolithic Error Types

**What people do:** Create one giant `BambuMateError` enum covering every possible error from every module.
**Why it's wrong:** Couples all modules together through the error type. Changes to the scraper module affect the mapper module's error handling.
**Do this instead:** Each module has its own error type (via `thiserror`). The CLI layer uses `anyhow::Result` for the top-level, converting module errors with context. Module boundaries stay clean.

## Integration Points

### External Services

| Service | Integration Pattern | Notes |
|---------|---------------------|-------|
| **Claude API** | HTTPS POST with API key, structured JSON response | Messages API with `image` content type for vision. Use `response_format` to enforce structured output. |
| **OpenAI API** | HTTPS POST with API key, structured JSON response | Chat completions with `image_url` content type. Use function calling or JSON mode for structured output. |
| **Manufacturer Websites** | HTTP GET + HTML parsing | Different per manufacturer. Start with top 10 brands. Respect `robots.txt`. Cache aggressively. |
| **Bambu Studio (filesystem)** | Direct JSON file read/write | System profiles are read-only. User profiles are read/write. Must handle device_id discovery. |
| **Bambu Studio (process)** | `std::process::Command::spawn()` | Launch with `--load-filaments`, `--load-settings`, and STL file arguments. |

### Internal Boundaries

| Boundary | Communication | Notes |
|----------|---------------|-------|
| CLI -> Scraper | Function call (async) | CLI passes filament name/brand, gets back `FilamentSpecs` struct |
| CLI -> AI Client | Function call (async) | CLI passes image bytes + context, gets back `DefectReport` struct |
| AI Client -> Defect Mapper | Struct passing | `DefectReport` in, `Vec<SettingAdjustment>` out. No I/O. |
| Defect Mapper -> Profile Manager | Struct passing | Adjustments applied to profile in memory |
| Profile Manager -> File System | `std::fs` read/write | JSON serialization/deserialization through serde |
| CLI -> Launcher | Function call | Builds Command, spawns process |

### OpenSCAD Studio Integration

OpenSCAD Studio is a Tauri 2.0 app at `/Users/michaelcurtis/Development/openscad-studio`. It exports STL files via its menu (Export as STL, 3MF, etc.). The integration pattern is file-based:

1. **OpenSCAD Studio exports an STL** to a file path
2. **User invokes BambuMate** with that file path: `bambumate launch /path/to/model.stl --profile "My PLA"`
3. **BambuMate** locates the profile, builds CLI args, and launches Bambu Studio

Future deeper integration could use:
- A shared directory convention (e.g., `~/.bambumate/inbox/`) where OpenSCAD Studio writes exports
- A file watcher in BambuMate that auto-detects new STLs
- A Tauri command in OpenSCAD Studio that invokes `bambumate` as a subprocess

For MVP, the file path handoff is sufficient. No IPC protocol needed.

## Build Order & Dependency Graph

```
Phase 1: Foundation
  ┌─────────────┐     ┌─────────────────┐
  │ CLI skeleton │     │ Profile Manager  │
  │ (clap)      │────▶│ (paths, schema,  │
  │             │     │  reader, writer) │
  └─────────────┘     └─────────────────┘
                              │
Phase 2: Intelligence         │ depends on profile schema
  ┌─────────────┐     ┌──────▼──────────┐
  │ Web Scraper │     │ Defect Mapper   │
  │ (reqwest +  │     │ (rule engine,   │
  │  scraper)   │     │  adjustments)   │
  └──────┬──────┘     └──────┬──────────┘
         │                   │
Phase 3: AI + Integration    │ depends on mapper + scraper
  ┌──────▼──────┐     ┌──────▼──────────┐
  │  AI Client  │     │   Generate cmd  │
  │ (Claude +   │────▶│   (full flow:   │
  │  OpenAI)    │     │   scrape+map+   │
  └─────────────┘     │   write)        │
                      └─────────────────┘
                              │
Phase 4: Polish               │
  ┌─────────────┐     ┌──────▼──────────┐
  │  Launcher   │     │  Apply cmd      │
  │ (detect +   │     │  (install to    │
  │  spawn)     │     │  BambuStudio)   │
  └─────────────┘     └─────────────────┘
```

### Suggested Build Order (with rationale)

**Phase 1: CLI + Profile Manager** (build first, everything depends on this)
1. `cli.rs` -- Clap derive structs with all subcommands (skeleton only, handlers return `todo!()`)
2. `config.rs` -- App configuration loading (`~/.bambumate/config.toml` for API keys, preferences)
3. `error.rs` -- Error types per module
4. `profile/paths.rs` -- OS-specific Bambu Studio path resolution (testable immediately against local install)
5. `profile/schema.rs` -- Serde types for filament profiles (with flatten for unknown fields)
6. `profile/reader.rs` -- Read and parse profiles from disk
7. `profile/inheritance.rs` -- Resolve inheritance chains (load parent, merge overrides)
8. `profile/writer.rs` -- Serialize and write profile JSON

**Why first:** The profile manager is the core domain model. Every other component either reads from it or writes to it. Getting the JSON schema handling right (especially the inheritance resolution and the preserve-unknown-fields pattern) is foundational. This is also immediately testable against the actual Bambu Studio installation on disk.

**Phase 2: Web Scraper + Defect Mapper** (independent, can be built in parallel)
1. `scraper/client.rs` -- Async HTTP client wrapper
2. `scraper/extractors.rs` -- HTML parsing for filament specs
3. `scraper/manufacturers.rs` -- Per-manufacturer extraction rules (start with 5 major brands)
4. `mapper/rules.rs` -- Load defect-to-setting rules from TOML
5. `mapper/adjustments.rs` -- Apply adjustments to profile values

**Why second:** The scraper and mapper are independent of each other but both need the profile schema types from Phase 1. The mapper is pure logic (no I/O) and can be thoroughly unit tested. The scraper requires network mocking for tests.

**Phase 3: AI Client + Generate Flow** (requires Phase 1 + 2)
1. `ai/claude.rs` -- Claude API client (vision analysis)
2. `ai/openai.rs` -- OpenAI API client (vision analysis)
3. `ai/prompt.rs` -- Structured prompts for consistent defect detection
4. Wire up `generate` command: scraper output + mapper rules -> profile writer
5. Wire up `analyze` command: AI client -> defect report -> mapper -> recommendations

**Why third:** AI integration depends on having the defect mapper (to make results actionable) and the profile manager (to generate profiles from analysis). The structured prompts need to align with the defect types defined in the mapper rules.

**Phase 4: Launcher + Apply + Polish**
1. `launcher/detect.rs` -- Find Bambu Studio on current OS
2. `launcher/mod.rs` -- Build CLI args and spawn process
3. Wire up `apply` command: validate profile, copy to correct directory
4. Wire up `launch` command: detect BambuStudio, build args, spawn
5. OpenSCAD Studio integration (accept STL paths in `launch` command)

**Why last:** Launcher and apply are thin wrappers. They depend on the profile manager being stable but add minimal logic. The OpenSCAD Studio integration is file-path-based and needs no special protocol.

## Scaling Considerations

This is a CLI tool, not a web service. "Scaling" means handling large profile collections and many manufacturers gracefully.

| Concern | Now (v1) | Future (v2+) |
|---------|----------|--------------|
| **Manufacturer coverage** | 5-10 hardcoded extractors | Plugin system or AI-assisted extraction for arbitrary pages |
| **Profile count** | Direct file system reads | SQLite cache of profile metadata for faster searches |
| **AI cost** | Per-invocation API calls | Local model option (llama.cpp) for offline analysis |
| **Cross-platform** | macOS only | `cfg(target_os)` gates already in place for Windows/Linux paths |
| **OpenSCAD integration** | File path argument | File watcher, or `bambumate serve` mode with local HTTP API for Tauri IPC |

## Sources

- Bambu Studio GitHub Wiki -- Command Line Usage: https://github.com/bambulab/BambuStudio/wiki/Command-Line-Usage
- Bambu Studio Profile Paths (forum): https://forum.bambulab.com/t/where-are-the-files-for-user-filament-and-process-profiles-located/7579
- Bambu Studio macOS Application Support (forum): https://forum.bambulab.com/t/application-support-folder-on-mac/55879
- Bambu Studio Submit Preset (wiki): https://wiki.bambulab.com/en/bambu-studio/submit-preset
- SlicerPrintProfiles repository: https://github.com/alexmi256/SlicerPrintProfiles
- Actual profile JSON files verified from local install at `~/Library/Application Support/BambuStudio/`
- Rust CLI patterns with Clap: https://kbknapp.dev/cli-structure-01/
- Rust error handling (thiserror + anyhow): https://leapcell.io/blog/choosing-the-right-rust-error-handling-tool
- Serde flatten for unknown fields: https://serde.rs/attr-flatten.html
- Rust web scraping (reqwest + scraper): https://www.scrapingbee.com/blog/web-scraping-rust/

---
*Architecture research for: BambuMate -- Rust CLI for Bambu Studio profile management and AI print analysis*
*Researched: 2026-02-04*
