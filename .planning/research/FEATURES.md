# Feature Research

**Domain:** Bambu Lab 3D printer companion CLI -- filament profile generation, print analysis, slicer integration
**Researched:** 2026-02-04
**Confidence:** MEDIUM (ecosystem survey based on web research; profile format details verified against official wiki and GitHub repos; AI vision analysis capabilities verified against multiple sources; no hands-on testing of competitor tools)

## Feature Landscape

### Table Stakes (Users Expect These)

Features users assume exist. Missing these = product feels incomplete.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **Valid Bambu Studio profile JSON output** | The entire value proposition depends on generating profiles that actually load in Bambu Studio without errors. Profiles use a tree-structured JSON format with inheritance (`inherits` field pointing to base like "Generic PLA"), `filament_id` (starts with "GF"), `setting_id` (starts with "GFS"), `instantiation`, and `compatible_printers`. If the output JSON is malformed or missing required fields, users will abandon the tool immediately. | MEDIUM | Must reverse-engineer the exact schema from BambuStudio source code (`resources/profiles/` directory on GitHub). The official wiki documents the structure but leaves edge cases undocumented. Need to handle inheritance correctly -- child profiles only need to override fields that differ from parent. |
| **Correct OS-specific profile installation paths** | Users expect `bambumate install` to put profiles in the right place. macOS: `~/Library/Application Support/BambuStudio/User/`, Windows: `%AppData%/BambuStudio/user/`, Linux: `~/.config/BambuStudio/user/`. Getting this wrong means profiles don't appear in Bambu Studio. | LOW | Well-documented in community forums. Straightforward path detection. Must handle both filament/ and process/ subdirectories. |
| **Core filament parameter coverage** | At minimum, users expect nozzle temperature, bed temperature, fan speed, retraction length/speed, and flow ratio. These are the parameters that most directly affect print quality and that manufacturers publish. | LOW | These ~10-15 parameters are well-documented. Community profile repos (BambuProfiles, BambuStudioFilamentLibrary) show which fields people actually set. |
| **Major filament brand coverage** | Users will try their exact filament first. If the scraper can't find Polymaker, eSUN, Hatchbox, Overture, Inland, Prusament, SUNLU, or Bambu's own filaments, they'll conclude the tool is useless. Need at least the top 10-15 brands. | MEDIUM | Each brand's website has different structure. Manufacturer pages, TDS PDFs, and 3dfilamentprofiles.com (19,957 filaments, 833 brands) are data sources. AI-assisted extraction from unstructured pages reduces per-brand engineering cost. |
| **Readable, actionable defect analysis output** | When users submit a print photo, they expect specific defect names (stringing, warping, elephant's foot, z-banding, layer adhesion, overhangs) with severity levels and concrete setting change recommendations ("reduce nozzle temp by 5C", "increase retraction to 1.2mm"). Vague "your print has issues" output is worthless. | MEDIUM | Claude and GPT-4V already handle this well in ad-hoc usage. The engineering challenge is the structured prompt + output parsing, not the AI capability. Must map defect types to specific Bambu Studio profile parameter names. |
| **Bambu Studio CLI launch with file/profile arguments** | Bambu Studio supports CLI arguments: `bambu-studio file.stl`, `--load-filaments`, `--load-settings`. Users expect BambuMate to leverage this for seamless handoff. | LOW | Well-documented on [BambuStudio GitHub wiki](https://github.com/bambulab/BambuStudio/wiki/Command-Line-Usage). Straightforward subprocess invocation. macOS needs to handle `.app` bundle path. |
| **Profile inheritance from correct base types** | Profiles must inherit from the right base (Generic PLA, Generic PETG, Generic ABS, etc.) or Bambu Studio won't handle them correctly. A PLA profile inheriting from Generic ABS would produce garbage defaults. | LOW | Mapping filament material type to base profile name is a simple lookup table. The tricky part is handling specialty materials (silk PLA, carbon fiber PETG, wood PLA) that may need different bases. |
| **Caching of scraped filament data** | Web scraping is slow and rate-limited. Users expect the second lookup for the same filament to be instant. Without caching, the tool feels broken. | LOW | Standard file-based cache with TTL. Store scraped specs as local JSON. |
| **Clear error messages and graceful failures** | CLI tools that silently fail or produce cryptic errors lose users fast. Must handle: filament not found, AI API key not configured, Bambu Studio not installed, profile directory doesn't exist, network errors. | LOW | Standard Rust CLI practice. Use `anyhow`/`thiserror` for error chains. |

### Differentiators (Competitive Advantage)

Features that set BambuMate apart. Not required, but create the "why this tool" answer.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **AI vision analysis of print photos** | No existing Bambu-specific tool offers "upload photo, get profile fix." Obico and SimplyPrint do real-time failure detection during printing (spaghetti, bed adhesion), but neither analyzes completed prints and maps defects to specific slicer settings. This is the core differentiator -- closing the loop from "I see a problem" to "here's the fix in your profile." | HIGH | Requires well-crafted prompts for Claude/GPT-4V, structured output parsing, and a defect-to-setting mapping engine. The AI does the vision; BambuMate does the translation to actionable profile changes. Accuracy depends heavily on prompt engineering and the mapping rules. |
| **Auto-apply analysis recommendations to profiles** | Other tools tell you what's wrong. BambuMate fixes it. `bambumate analyze photo.jpg --apply` modifies the profile JSON in-place with the recommended changes. No manual editing, no copy-paste from a tutorial. | MEDIUM | Depends on AI analysis feature. JSON manipulation in Rust is straightforward. Must preserve profile structure and only modify relevant fields. Needs safety: backup original, show diff before applying, allow undo. |
| **Web-scraped manufacturer specs to profile pipeline** | No existing tool automates "I bought Polymaker PLA Pro, give me a Bambu Studio profile." Users currently: (1) Google the filament, (2) find the TDS, (3) manually enter 10+ settings into Bambu Studio. BambuMate collapses this to one command. | HIGH | Web scraping is inherently fragile. Each manufacturer site has different structure. AI-assisted extraction (feed page HTML to LLM, extract structured specs) is more robust than traditional scraping but costs API calls. Must handle: no data found, partial data, conflicting specs. |
| **OpenSCAD Studio integration** | Unique to this ecosystem. No other tool bridges parametric CAD (OpenSCAD) to Bambu Studio with optimized profiles. For the developer's own workflow, this is high-value. For the broader market, this is niche but interesting. | MEDIUM | Depends on OpenSCAD Studio's export format and IPC mechanism. If it's just "accept an STL path and launch Bambu Studio with it," complexity is LOW. If it involves bidirectional communication, MEDIUM-HIGH. |
| **Unified filament-to-print workflow in a single CLI** | Existing tools are fragmented: 3dfilamentprofiles.com for lookup, Bambu Studio UI for profile creation, OrcaSlicer for calibration, separate analysis tools. BambuMate combines lookup + generate + analyze + apply + launch in one `bambumate` command with subcommands. | LOW (architecture) | The integration itself is the value, not any single feature. This is a design advantage, not an engineering challenge. |
| **Defect-to-setting mapping knowledge base** | A curated, version-controlled mapping from defect types to Bambu Studio parameter adjustments (e.g., stringing -> retraction_length +0.4mm, nozzle_temperature -5C). This makes the AI analysis actionable and improvable over time. | MEDIUM | Requires 3D printing domain expertise to build the initial mapping. Must handle interactions (fixing stringing by increasing retraction can cause under-extrusion). The mapping is the "secret sauce" that makes AI recommendations specific. |
| **Profile diff and comparison** | `bambumate diff profile-a.json profile-b.json` shows what changed between profiles. Useful for understanding what the AI changed, comparing community profiles, or debugging print quality differences. | LOW | JSON diff is trivial. The value is in presenting it in a human-readable, 3D-printing-aware format (group by category: temperatures, retraction, speeds, cooling). |
| **Batch profile generation** | `bambumate generate --brand polymaker --all-materials` generates profiles for every Polymaker filament in one shot. Power users with many filaments save hours. | LOW | Extension of single-filament generation. Main challenge is scraping multiple product pages without getting rate-limited. |

### Anti-Features (Commonly Requested, Often Problematic)

Features that seem good but create problems. Deliberately NOT building these.

| Feature | Why Requested | Why Problematic | Alternative |
|---------|---------------|-----------------|-------------|
| **Real-time print monitoring via MQTT** | Users want to watch prints remotely and catch failures. The PRD originally included this as Phase 2. | Massive scope increase (MQTT client, TLS, auth, dashboard, alerts). Post-Jan-2025 firmware requires authentication for most operations. Obico and SimplyPrint already do this well. Splits focus from the core value prop (profile intelligence). | Stay file-based and offline. The CLI tool generates profiles and analyzes photos. Users who want monitoring already have Obico/SimplyPrint/Bambu Handy. |
| **Community profile sharing platform** | Users want to discover profiles others have tested. The PRD included this as Phase 4. | Requires server infrastructure, user accounts, moderation, trust systems. Contradicts "self-hosted, local-only" constraint. Community repos on GitHub (BambuProfiles, BambuStudioFilamentLibrary) already serve this need adequately. | Support importing from existing community repos. `bambumate import --from-github doridian/BambuProfiles`. Let GitHub be the sharing platform. |
| **Built-in GUI / web dashboard** | Users want a visual interface, not a terminal. | Rust CLI + web UI is a different product category (Tauri app). GUI development is 3-5x the effort of CLI. The CLI can always be wrapped later. Building GUI first means shipping nothing for months. | Ship CLI first. Consider a TUI (terminal UI with `ratatui`) for interactive mode later. A GUI wrapper (Tauri) is a future product, not v1. |
| **Multi-printer farm management** | Farm operators want centralized control across many printers. | Enterprise feature with enormous scope. Printago, 3DPrinterOS, and Bambu's own tools serve this market. BambuMate's value is per-filament intelligence, not fleet management. | Profiles generated by BambuMate work across any number of printers. The profile is the product, not the printer management. |
| **Local ML model inference** | Running defect detection models locally avoids API costs and latency. | Requires bundling large model files, GPU dependencies, and ML runtime. Dramatically increases binary size and system requirements. Claude/GPT-4V are better at this than any model we could run locally today, especially for the nuanced "what setting should I change" reasoning. | Use external AI APIs (Claude/GPT-4V). The API cost per analysis is cents. If API costs become a concern, consider a lightweight local pre-filter that only sends unclear cases to the API. |
| **Filament inventory/spool tracking** | Users want to track how much filament is left on each spool. Spoolman and FilaMan do this with NFC + weight sensors. | Physical spool tracking requires hardware integration (NFC, scales) or AMS MQTT data. Completely orthogonal to profile generation. Spoolman already does this well and has mature integrations with Klipper/OctoPrint. | Don't track spools. Generate profiles. If users want spool tracking, they use Spoolman. BambuMate could output Spoolman-compatible metadata as a nice-to-have. |
| **Support for non-Bambu printers** | "Can you add PrusaSlicer/Cura support?" is inevitable. | Each slicer has a completely different profile format, directory structure, and parameter naming. Supporting even one additional slicer doubles the profile generation complexity. Bambu-only focus enables deep integration. | Bambu-only is a feature, not a limitation. Deep integration with one ecosystem beats shallow integration with many. If demand is overwhelming, PrusaSlicer could be a v2 target since OrcaSlicer (Bambu fork) profiles are similar. |
| **Automatic calibration print generation** | OrcaSlicer has built-in temperature towers, flow rate tests, retraction tests. Users might want BambuMate to generate these. | OrcaSlicer already does this better than we could. Generating calibration G-code is a slicer's job, not a profile manager's job. | Instead, analyze photos OF calibration prints. "Run OrcaSlicer's temp tower, photograph it, feed to `bambumate analyze`." BambuMate interprets results, not generates tests. |

## Feature Dependencies

```
[Filament Web Scraping]
    |
    +--produces data for--> [Profile JSON Generation]
    |                           |
    |                           +--requires--> [Bambu Studio JSON Schema Knowledge]
    |                           |
    |                           +--outputs to--> [Profile Installation to BS Config Dir]
    |                           |
    |                           +--consumed by--> [Bambu Studio CLI Launch]
    |
    +--data can be enriched by--> [AI Vision Print Analysis]
                                       |
                                       +--requires--> [AI API Integration (Claude/GPT-4V)]
                                       |
                                       +--feeds into--> [Defect-to-Setting Mapping Engine]
                                       |                     |
                                       |                     +--applies to--> [Auto-Tuning (Profile Modification)]
                                       |                                          |
                                       |                                          +--modifies--> [Profile JSON Generation] (existing profiles)
                                       |
                                       +--enhances--> [Profile JSON Generation] (initial profile improvement)

[OpenSCAD Studio Integration]
    |
    +--accepts STL from--> [OpenSCAD Studio export]
    |
    +--launches--> [Bambu Studio CLI Launch]
    |
    +--optionally uses--> [Profile JSON Generation] (load profile with STL)
```

### Dependency Notes

- **Profile JSON Generation requires Bambu Studio JSON Schema Knowledge:** This is the foundational dependency. Without understanding the exact JSON format (inheritance, required fields, naming conventions), nothing else works. Must be built first and validated thoroughly.
- **AI Vision Analysis requires AI API Integration:** The Claude/GPT-4V integration must be working before any analysis features can be built. This includes API key management, request/response handling, and structured output parsing.
- **Auto-Tuning requires both Profile Generation AND AI Analysis:** Auto-tuning is the convergence point -- it takes AI analysis output and applies it to profile JSON. Both upstream features must be solid before this makes sense.
- **Filament Scraping and AI Analysis are independent:** These can be built in parallel. Scraping produces initial profiles from manufacturer specs; AI analysis improves profiles from print results. They converge at profile generation but don't depend on each other.
- **OpenSCAD Studio Integration is fully independent:** This feature has no dependencies on other BambuMate features. It's a standalone workflow bridge that can be built at any time. Its only external dependency is OpenSCAD Studio's export mechanism.
- **Bambu Studio CLI Launch is a leaf dependency:** Multiple features feed into it (profile generation loads profiles, OpenSCAD integration passes STLs), but it depends on nothing within BambuMate except knowing the Bambu Studio binary path.

## MVP Definition

### Launch With (v1)

Minimum viable product -- what's needed to validate that people want AI-assisted filament profile management.

- [ ] **Filament spec lookup** (`bambumate lookup "Polymaker PLA Pro"`) -- Scrape manufacturer specs and display structured output (temperatures, speeds, cooling, retraction). This is the entry point; users can validate accuracy before trusting profile generation.
- [ ] **Profile generation** (`bambumate generate "Polymaker PLA Pro" --printer p1s`) -- Produce a valid Bambu Studio filament profile JSON that inherits from the correct base and sets scraped parameters. This is the core deliverable.
- [ ] **Profile installation** (`bambumate install profile.json`) -- Copy the generated profile to the correct Bambu Studio config directory for the user's OS. Without this, users must manually find the directory.
- [ ] **AI print analysis** (`bambumate analyze photo.jpg`) -- Send a test print photo to Claude/GPT-4V, receive structured defect analysis with severity scores and setting change recommendations. This is the "wow" feature that differentiates from static profile repos.
- [ ] **Bambu Studio JSON schema handling** -- Correct inheritance, required fields, naming conventions. The invisible foundation everything depends on.

### Add After Validation (v1.x)

Features to add once core is working and users confirm the value.

- [ ] **Auto-apply recommendations** (`bambumate analyze photo.jpg --apply profile.json`) -- Trigger: users manually copying AI recommendations into profiles, proving they trust the analysis.
- [ ] **Bambu Studio launch integration** (`bambumate launch model.stl --profile profile.json`) -- Trigger: users asking "how do I get this into Bambu Studio faster?"
- [ ] **Profile diff/compare** (`bambumate diff a.json b.json`) -- Trigger: users wanting to understand what changed between iterations.
- [ ] **Batch generation** (`bambumate generate --brand polymaker --all`) -- Trigger: power users requesting profiles for their entire filament collection.
- [ ] **OpenSCAD Studio bridge** -- Trigger: OpenSCAD Studio workflow integration demand from the developer's own usage.

### Future Consideration (v2+)

Features to defer until product-market fit is established.

- [ ] **Import from community repos** (`bambumate import --from-github ...`) -- Why defer: requires designing a repo format standard. GitHub repos already work via manual download.
- [ ] **Interactive TUI mode** -- Why defer: CLI subcommands are sufficient for v1. TUI adds polish but not capability.
- [ ] **Process profile generation** (not just filament) -- Why defer: process profiles (layer height, infill, speed) are more complex and more subjective than filament profiles. Filament profiles are the higher-value target.
- [ ] **Multi-photo analysis** (multiple angles of same print) -- Why defer: single-photo analysis validates the concept. Multi-photo improves accuracy but is additive, not foundational.
- [ ] **Calibration print interpretation** (analyze temp tower photos to extract optimal temp) -- Why defer: requires specialized image analysis beyond general defect detection. High value but high complexity.

## Feature Prioritization Matrix

| Feature | User Value | Implementation Cost | Priority |
|---------|------------|---------------------|----------|
| Profile JSON generation (valid output) | HIGH | MEDIUM | P1 |
| Bambu Studio JSON schema handling | HIGH | MEDIUM | P1 |
| Filament spec web scraping | HIGH | HIGH | P1 |
| Profile installation to correct OS path | HIGH | LOW | P1 |
| AI print analysis (defect detection) | HIGH | MEDIUM | P1 |
| Caching of scraped data | MEDIUM | LOW | P1 |
| Error handling and CLI UX | MEDIUM | LOW | P1 |
| Auto-apply AI recommendations | HIGH | MEDIUM | P2 |
| Bambu Studio CLI launch | MEDIUM | LOW | P2 |
| Defect-to-setting mapping engine | HIGH | MEDIUM | P2 |
| Profile diff/compare | MEDIUM | LOW | P2 |
| OpenSCAD Studio integration | MEDIUM | MEDIUM | P2 |
| Batch profile generation | MEDIUM | LOW | P2 |
| Import from community repos | LOW | MEDIUM | P3 |
| Interactive TUI mode | LOW | MEDIUM | P3 |
| Process profile generation | MEDIUM | HIGH | P3 |
| Multi-photo analysis | MEDIUM | MEDIUM | P3 |
| Calibration print interpretation | HIGH | HIGH | P3 |

**Priority key:**
- P1: Must have for launch -- validates the core value proposition
- P2: Should have, add when core is proven -- extends value and improves workflow
- P3: Nice to have, future consideration -- expands scope after product-market fit

## Competitor Feature Analysis

| Feature | 3dfilamentprofiles.com | BambuProfiles (GitHub) | OrcaSlicer Calibration | Obico / SimplyPrint | Spoolman / FilaMan | Printago | **BambuMate** |
|---------|----------------------|----------------------|----------------------|-------------------|-------------------|---------|-------------|
| Filament spec database | 19,957 filaments, 833 brands. Web-based search/compare. | Curated community profiles with test print photos. ~30 filaments. | N/A | N/A | Community filament DB for inventory | N/A | AI-scraped from manufacturer sites on demand. Unlimited brands. |
| Profile generation | N/A (data only) | Pre-made JSON files for import | Built-in for calibration results | N/A | N/A | Auto-slicing with material awareness | Automated JSON generation from scraped specs |
| Profile format | N/A | Bambu Studio JSON | OrcaSlicer JSON (compatible) | N/A | N/A | Internal | Bambu Studio JSON with correct inheritance |
| Print defect analysis | N/A | N/A | N/A (calibration, not analysis) | Real-time failure detection (spaghetti, bed adhesion). 80M+ hours analyzed. | N/A | N/A | Post-print AI vision analysis with setting recommendations |
| Setting recommendations | Community-contributed parameters | Static profiles | Automated calibration tests (temp tower, flow rate, retraction) | Pause/stop on failure. No setting recommendations. | N/A | N/A | AI-powered defect-to-setting mapping |
| Auto-apply changes | N/A | Manual import | Saves calibration to filament profile | Auto-pause/stop | N/A | Auto-routing | Modifies profile JSON in-place |
| Slicer integration | N/A | Manual import into Bambu Studio | IS the slicer | OctoPrint/Klipper plugin | OctoPrint/Klipper | Cloud slicing | CLI launch with `--load-filaments`, profile installation to config dir |
| Self-hosted / local | N/A (web only) | Local files | Local application | Cloud + self-hosted option | Self-hosted | Cloud | Fully local CLI |
| Bambu-specific | Bambu profiles included | Bambu-only | Bambu + many others | Bambu supported (not primary) | Printer-agnostic | Bambu supported | Bambu-only (deep integration) |

### Key Competitive Insights

1. **No tool closes the full loop.** 3dfilamentprofiles.com has data but no profile generation. OrcaSlicer has calibration but no AI analysis. Obico detects failures but doesn't recommend setting changes. BambuMate's opportunity is the complete pipeline: data -> profile -> print -> analyze -> improve.

2. **AI analysis of completed prints is an open niche.** Obico/SimplyPrint focus on real-time failure detection during printing (spaghetti, bed adhesion loss). Nobody offers "analyze this finished print and tell me how to improve my profile." This is BambuMate's primary differentiator.

3. **Bambu-specific depth beats breadth.** OrcaSlicer supports dozens of printers but its Bambu support is generic. BambuMate knowing the exact Bambu Studio JSON schema, inheritance model, profile paths, and CLI arguments provides a smoother experience for Bambu users specifically.

4. **CLI is an underserved interface.** The existing bambu-cli (davglass) focused on printer control and is defunct due to auth changes. No CLI tool focuses on the profile management workflow. Power users and automation workflows (CI/CD for print farms) benefit from CLI.

5. **The fragmentation is the opportunity.** Users currently bounce between 3-4 tools/websites to go from "I bought new filament" to "I have an optimized profile." Consolidating this into one command is genuine workflow improvement.

## Sources

**Official Bambu Lab Documentation:**
- [Bambu Studio Command Line Usage](https://github.com/bambulab/BambuStudio/wiki/Command-Line-Usage) -- CLI arguments for launching with files/profiles
- [Creating Custom Filaments in Bambu Studio](https://wiki.bambulab.com/en/bambu-studio/create-filament) -- Official filament creation workflow
- [How to Submit Filament to be Preset in Bambu Studio](https://wiki.bambulab.com/en/bambu-studio/submit-preset) -- Profile JSON format documentation
- [Bambu Studio Filament Profile Package Update Guide](https://wiki.bambulab.com/en/software/bambu-studio/filament-package-update) -- Profile packaging
- [Bambu Lab Filament Guide Material Table](https://wiki.bambulab.com/en/general/filament-guide-material-table) -- Official filament compatibility

**Community Profile Repositories:**
- [Doridian/BambuProfiles](https://github.com/Doridian/BambuProfiles) -- Community Bambu Lab profiles with test prints (27 stars, CC0)
- [dgauche/BambuStudioFilamentLibrary](https://github.com/dgauche/BambuStudioFilamentLibrary) -- 3rd party filament settings library
- [lestephen/bambu-filament](https://github.com/lestephen/bambu-filament) -- X1C filament profiles
- [BambuStudio resources/profiles](https://github.com/bambulab/BambuStudio/tree/master/resources/profiles) -- Official built-in profiles (source of truth for JSON schema)

**Competitor Tools:**
- [3D Filament Profiles](https://3dfilamentprofiles.com/) -- 19,957 filaments, 833 brands, search/compare database
- [Obico AI Failure Detection](https://www.obico.io/failure-detection.html) -- Real-time AI detection, 80M+ hours, YOLOv3-based
- [SimplyPrint AI Detection](https://simplyprint.io/features/ai-detection) -- 5M+ hours trained, real-time monitoring
- [Printago](https://www.printago.io/) -- Print farm automation for Bambu Lab with material management
- [Spoolman](https://github.com/Donkie/Spoolman) -- Self-hosted filament spool inventory management
- [FilaMan](https://github.com/ManuelW77/Filaman) -- ESP32-based NFC spool tracking with Spoolman integration
- [OrcaSlicer Calibration](https://github.com/OrcaSlicer/OrcaSlicer/wiki/Calibration) -- Built-in temp tower, flow rate, retraction, tolerance tests
- [davglass/bambu-cli](https://github.com/davglass/bambu-cli) -- Defunct CLI for Bambu printers (printer control, not profiles)

**Community Pain Points:**
- [Profile confusion discussion](https://forum.bambulab.com/t/bambu-studio-profile-confusion-and-some-clarity/124700)
- [New users frustrations](https://forum.bambulab.com/t/new-users-frustrations/145333)
- [Profiles lost on upgrade](https://github.com/bambulab/BambuStudio/issues/1171)
- [Batch profile transfer request](https://github.com/bambulab/BambuStudio/issues/7484)
- [Filament management usability request](https://forum.bambulab.com/t/feature-request-filament-profiles-managment-that-are-usable/28877)
- [Can't add custom filament profile](https://forum.bambulab.com/t/bambu-studio-p2s-cant-add-custom-filament-profile/206551)

**AI for 3D Printing:**
- [ORNL Peregrine (real-time quality assessment)](https://www.ornl.gov/news/ai-software-enables-real-time-3d-printing-quality-assessment)
- [New AI system fixes 3D printing defects in real time (Feb 2026)](https://techxplore.com/news/2026-02-ai-3d-defects-real.html) -- Multi-agent LLM system for detection + correction
- [Computer vision feedback systems for 3D printers (Ultralytics)](https://www.ultralytics.com/blog/computer-vision-based-feedback-systems-for-3d-printers)

---
*Feature research for: BambuMate -- Bambu Lab companion CLI*
*Researched: 2026-02-04*
