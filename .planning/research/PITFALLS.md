# Pitfalls Research

**Domain:** Bambu Studio companion CLI tool (Rust) -- filament scraping, AI print analysis, slicer profile generation
**Researched:** 2026-02-04
**Confidence:** MEDIUM-HIGH (profile format and directory pitfalls verified via official sources and GitHub issues; AI vision pitfalls extrapolated from Obico and domain literature; scraping pitfalls from general web scraping domain knowledge)

---

## Critical Pitfalls

### Pitfall 1: Bambu Studio Updates Silently Break or Delete Custom Profiles

**What goes wrong:**
Bambu Studio updates frequently reset, overwrite, or mark custom filament profiles as "unsupported." This is a recurring, well-documented problem across multiple Bambu Studio versions (1.8.x, 2.3.x, 2.4.x). Custom profiles placed by BambuMate into the user directory can vanish after a Bambu Studio update, leaving users with no profiles and no explanation. The H2D printer introduction in 2025 caused widespread "unsupported" labeling of previously working custom profiles.

**Why it happens:**
Bambu Studio's profile system has a fragile relationship between system presets and user presets. When Bambu updates the system preset hierarchy or adds new printers, user profiles that inherit from changed parent presets break silently. The `compatible_printers` field is validated against the current printer list -- new printers added or renamed by Bambu break existing profiles. Cloud sync can also overwrite local changes.

**How to avoid:**
- Never assume profiles are permanent once written. Design BambuMate to re-generate profiles on demand, not just as a one-time action.
- Store BambuMate's own canonical profile data separately (in BambuMate's own config directory), and write to Bambu Studio's directory as an export/install step.
- Implement profile health checks: before any operation, verify the target profiles still exist and are valid in Bambu Studio's directory.
- Include a `bambumate profiles check` command that validates installed profiles against the current Bambu Studio version.
- Track the Bambu Studio version number and warn users when a version change is detected.

**Warning signs:**
- Users report "my profiles disappeared" after updating Bambu Studio.
- The `compatible_printers` field references printer names that no longer exist in the current Bambu Studio version.
- Profiles appear under "Unsupported" in Bambu Studio's UI despite being in the correct directory.

**Phase to address:**
Phase 1 (Profile Generator). This is the foundation -- if profiles break on Bambu Studio update, the entire tool loses trust. Build the "source of truth stays in BambuMate" pattern from day one.

---

### Pitfall 2: Profile JSON Format Is Undocumented and Changes Without Notice

**What goes wrong:**
The Bambu Studio filament profile JSON schema is not formally documented. Required fields, naming conventions, inheritance rules, and validation logic must be reverse-engineered from the BambuStudio open-source repository (`resources/profiles/BBL.json` and subdirectories). Bambu has introduced new required fields across versions (e.g., dual-nozzle parameters for H2D), and the CLI's `--load-filaments` flag requires "full JSONs including the inherit values from their inherited jsons" -- partial profiles that work when manually imported may fail when loaded programmatically.

**Why it happens:**
Bambu Studio is an open-source slicer but the profile format is an internal implementation detail, not a public API. Bambu Lab treats it as mutable. New printer models (H2D, P2S) introduce new fields. The `filament_id` (must start with "GF"), `setting_id` (must start with "GFS"), `inherits`, `instantiation`, `from`, and `compatible_printers` fields all have undocumented constraints that change between versions.

**How to avoid:**
- Pin BambuMate's profile generation to a specific, tested Bambu Studio version. When a new Bambu Studio version ships, regression-test generated profiles before claiming support.
- Parse actual Bambu Studio system profiles from the user's installation as the schema reference, not hardcoded assumptions. Read the existing system profiles at runtime to discover current field names, printer names, and inheritance chains.
- Write a comprehensive profile validation module that checks every generated JSON against a known-good exemplar before writing to disk.
- Maintain a test suite of golden-file profiles for each supported Bambu Studio version.
- Generate profiles that inherit from Bambu's existing base profiles (e.g., "Generic PLA") and override only the fields you need. Do not attempt to generate complete standalone profiles.

**Warning signs:**
- Bambu Studio silently ignores imported profiles (they don't appear in the UI).
- Bambu Studio shows "Action Required" next to profiles.
- New Bambu Studio releases change the `resources/profiles` directory structure.
- The `--load-filaments` CLI flag rejects generated JSON files.

**Phase to address:**
Phase 1 (Profile Generator). Build a profile format detection/adaptation layer from the start. This must be the first thing tested when supporting a new Bambu Studio version.

---

### Pitfall 3: Cloud Sync Conflicts Destroy Locally-Written Profiles

**What goes wrong:**
Bambu Studio's cloud sync feature can silently overwrite local profile files. If BambuMate writes a profile to disk and the user has cloud sync enabled, Bambu Studio may download an older version from the cloud and overwrite BambuMate's changes on next launch. Worse, malformed JSON values (e.g., `"nil"` in arrays) can trigger a cascading sync failure where the cloud version overwrites all local profiles.

**Why it happens:**
Bambu Studio's sync mechanism compares local and cloud profiles by timestamp and content. A third-party tool writing directly to the profile directory bypasses the sync protocol. The `.info` metadata files that track sync state are not updated by external writes. Bambu Studio treats externally-modified profiles as either corrupted or out-of-date and reverts them.

**How to avoid:**
- Use Bambu Studio's `File > Import > Import Configs` pathway rather than direct file writes whenever possible. Generate `.json` or `.bbsflmt` bundle files that users import through the UI.
- If direct file writing is required, also update the accompanying `.info` metadata files with correct timestamps.
- Warn users clearly about cloud sync conflicts. Consider a `bambumate install-profile --disable-cloud-warning` flag.
- Validate all array fields for consistency (no `"nil"` values mixed with numeric strings) before writing.
- Provide a `--export-only` mode that generates the profile file but does not install it, letting users import manually.

**Warning signs:**
- Users report profiles reverting to old values after Bambu Studio restart.
- Profile files have unexpected modification timestamps.
- The `.info` sidecar file next to a profile has a different timestamp than the profile itself.

**Phase to address:**
Phase 1 (Profile Installation). The installation strategy (direct write vs. UI import vs. `.bbsflmt` bundle) must be decided early and tested against cloud sync scenarios.

---

### Pitfall 4: AI Vision Defect Analysis Is Ambiguous -- Same Symptom, Multiple Causes

**What goes wrong:**
Many 3D printing defects have overlapping visual signatures. Stringing can be caused by high temperature, insufficient retraction, or too-slow travel speed. Elephant's foot can be caused by nozzle too close to bed OR bed temperature too high. Warping can be caused by insufficient bed adhesion, drafts, or wrong first-layer settings. An AI vision model analyzing a photo cannot determine which root cause applies without additional context (filament type, current settings, printer model, environment).

**Why it happens:**
Developers assume "see defect -> recommend fix" is a simple mapping. In reality, defect-to-setting relationships are many-to-many. The correct fix depends on the current profile settings, the specific filament, the printer model, and sometimes environmental conditions. A recommendation of "lower temperature 5 degrees" could be wrong if temperature is already at the minimum and the real cause is retraction distance.

**How to avoid:**
- Always require the current profile settings as context alongside the photo. The AI prompt must include current temperature, retraction, speed, fan settings, and filament type.
- Generate ranked lists of possible causes, not single-point recommendations. Present "most likely: X, also possible: Y, Z" to the user.
- Implement conservative change boundaries: never recommend changes that exceed safe ranges for the filament type (e.g., don't suggest PLA at 250C).
- Design the defect-to-settings mapping as a separate, testable module (not embedded in AI prompts). Use the AI for defect detection and a rule engine for setting recommendations.
- Include confidence scores with every recommendation and require user confirmation before applying changes.
- Build in an iterative loop: first change is small, user prints again, re-analyzes, system refines.

**Warning signs:**
- Users report that applying recommended changes made things worse.
- AI recommends contradictory changes across different defects in the same print.
- Recommendations exceed safe operating ranges for the filament type.

**Phase to address:**
Phase 2 (AI Analysis). But the architecture decision (AI detects defects, rule engine maps to settings) must be made in Phase 1 design.

---

### Pitfall 5: Web Scraping Filament Data Is Fragile and Inconsistent Across Manufacturers

**What goes wrong:**
Every filament manufacturer structures their product pages differently. Polymaker uses a clean tabular format. eSUN buries specs in product descriptions. Hatchbox has minimal specs on Amazon listings. Some manufacturers use JavaScript-rendered content that `reqwest` cannot see. Temperature ranges are expressed inconsistently: "200-230C," "210+-10C," "recommended: 215C," "nozzle temp: 200-230 / bed: 55-65." Pages change layout without warning, breaking scrapers.

**Why it happens:**
Developers build a scraper for one manufacturer and assume others follow the same pattern. Each brand has different web architecture, different spec formats, and different levels of detail. Some use Shopify (server-rendered), some use React SPAs (JavaScript-required), some only publish PDFs. The data itself is inconsistent -- some brands specify fan speed recommendations, others don't mention cooling at all.

**How to avoid:**
- Use LLM-based extraction as the primary method, not CSS-selector scraping. Send the page content (or cleaned text) to Claude/GPT and ask it to extract structured data. This is more resilient to layout changes than hardcoded selectors.
- Validate extracted data against physical constraints: nozzle temp 0-400C, bed temp 0-120C, retraction 0-15mm, etc. Reject and flag anything outside bounds.
- Implement per-manufacturer adapter modules, but with a common fallback LLM extractor. When a manufacturer adapter fails, fall back to the generic extractor.
- Cache aggressively. Filament specs rarely change. Cache for 30+ days.
- Include a manual override/correction pathway for users to fix bad extractions.
- Respect `robots.txt`. Rate-limit to 1 request per second per domain. Use appropriate User-Agent strings.
- For JavaScript-heavy sites, consider `chromiumoxide` crate for headless Chromium rendering in Rust, but recognize this adds significant binary size and complexity.

**Warning signs:**
- Extracted temperatures are outside normal ranges (e.g., PLA at 300C).
- Same filament returns different specs on re-scrape.
- Scraper returns empty results for a previously working manufacturer.
- High variance in extracted data fields across brands (some have 3 fields, others have 12).

**Phase to address:**
Phase 1 (Filament Scraper). Start with LLM extraction + validation, not CSS selectors. Support 3-5 brands initially and expand.

---

### Pitfall 6: Writing to Bambu Studio's Config Directory While It's Running

**What goes wrong:**
If BambuMate writes profile files to Bambu Studio's config directory while Bambu Studio is running, the results are unpredictable. Bambu Studio may not see the new files until restart. Worse, it may read a partially-written file and corrupt its internal state. There's no file locking protocol. On macOS, `~/Library/Application Support/BambuStudio/` is the user's Library directory -- no special permissions needed, but concurrent access is unmanaged.

**Why it happens:**
Developers test with Bambu Studio closed and don't consider the concurrent access case. Bambu Studio reads profiles at startup and may cache them in memory. Writing files while it's running creates a race condition.

**How to avoid:**
- Detect whether Bambu Studio is running before writing profiles. On macOS: check for the process. On Windows: check process list or file locks.
- If Bambu Studio is running, warn the user and recommend closing it first, OR use the `Import Configs` approach (generate file, user imports through UI).
- Write profiles atomically: write to a temp file, then rename (atomic on most filesystems). Never write directly to the target path.
- After writing, provide clear instructions: "Restart Bambu Studio to see your new profiles."
- Consider offering a `--force` flag for advanced users who understand the risks.

**Warning signs:**
- Users report "I ran the command but the profile doesn't show up."
- Bambu Studio crashes or shows corrupted settings after BambuMate writes profiles.
- Intermittent "profile appears sometimes" behavior.

**Phase to address:**
Phase 1 (Profile Installation). Implement process detection and atomic writes from the beginning.

---

## Technical Debt Patterns

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| Hardcoding Bambu Studio config path per OS | Fast to implement | Breaks if Bambu Studio changes its config directory or if user has non-standard installation | MVP only; switch to auto-detection (read BambuStudio.conf or discover via running process) in Phase 2 |
| Single AI provider (Claude-only) | Simpler API layer | Vendor lock-in; if Claude API changes or pricing spikes, no fallback | MVP, but define the provider abstraction trait from day one so GPT-4V is easy to add |
| Hardcoding defect-to-settings rules | Works for common defects | Cannot handle edge cases; becomes a maintenance burden as more defects/settings are added | Never acceptable as the sole approach; pair with AI analysis from the start |
| Storing scraped data as raw JSON blobs | Fast to prototype | Difficult to query, validate, or version. No schema migration path | MVP only; add structured types with serde validation by Phase 2 |
| Using CSS selectors for scraping | Fast for one site | Breaks on every site redesign; different per manufacturer | Never acceptable as primary approach; use LLM extraction with selector fallback |
| Generating standalone profiles (not inheriting) | Simpler generation | Profiles miss Bambu Studio updates to base profiles; file size bloat; incompatible with future settings | Never; always inherit from Bambu's base profiles |

## Integration Gotchas

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| Bambu Studio profile directory | Assuming a fixed path without checking the actual installation | Read `BambuStudio.conf` or use platform-specific discovery. macOS: `~/Library/Application Support/BambuStudio/user/<user_id>/filament/`. Windows: `%APPDATA%\BambuStudio\user\<user_id>\filament\`. The `<user_id>` subdirectory varies per user and is not always "default." |
| Bambu Studio CLI (`--load-filaments`) | Passing exported/partial JSON files | CLI requires "full JSONs including the inherit values from their inherited jsons." You must resolve the full inheritance chain and produce a complete JSON, or the CLI will reject it with unhelpful error messages. |
| Bambu Studio CLI (`--load-settings`) | Passing semicolon-separated file paths | Known bug: semicolon-delimited paths are treated as a single filename rather than being split. Test with your target Bambu Studio version. |
| Claude/GPT-4V vision API | Sending high-resolution photos without size constraints | Vision APIs have token limits for image analysis. Resize photos to reasonable dimensions (1024px max) before sending. Large images cost more tokens without proportional accuracy improvement. |
| Claude/GPT-4V for data extraction | Trusting extracted numeric values without validation | LLMs hallucinate numbers. A model might return "nozzle temperature: 250" for PLA because it confused the filament with ABS. Always validate extracted data against known physical constraints. |
| OpenSCAD Studio integration | Tight coupling between the two tools | Use a well-defined IPC contract (e.g., CLI invocation with specific arguments, or a temp file handoff). Don't share internal data structures. OpenSCAD Studio should call BambuMate as an external process. |
| Bambu Studio `.bbsflmt` bundle format | Treating it as a simple JSON file | `.bbsflmt` is a bundle format (likely zip-based or structured container). Investigate the actual format before attempting to generate it. Fall back to plain `.json` export if `.bbsflmt` generation is too complex. |
| macOS file permissions | Assuming `~/Library/Application Support/` is always writable | On sandboxed macOS environments or managed devices, this directory may require explicit permissions. Test on both standard and restricted macOS configurations. |

## Performance Traps

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Scraping manufacturer websites synchronously | CLI feels slow; 10+ seconds to look up one filament | Use async HTTP with `tokio` + `reqwest`. Cache results aggressively (30-day TTL for filament specs). Run scraping in background, return cached data immediately if available. | Immediately noticeable with 3+ manufacturer lookups |
| Sending full-size photos to vision API | Slow response (10+ seconds), high token cost, API timeouts | Resize images to max 1024px on longest side before API call. Strip EXIF data. Use JPEG compression. | First use with a high-res camera photo (4000+ px) |
| Reading all Bambu Studio profiles to find the right one | Slow profile discovery on systems with many profiles | Index profiles on first run and cache the index. Update index incrementally. | Users with 50+ custom profiles across multiple printers |
| Headless browser for scraping | 500MB+ binary size, slow startup, memory-heavy | Only use headless browser for sites that absolutely require JavaScript rendering. Prefer static HTML scraping + LLM extraction. | Immediately; Chromium adds ~150MB to binary |
| LLM API calls for every profile generation | Slow, expensive, rate-limited | Separate scraping (needs LLM) from profile generation (template-based). Cache LLM extraction results. A profile generation from cached data should be <100ms. | When generating profiles for 10+ filaments in batch |

## Security Mistakes

| Mistake | Risk | Prevention |
|---------|------|------------|
| Storing AI API keys in config files without encryption | Keys exposed if config directory is shared or backed up to cloud | Use OS keychain (macOS Keychain, Windows Credential Manager) for API keys. Support environment variables as alternative. Never write keys to the profile JSON. |
| Scraping without respecting `robots.txt` | Legal liability; IP bans; manufacturer cease-and-desist | Check `robots.txt` before scraping. Implement rate limiting. Cache aggressively. Include identifiable User-Agent. |
| Writing arbitrary data from web scraping into profile JSON | Injection of malformed JSON that corrupts Bambu Studio state | Sanitize all scraped data. Validate against schema. Never write raw scraped strings into profile fields without type-checking and range-checking. |
| Sending user's print photos to AI API without disclosure | Privacy violation; user may not realize photos leave their machine | Clear first-run disclosure: "Photos will be sent to [Claude/OpenAI] API for analysis." Provide offline analysis option (even if limited). |
| Trusting AI-recommended settings without bounds checking | Dangerous printer settings (e.g., nozzle temp 400C for PLA) could damage hardware | Hard-code absolute maximum/minimum bounds per filament type. No AI recommendation should exceed safe operating ranges regardless of what the model suggests. |

## UX Pitfalls

| Pitfall | User Impact | Better Approach |
|---------|-------------|-----------------|
| Applying AI-recommended changes without user confirmation | User loses their manually tuned settings; change might make things worse | Always show a diff: "Current: 215C -> Recommended: 210C." Require explicit confirmation. Offer undo/rollback. |
| Silent failure when Bambu Studio path is wrong | User runs command, nothing happens, no error | Validate Bambu Studio installation on first run. Store discovered path in BambuMate config. Provide `bambumate doctor` command that checks all prerequisites. |
| Jargon-heavy defect reports | Novice users don't understand "retraction distance" or "flow ratio" | Provide two levels: simple ("Temperature is too high -- try lowering it 5 degrees") and detailed ("Reduce nozzle_temperature from 215 to 210 based on stringing analysis"). |
| Overwriting user's existing profile without backup | Permanent loss of carefully tuned settings | Before any profile modification, create a timestamped backup in BambuMate's own directory. Provide `bambumate profiles restore` command. |
| Requiring Bambu Studio restart without clear messaging | User thinks the tool is broken because changes don't appear | After profile installation, print explicit instruction: "Profile installed. Restart Bambu Studio to see changes." If Bambu Studio is running, detect this and warn. |

## "Looks Done But Isn't" Checklist

- [ ] **Profile Generator:** Often missing the `compatible_printers` field with correct printer+nozzle strings (e.g., "Bambu Lab P1S 0.4 nozzle") -- verify profiles appear in Bambu Studio for the user's specific printer model and nozzle size
- [ ] **Profile Generator:** Often missing the `inherits` field or pointing to a non-existent parent -- verify the parent profile name exists in the user's Bambu Studio installation
- [ ] **Profile Generator:** Often using wrong `filament_id`/`setting_id` prefix -- verify `filament_id` starts with "GF" and `setting_id` starts with "GFS"
- [ ] **Profile Generator:** Often generating profiles that work on import but show as "unsupported" after Bambu Studio restart -- test the full cycle: generate, install, close Bambu Studio, reopen, verify profile is visible
- [ ] **Filament Scraper:** Often extracting a single temperature value when Bambu Studio needs a range (low, recommended, high) -- verify all three temperature values are populated
- [ ] **AI Analysis:** Often detecting defects but not mapping them to Bambu Studio's specific parameter names -- verify recommendations use exact Bambu Studio JSON field names (e.g., `filament_retraction_length`, not `retraction_distance`)
- [ ] **AI Analysis:** Often providing recommendations without considering the user's current settings -- verify the analysis includes current values in the prompt context
- [ ] **Profile Installation:** Often writing the file but not updating the directory index -- verify the profile appears in Bambu Studio's UI, not just exists on disk
- [ ] **CLI Launch:** Often launching Bambu Studio but not with the correct profile active -- verify the launched instance actually uses the specified filament profile, not the default
- [ ] **Cross-Platform:** Often working on macOS but failing on Windows due to path separators or AppData vs Application Support differences -- test on both platforms before claiming cross-platform support

## Recovery Strategies

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| Bambu Studio update breaks installed profiles | LOW | Re-run `bambumate profiles install` to regenerate from BambuMate's canonical data. User loses no data because BambuMate is the source of truth. |
| Cloud sync overwrites BambuMate's profiles | LOW | Re-run profile install command. Optionally, provide guidance on disabling cloud sync for BambuMate-managed profiles. |
| AI recommends wrong settings, user applies them | MEDIUM | `bambumate profiles restore` reverts to the pre-change backup. If no backup was made, user must manually revert. This is why backups before modification are critical. |
| Scraper extracts wrong filament data | LOW | User corrects via `bambumate filament edit` or re-runs scraper with `--force-refresh`. Cached data is updated. |
| Profile JSON format changes in new Bambu Studio version | MEDIUM | Update BambuMate's profile generation templates. Requires a BambuMate release. Mitigated by reading system profiles at runtime rather than hardcoding the schema. |
| Writing profiles while Bambu Studio was running causes corruption | HIGH | User may need to delete and recreate affected profiles in Bambu Studio. BambuMate should never cause this if process detection is implemented. |

## Pitfall-to-Phase Mapping

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| Bambu Studio updates break profiles | Phase 1: Profile Generator | Test generated profiles across 2+ Bambu Studio versions. `bambumate profiles check` command passes. |
| Undocumented JSON format | Phase 1: Profile Generator | Golden-file test suite. Profiles import successfully in Bambu Studio and appear in UI. |
| Cloud sync conflicts | Phase 1: Profile Installation | Test with cloud sync enabled. Profile survives Bambu Studio restart + sync cycle. |
| Ambiguous defect-to-settings mapping | Phase 2: AI Analysis | User acceptance testing. Recommendations include ranked alternatives. Applied changes improve print quality >60% of the time. |
| Fragile web scraping | Phase 1: Filament Scraper | Extracted data for top 10 filaments matches manufacturer-published specs. Validation rejects out-of-range values. |
| Concurrent file access | Phase 1: Profile Installation | Process detection works on macOS and Windows. Atomic write confirmed. Warning shown when Bambu Studio is running. |
| API key security | Phase 1: Configuration | Keys stored in OS keychain. `bambumate doctor` confirms no keys in plain-text config files. |
| AI hallucination in data extraction | Phase 1: Filament Scraper | Validation layer catches 100% of out-of-range values. Manual review for first 20 filaments confirms accuracy. |
| AI photo analysis accuracy | Phase 2: AI Analysis | Confidence scores included. User confirmation required. Iterative refinement loop implemented. |
| Cross-platform path handling | Phase 1: Foundation | CI tests on macOS + Windows. Profile installation works on both platforms. |

## Sources

- [Bambu Studio profile directory locations (community forum)](https://forum.bambulab.com/t/where-are-the-files-for-user-filament-and-process-profiles-located/7579)
- [Cloud sync overwriting local profiles -- root cause and fix (community forum)](https://forum.bambulab.com/t/cloud-sync-overwriting-local-profiles-root-cause-and-fix/207065)
- [Bambu Studio CLI cannot load settings from JSON (GitHub issue #2889)](https://github.com/bambulab/BambuStudio/issues/2889)
- [System presets override vs overwrite (GitHub issue #2831)](https://github.com/bambulab/BambuStudio/issues/2831)
- [All printer profiles missing on upgrade (GitHub issue #1171)](https://github.com/bambulab/BambuStudio/issues/1171)
- [Custom filament profiles listed as unsupported (GitHub issue #8988)](https://github.com/bambulab/BambuStudio/issues/8988)
- [User presets disappear after update (GitHub issue #4071)](https://github.com/bambulab/BambuStudio/issues/4071)
- [How to submit filament preset (Bambu Lab Wiki)](https://wiki.bambulab.com/en/bambu-studio/submit-preset)
- [Custom filament problems and solutions (Bambu Lab Wiki)](https://wiki.bambulab.com/en/software/bambu-studio/custom-filament-issue)
- [Bambu Studio CLI command line usage (GitHub Wiki)](https://github.com/bambulab/BambuStudio/wiki/Command-Line-Usage)
- [Bambu Studio profile confusion and clarity (community forum)](https://forum.bambulab.com/t/bambu-studio-profile-confusion-and-some-clarity/124700)
- [Obico failure detection false alarms (Obico docs)](https://www.obico.io/docs/user-guides/failure-detection-false-alarms/)
- [Obico AI failure detection in 3D printing (Obico blog)](https://www.obico.io/blog/ai-failure-detection-in-3d-printing/)
- [3D printing defect troubleshooting (All3DP)](https://all3dp.com/1/common-3d-printing-problems-troubleshooting-3d-printer-issues/)
- [BambuStudio resources/profiles/BBL.json (GitHub source)](https://github.com/bambulab/BambuStudio/blob/master/resources/profiles/BBL.json)
- [BambuStudioFilamentLibrary (community project)](https://github.com/dgauche/BambuStudioFilamentLibrary)
- [Bambu Studio exporting filament profiles (Bambu Lab Wiki)](https://wiki.bambulab.com/en/bambu-studio/export-filament)
- [Rust web scraping in 2026 (ZenRows)](https://www.zenrows.com/blog/rust-web-scraping)
- [Web scraping with Rust (BrightData)](https://brightdata.com/blog/how-tos/web-scraping-with-rust)
- [OrcaSlicer vs Bambu Studio compatibility (Obico)](https://www.obico.io/blog/orca-slicer-vs-bambu-studio/)

---
*Pitfalls research for: Bambu Studio companion CLI tool (Rust) -- filament scraping, AI print analysis, slicer profile generation*
*Researched: 2026-02-04*
