# Page Redesign Wave 3: Analyze, Settings, Setup, Printer and Slice

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish piece 2 of the redesign. This wave covers the pages waves 1 and 2 left alone: Analyze, with the four severity tiers made distinct; the Settings hub's General, Health and About tabs; the first-run setup wizard; and touch-ups to Printer and Slice.

**Architecture:**
- Each page's decisions go in a small pure module with unit tests. The views compose wave 1 and 2's patterns.
- Data calls, handlers and stored values are unchanged.
- Page `<h2>`s go:
  - Hub tabs already get an h1 from the hub header.
  - Analyze, Printer and Slice aren't hubs, so they render their own `PageHeader`, which provides their h1.
- Spinners and coloured message boxes become `StateText`.
- Only one backend-facing behaviour is new: Printer keeps a session-only list of HMS codes it has seen, in the frontend.

**Tech Stack:** Rust, Leptos 0.8 CSR (WASM, built by Trunk), Playwright WebKit/Chromium tests in `tests/webkit/`.

**Spec:** `docs/superpowers/specs/2026-10-17-page-redesign-design.md`. Wave 3 covers the §4 rows for Analyze and the three Settings tabs (amended 2026-10-22), the severity colours, §5 (setup wizard), and the Printer and Slice touch-ups in §4.

## Global Constraints

- **Read-only printer.** Nothing in this piece sends a command to the printer.
- **Tokens only.** New CSS uses the `--nd-*` tokens from `style/tokens.css`. No new colours are introduced. Rewritten rule blocks drop legacy `--bg-*` / `--text-*` aliases.
- **Both themes are first-class.** Every new text/background pair passes `contrast.mjs` in dark and light: 4.5:1 for text, 3:1 for disabled text.
- **Font budget per screen.** At most two families (Space Grotesk and Space Mono), plus Doto for the single hero. At most three sizes and two weights.
- **Existing behaviour is kept.** Every feature on these pages is still reachable. Nothing is removed. The setup wizard's steps, logic and stored values are unchanged.
- **Honest copy.** No UI claims something it can't know. Examples: API keys are optional, so they never count as Health issues; there's no quality "score", because the analysis returns a word.
- **Repo rules:**
  - Never commit `.omc/`, `.claude/`, `.superpowers/`, `dist/` or test PNGs.
  - Never run a bare `git stash`.
  - No attribution lines in commits.
  - Tests never launch the Bambu Studio GUI or call `open`.
  - The Claude product is "Claude Agent".

## Commands used throughout

Run all of these from the repository root:

| What | Command |
|---|---|
| Frontend unit tests | `cargo test --bin bambumate` |
| Frontend WASM build check | `cargo check --target wasm32-unknown-unknown` |
| Format check | `cargo fmt --check` |
| Frontend bundle | `trunk build` (the browser tests need `dist/`) |
| Browser tests | `(cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)` |

- Format single files with `rustfmt --edition 2021 <file>`, never `cargo fmt -- <file>`.
- `tests/webkit/app-flows.mjs` defines `goTo(route, expect?)`, `openTab(hub, tab)`, `setFixture`, `withFixtures` and `callsOf` near the top of `driveApp`.
- Use `textContent`, not `innerText`, for anything uppercased by CSS.

## What waves 1 and 2 provide (on `main`)

All of these live in `src/components/patterns.rs`:
- `PageHeader(title, tabs?, children?)`, which renders the h1.
- `Hero(label, value, unit?, detail?, tone?)`.
- `HeroStats`.
- `SegmentedBar(percent, total, label)`.
- `StatRow(label, value)`.
- `StateText(kind, message?)`.
- `StepStrip`.
- `Tone::{Neutral, Signal, Warning, Accent}`.

Also available:
- CSS: `.nd-table`, `.nd-num`, `.nd-label`, `.nd-visually-hidden`.
- `segments_filled(percent, total)`.

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/components/severity.rs` | new | `SeverityTier`, `severity_tier(f32)`, `tier_label`, `tier_key` |
| `src/pages/analysis_view.rs` | new | `quality_tone`, `quality_word`, `defects_by_severity`, `analysis_detail` |
| `src/components/defect_report.rs` + `.css` | modified | Severity badges from `severity_tier`; numbered defects; `nd-*` tokens |
| `src/pages/print_analysis.rs` + `.css` | modified | `PageHeader`, quality hero beside the photo, `StateText` |
| `src/pages/health_view.rs` | new | `HealthSummary`, `health_summary` |
| `src/components/health_hero.rs` | new | `HealthHero`, which runs the check and shows the summary (used by General and Health) |
| `src/pages/health.rs`, `src/pages/settings.rs`, `style/main.css` | modified | Health tab and General tab |
| `src/pages/about.rs`, `style/main.css` | modified | About tab |
| `src/pages/setup_wizard.rs`, `style/main.css` | modified | Stepped full-screen flow |
| `src/printer/hms_history.rs` | new | `SeenError`, `merge_seen` |
| `src/printer/mod.rs`, `src/pages/printer.rs`, `style/printer.css` | modified | `PageHeader`, Doto percent, `SegmentedBar`, "Earlier this session" HMS list |
| `src/pages/slice.rs`, `style/slice.css` | modified | `PageHeader`, Doto hero, `SegmentedBar`, `.nd-table` compare |
| `tests/webkit/contrast.mjs`, `tests/webkit/app-flows.mjs` | modified | New pairs and steps |

---

### Task 1: Severity tiers and the Analyze page

**Files:**
- Create: `src/components/severity.rs`, `src/pages/analysis_view.rs`
- Modify: `src/components/mod.rs`, `src/pages/mod.rs`, `src/components/defect_report.rs`, `src/components/defect_report.css`, `src/pages/print_analysis.rs`, `src/pages/print_analysis.css`, `tests/webkit/contrast.mjs`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Produces:
  - `severity::SeverityTier { Minor, Noticeable, Significant, Severe }`;
  - `severity::severity_tier(severity: f32) -> SeverityTier`;
  - `SeverityTier::label() -> &'static str` and `SeverityTier::key() -> &'static str`;
  - `analysis_view::{quality_tone(&str) -> Tone, quality_word(&str) -> String, defects_by_severity(&[DetectedDefect]) -> Vec<DetectedDefect>, analysis_detail(&DefectReport, &str) -> String}`.

- [ ] **Step 1: Write the failing tests**

Create `src/components/severity.rs` with only the tests module, and add `pub mod severity;` to `src/components/mod.rs` in alphabetical order:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_split_at_the_existing_thresholds() {
        assert_eq!(severity_tier(0.0), SeverityTier::Minor);
        assert_eq!(severity_tier(0.29), SeverityTier::Minor);
        assert_eq!(severity_tier(0.3), SeverityTier::Noticeable);
        assert_eq!(severity_tier(0.49), SeverityTier::Noticeable);
        assert_eq!(severity_tier(0.5), SeverityTier::Significant);
        assert_eq!(severity_tier(0.69), SeverityTier::Significant);
        assert_eq!(severity_tier(0.7), SeverityTier::Severe);
        assert_eq!(severity_tier(1.0), SeverityTier::Severe);
    }

    #[test]
    fn every_tier_has_its_own_key_and_label() {
        let tiers = [SeverityTier::Minor, SeverityTier::Noticeable, SeverityTier::Significant, SeverityTier::Severe];
        assert_eq!(tiers.map(SeverityTier::key), ["minor", "noticeable", "significant", "severe"]);
        assert_eq!(tiers.map(SeverityTier::label), ["Minor", "Noticeable", "Significant", "Severe"]);
    }
}
```

Create `src/pages/analysis_view.rs` with only the tests module, and add `pub mod analysis_view;` to `src/pages/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn defect(t: &str, s: f32) -> DetectedDefect {
        DetectedDefect { defect_type: t.into(), severity: s, confidence: 0.9 }
    }

    #[test]
    fn quality_words_are_toned() {
        assert_eq!(quality_tone("excellent"), Tone::Signal);
        assert_eq!(quality_tone("good"), Tone::Signal);
        assert_eq!(quality_tone("acceptable"), Tone::Warning);
        assert_eq!(quality_tone("fair"), Tone::Warning);
        assert_eq!(quality_tone("poor"), Tone::Accent);
        assert_eq!(quality_tone("failed"), Tone::Accent);
        assert_eq!(quality_tone("whatever"), Tone::Neutral);
        assert_eq!(quality_word(" fair "), "FAIR");
        assert_eq!(quality_word(""), "—");
    }

    #[test]
    fn defects_are_ordered_most_severe_first() {
        let sorted = defects_by_severity(&[defect("a", 0.35), defect("b", 0.65), defect("c", 0.1)]);
        let order: Vec<&str> = sorted.iter().map(|d| d.defect_type.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
    }

    #[test]
    fn the_detail_counts_defects_and_names_the_material() {
        let report = DefectReport { defects: vec![defect("a", 0.5), defect("b", 0.2)], overall_quality: "fair".into(), notes: None };
        assert_eq!(analysis_detail(&report, "PLA"), "2 defects · PLA");
        let one = DefectReport { defects: vec![defect("a", 0.5)], ..report.clone() };
        assert_eq!(analysis_detail(&one, ""), "1 defect");
        let none = DefectReport { defects: vec![], ..report };
        assert_eq!(analysis_detail(&none, "PETG"), "No defects · PETG");
    }
}
```

Run `cargo test --bin bambumate -- severity analysis_view`.
Expected: compile errors.

- [ ] **Step 2: Implement the helpers**

Above the tests in `src/components/severity.rs`:

```rust
//! The four defect severity tiers (spec 2026-10-17 §4). Distinct in colour
//! and form: severe is filled accent, significant accent text, noticeable
//! warning text, minor secondary text. Blue stays for links.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeverityTier {
    Minor,
    Noticeable,
    Significant,
    Severe,
}

impl SeverityTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Minor => "Minor",
            Self::Noticeable => "Noticeable",
            Self::Significant => "Significant",
            Self::Severe => "Severe",
        }
    }

    /// `severity-<key>` class and `data-tier`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Noticeable => "noticeable",
            Self::Significant => "significant",
            Self::Severe => "severe",
        }
    }
}

/// The thresholds `defect_report::severity_label` used before.
pub fn severity_tier(severity: f32) -> SeverityTier {
    if severity < 0.3 {
        SeverityTier::Minor
    } else if severity < 0.5 {
        SeverityTier::Noticeable
    } else if severity < 0.7 {
        SeverityTier::Significant
    } else {
        SeverityTier::Severe
    }
}
```

Above the tests in `src/pages/analysis_view.rs`:

```rust
//! What Analyze leads with (spec 2026-10-17 §4, amended 2026-10-22): the
//! overall quality word, toned; the analysis returns a word, not a score,
//! and no defect positions.

use crate::components::patterns::Tone;
pub use crate::pages::print_analysis::{DefectReport, DetectedDefect};

pub fn quality_tone(quality: &str) -> Tone {
    match quality.trim().to_ascii_lowercase().as_str() {
        "excellent" | "good" => Tone::Signal,
        "acceptable" | "fair" => Tone::Warning,
        "poor" | "failed" => Tone::Accent,
        _ => Tone::Neutral,
    }
}

pub fn quality_word(quality: &str) -> String {
    let w = quality.trim();
    if w.is_empty() {
        "—".into()
    } else {
        w.to_uppercase()
    }
}

/// Most severe first; ties keep their order.
pub fn defects_by_severity(defects: &[DetectedDefect]) -> Vec<DetectedDefect> {
    let mut sorted = defects.to_vec();
    sorted.sort_by(|a, b| b.severity.partial_cmp(&a.severity).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

/// `2 defects · PLA`, `1 defect`, `No defects · PETG`.
pub fn analysis_detail(report: &DefectReport, material: &str) -> String {
    let count = match report.defects.len() {
        0 => "No defects".to_string(),
        1 => "1 defect".to_string(),
        n => format!("{n} defects"),
    };
    let material = material.trim();
    if material.is_empty() {
        count
    } else {
        format!("{count} · {material}")
    }
}
```

`DefectReport` and `DetectedDefect` need `Clone`. They already derive it.

Run `cargo test --bin bambumate -- severity analysis_view`.
Expected: 5 passed.

- [ ] **Step 3: Severity tiers in the defect report**

In `src/components/defect_report.rs`:

(a) Delete the old `severity_label` function. `DefectCard` gains a leading `number: usize` prop and becomes:

```rust
#[component]
fn DefectCard(number: usize, defect_type: String, severity: f32, confidence: f32) -> impl IntoView {
    let tier = severity_tier(severity);
    view! {
        <li class="defect-card" data-tier=tier.key()>
            <span class="defect-number nd-mono">{format!("{number:02}")}</span>
            <span class="defect-name">{defect_display_name(&defect_type)}</span>
            <span class=format!("severity-badge severity-{}", tier.key())>{tier.label()}</span>
            <span class="defect-meta nd-mono">
                {format!("Severity {:.0}% · confidence {:.0}%", severity * 100.0, confidence * 100.0)}
            </span>
        </li>
    }
}
```

   Add `use crate::components::severity::severity_tier;`.

(b) In `DefectReportDisplay`:
- Remove the `quality-section` block, because the page hero shows the quality now. If the notes exist, keep them in a `<p class="quality-notes">` at the top of the defects section.
- Render the list as `<ol class="defects-list">`, using `defects_by_severity(&defect_report.defects)` and `.enumerate()` with `number = i + 1`.
- The section heading becomes `<h2 class="nd-label">"Defects"</h2>`. Turn the other `<h3>`s in this component into `<h2 class="nd-label">` too.
- Keep the empty message "No defects detected - your print looks great!" as `<StateText kind=StateKind::Empty message="No defects detected" />`.
- Delete `QualityBadge` if it is now unused.

(c) In `src/components/defect_report.css`:
- Replace the `.severity-*` rules with:

```css
.severity-badge {
    padding: 2px 10px;
    border-radius: var(--nd-radius-pill);
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border: 1px solid var(--nd-border-visible);
}

.severity-minor {
    color: var(--nd-text-secondary);
}

.severity-noticeable {
    color: var(--nd-warning);
}

.severity-significant {
    color: var(--nd-accent);
    border-color: var(--nd-accent);
}

.severity-severe {
    color: var(--nd-black);
    background-color: var(--nd-accent);
    border-color: var(--nd-accent);
}

.defects-list {
    list-style: none;
    display: flex;
    flex-direction: column;
}

.defect-card {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 2px var(--nd-space-md);
    align-items: baseline;
    padding: var(--nd-space-sm) 0;
    border-bottom: 1px solid var(--nd-border);
}

.defect-number {
    color: var(--nd-text-secondary);
}

.defect-meta {
    grid-column: 2 / -1;
    font-size: 11px;
    color: var(--nd-text-secondary);
}
```

- Remove the rules for classes that no longer exist (`.quality-*`, `.defect-header`, `.defect-details`, `.detail-row`, `.detail-label`, `.severity-bar`, `.severity-fill`, `.detail-value`), checking each with `grep -rn` across `src/`.
- Change any remaining legacy `var(--bg-*)` / `var(--text-*)` in the rules you touch to the matching `--nd-*` token.

(d) In `tests/webkit/contrast.mjs`, add the severe badge to `FILLED`:

```js
  ["--nd-black", "--nd-accent", 4.5],
```

- [ ] **Step 4: The Analyze page**

In `src/pages/print_analysis.rs`:

(a) Add these imports:

```rust
use crate::components::patterns::{Hero, PageHeader, StateKind, StateText};
use crate::pages::analysis_view::{analysis_detail, quality_tone, quality_word};
```

(b) Replace `<h2>"Print Analysis"</h2>` with `<PageHeader title="Analyze" />`.

   In the AI-required fallback, keep the text but replace `<h3>"AI Required"</h3>` with `<p class="nd-label">"AI required"</p>`. Delete the `page-description` paragraph.

(c) In `AnalysisState::Analyzing`, replace the `loading-indicator` div with:

```rust
                            <StateText kind=StateKind::Loading message="Analyzing your print — this may take 10–30 seconds" />
```

   In `PhotoDropZone`, replace the loading content with `<StateText kind=StateKind::Loading message="Loading image" />`.

(d) In `AnalysisState::Complete`, replace the small preview image and the start of the results with a hero row, keeping the `.analysis-results` hook:

```rust
                            <div class="analysis-results">
                                <div class="an-hero-row">
                                    <Hero
                                        label="Overall quality"
                                        value=quality_word(&response.defect_report.overall_quality)
                                        detail=analysis_detail(&response.defect_report, &response.material_type)
                                        tone=quality_tone(&response.defect_report.overall_quality)
                                    />
                                    {move || image_preview.get().map(|src| view! {
                                        <img src=src class="preview-image an-photo" alt="Analyzed print" />
                                    })}
                                </div>
```

   The apply message, `DefectReportDisplay`, the buttons, the dialog and the history all follow unchanged.

(e) In `AnalysisState::Error`, replace the `error-message` block with `<StateText kind=StateKind::Error message=msg />`, followed by the existing "Try Again" button.

(f) In `src/pages/print_analysis.css`:
- Add:

```css
.an-hero-row {
    display: grid;
    grid-template-columns: 1fr minmax(180px, 280px);
    gap: var(--nd-space-xl);
    align-items: start;
    margin-bottom: var(--nd-space-xl);
}

.an-photo {
    width: 100%;
    border-radius: var(--nd-radius-card);
    border: 1px solid var(--nd-border);
}

@media (max-width: 900px) {
    .an-hero-row {
        grid-template-columns: 1fr;
    }
}
```

- Remove the rules for `.loading-indicator`, `.spinner`, `.error-message`, `.error-state h3` and `.preview-image.small`, if `grep -rn` shows they're unused.

- [ ] **Step 5: Browser steps**

In `tests/webkit/app-flows.mjs`, extend "analysis runs and reports defects". After `.analysis-results` appears, add:

```js
      const word = (await page.locator(".an-hero-row .nd-hero-number").textContent()).trim();
      if (word !== "FAIR") throw new Error(`quality reads "${word}"`);
      const tone = await page.locator(".an-hero-row .nd-hero").getAttribute("data-tone");
      if (tone !== "warning") throw new Error(`quality tone ${tone}`);
      const order = await page.locator(".defects-list .defect-card").evaluateAll((els) => els.map((e) => e.dataset.tier));
      if (order.join(",") !== "significant,noticeable") throw new Error(`defects in order ${order}`);
      const first = (await page.locator(".defects-list .defect-number").first().textContent()).trim();
      if (first !== "01") throw new Error(`first defect numbered ${first}`);
```

The fixture is `overall_quality: "fair"` with defects at 0.65 and 0.35. Keep the step's existing `/string/i` check.

- [ ] **Step 6: Run everything**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo fmt --check && trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: all pass in both engines. contrast shows the new `--nd-black on filled --nd-accent` lines as OK.

- [ ] **Step 7: Commit**

```bash
git add src/components/severity.rs src/components/mod.rs src/pages/analysis_view.rs src/pages/mod.rs src/components/defect_report.rs src/components/defect_report.css src/pages/print_analysis.rs src/pages/print_analysis.css tests/webkit/contrast.mjs tests/webkit/app-flows.mjs
git commit -m "feat(analyze): quality hero beside the photo; four distinct severity tiers; numbered defects"
```

---

### Task 2: Settings · Health and General

**Files:**
- Create: `src/pages/health_view.rs`, `src/components/health_hero.rs`
- Modify: `src/pages/mod.rs`, `src/components/mod.rs`, `src/pages/health.rs`, `src/pages/settings.rs`, `style/main.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes: `commands::HealthReport { bambu_studio_installed, bambu_studio_path, profile_dir_accessible, profile_dir_path, claude_api_key_set, openai_api_key_set, kimi_api_key_set, openrouter_api_key_set }`, from `commands::run_health_check()`.
- Produces:
  - `health_view::HealthSummary { word: String, tone: Tone, issues: usize, keys_set: usize }`
  - `health_view::health_summary(&HealthReport) -> HealthSummary`
  - `health_view::health_detail(&HealthSummary, ago: &str) -> String`
  - `health_hero::HealthHero()`, which runs `run_health_check` on mount and renders a `Hero` with label "System".

- [ ] **Step 1: Write the failing tests**

Create `src/pages/health_view.rs` with only the tests, and add `pub mod health_view;` to `src/pages/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn report(bs: bool, dir: bool, keys: [bool; 4]) -> HealthReport {
        HealthReport {
            bambu_studio_installed: bs,
            bambu_studio_path: None,
            profile_dir_accessible: dir,
            profile_dir_path: None,
            claude_api_key_set: keys[0],
            openai_api_key_set: keys[1],
            kimi_api_key_set: keys[2],
            openrouter_api_key_set: keys[3],
        }
    }

    #[test]
    fn only_required_checks_count_as_issues() {
        let s = health_summary(&report(true, true, [false; 4]));
        assert_eq!((s.word.as_str(), s.tone, s.issues, s.keys_set), ("ALL GOOD", Tone::Signal, 0, 0));
        let s = health_summary(&report(false, true, [true, false, false, false]));
        assert_eq!((s.word.as_str(), s.tone, s.issues, s.keys_set), ("1 ISSUE", Tone::Accent, 1, 1));
        let s = health_summary(&report(false, false, [true; 4]));
        assert_eq!((s.word.as_str(), s.issues, s.keys_set), ("2 ISSUES", 2, 4));
    }

    #[test]
    fn the_detail_says_what_was_checked() {
        let s = health_summary(&report(true, true, [true, false, false, false]));
        assert_eq!(health_detail(&s, "just now"), "2 required checks · 1 API key set · last run just now");
        let s = health_summary(&report(true, true, [true, true, false, false]));
        assert_eq!(health_detail(&s, "5m ago"), "2 required checks · 2 API keys set · last run 5m ago");
    }
}
```

Run `cargo test --bin bambumate health_view`.
Expected: compile errors.

- [ ] **Step 2: Implement**

Above the tests in `src/pages/health_view.rs`:

```rust
//! The Health summary that leads Settings (spec 2026-10-17 §4, amended
//! 2026-10-22). Only Bambu Studio and the profile folder are required; API
//! keys are optional (one provider is enough, a local model needs none), so
//! they're counted but never an issue.

use crate::commands::HealthReport;
use crate::components::patterns::Tone;

/// Checks that must pass.
const REQUIRED: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct HealthSummary {
    pub word: String,
    pub tone: Tone,
    pub issues: usize,
    pub keys_set: usize,
}

pub fn health_summary(r: &HealthReport) -> HealthSummary {
    let issues = [r.bambu_studio_installed, r.profile_dir_accessible]
        .iter()
        .filter(|ok| !**ok)
        .count();
    let keys_set = [r.claude_api_key_set, r.openai_api_key_set, r.kimi_api_key_set, r.openrouter_api_key_set]
        .iter()
        .filter(|k| **k)
        .count();
    let (word, tone) = match issues {
        0 => ("ALL GOOD".to_string(), Tone::Signal),
        1 => ("1 ISSUE".to_string(), Tone::Accent),
        n => (format!("{n} ISSUES"), Tone::Accent),
    };
    HealthSummary { word, tone, issues, keys_set }
}

pub fn health_detail(s: &HealthSummary, ago: &str) -> String {
    let keys = if s.keys_set == 1 { "1 API key set".to_string() } else { format!("{} API keys set", s.keys_set) };
    format!("{REQUIRED} required checks · {keys} · last run {ago}")
}
```

If `HealthReport` doesn't derive `Clone` and `Debug`, add both in `src/commands.rs`.

Run `cargo test --bin bambumate health_view`.
Expected: 2 passed.

- [ ] **Step 3: `HealthHero`**

Create `src/components/health_hero.rs` and add `pub mod health_hero;` to `src/components/mod.rs`:

```rust
//! The Health summary as a hero: runs the health check on mount and leads
//! Settings · General and Settings · Health with it.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, HealthReport};
use crate::components::patterns::{Hero, StateKind, StateText, Tone};
use crate::pages::home_state::format_ago;
use crate::pages::health_view::{health_detail, health_summary};

#[component]
pub fn HealthHero(
    /// The report, shared with the page so it can list each check.
    #[prop(optional)]
    report: Option<RwSignal<Option<HealthReport>>>,
) -> impl IntoView {
    let report = report.unwrap_or_else(|| RwSignal::new(None));
    let error = RwSignal::new(None::<String>);
    let ran_at = RwSignal::new(None::<f64>);
    spawn_local(async move {
        match commands::run_health_check().await {
            Ok(r) => {
                report.try_set(Some(r));
                ran_at.try_set(Some(js_sys::Date::now()));
            }
            Err(e) => {
                error.try_set(Some(e));
            }
        }
    });
    view! {
        <div class="health-hero">
            {move || match (report.get(), error.get()) {
                (Some(r), _) => {
                    let s = health_summary(&r);
                    let ago = ran_at.get().map(|t| format_ago(js_sys::Date::now(), t)).unwrap_or_default();
                    view! {
                        <Hero label="System" value=s.word.clone() detail=health_detail(&s, &ago) tone=s.tone />
                    }.into_any()
                }
                (None, Some(e)) => view! { <StateText kind=StateKind::Error message=format!("Health check failed: {e}") /> }.into_any(),
                (None, None) => view! {
                    <Hero label="System" value="…" tone=Tone::Neutral />
                }.into_any(),
            }}
        </div>
    }
}
```

`format_ago` lives in `src/pages/home_state.rs` and is `pub`.

- [ ] **Step 4: Health tab**

In `src/pages/health.rs`, keep the Run button, the error handling, the six `StatusBadge` rows and `DiagnosticsPanel`, but:
- Delete the `<h2>` and the description.
- Make the page's `report` an `RwSignal<Option<HealthReport>>`, and pass it to `<HealthHero report=report />` placed first on the page. The hero then runs the check on mount, and the page's own auto-run `Effect` goes. The "Run Health Check" button keeps calling `run_health_check` and setting the same signal, so the hero updates. The hero's "last run" covers only its own first run; that's acceptable.
- Delete the old `health-summary` line. Keep the `.health-results` and `.health-item` hooks.
- Inside `.health-results`, label the API key rows `"Claude API key (optional)"`, `"OpenAI API key (optional)"` and so on, so the list matches the hero's rule.

In the browser step "navigate to Health Check", the step currently returns the old `.health-summary` text. Replace that with:

```js
    const word = (await page.locator(".health-hero .nd-hero-number").textContent()).trim();
    if (!/^(ALL GOOD|\d+ ISSUES?)$/.test(word)) throw new Error(`health hero reads "${word}"`);
    return word;
```

Keep the existing `health-item` count assertion of 6.

- [ ] **Step 5: General tab**

In `src/pages/settings.rs`:
- Delete `<h2>"Settings"</h2>`.
- Add `<HealthHero />` as the first child of the page (`use crate::components::health_hero::HealthHero;`).
- Change every section heading `<h3>…</h3>` directly inside a `settings-section` to `<h2 class="nd-label settings-section-title">…</h2>`, keeping its text. The `<h4>` titles inside the mode cards stay.

In `style/main.css`, restyle the sections so they read as label/value groups instead of boxed cards:
- `.settings-section`: remove `background`, `border`, `border-radius`, `padding` and `box-shadow`. Use `padding: var(--nd-space-xl) 0; border-top: 1px solid var(--nd-border);`.
- `.settings-section-danger`: keep its danger cue as `border-top-color: var(--nd-accent)`, with no fill.
- Replace `.settings-section h3` with `.settings-section-title { margin-bottom: var(--nd-space-sm); }`. `.page h2` has lower specificity than `.settings-section .settings-section-title`, so write it that way if needed to win.
- `.section-description`: `color: var(--nd-text-secondary)`.

`PrinterSettings`, `SlicerSettingsSection` and the API key forms stay as they are. They pick up the restyle through the shared section class.

Add a browser step after "navigation leaves the new page's left controls clickable":

```js
  await step(run, page, "Settings leads with the Health summary", async () => {
    await page.click('nav.sidebar a[href="/settings"]');
    await page.waitForSelector(".settings-page .health-hero .nd-hero-number", { timeout: 15000 });
    const word = (await page.locator(".settings-page .health-hero .nd-hero-number").textContent()).trim();
    if (!/^(ALL GOOD|\d+ ISSUES?)$/.test(word)) throw new Error(`hero reads "${word}"`);
    const h2s = await page.locator(".settings-page h2.settings-section-title").count();
    if (h2s < 4) throw new Error(`${h2s} section titles`);
  });
```

`run_health_check` must have a fixture; check `tests/webkit/fixtures.mjs` and add one if it's missing, with all six fields.

- [ ] **Step 6: Run everything and commit**

Run the full command list. All must pass in both engines.

```bash
git add src/pages/health_view.rs src/pages/mod.rs src/components/health_hero.rs src/components/mod.rs src/pages/health.rs src/pages/settings.rs src/commands.rs style/main.css tests/webkit/app-flows.mjs tests/webkit/fixtures.mjs
git commit -m "feat(settings): Health summary leads General and Health; sections become label groups"
```

---

### Task 3: Settings · About

**Files:**
- Modify: `src/pages/about.rs`, `style/main.css`, `tests/webkit/app-flows.mjs`

- [ ] **Step 1: Rebuild the view**

In `src/pages/about.rs`, keep all signals and handlers (version load, update check, open releases, open links). Replace the `view!` body with:

```rust
    view! {
        <div class="page about-page">
            <Hero
                label="About"
                value="BambuMate"
                detail=Signal::derive(move || format!("v{}", current_version.get()))
            />
            <div class="about-rows">
                <StatRow label="Current version" value=Signal::derive(move || format!("v{}", current_version.get())) />
                {move || update_info.get().map(|info| view! {
                    <StatRow label="Latest release" value=format!("v{}", info.latest_version) />
                })}
            </div>
            // The update-available block, "You're up to date!" (class about-up-to-date),
            // the check error and the two update buttons: keep their current markup and
            // handlers, moved here unchanged.
            <div class="about-links">
                <h2 class="nd-label">"Links"</h2>
                // the existing <ul class="about-link-list">…</ul>, unchanged
            </div>
        </div>
    }
```

- Add `use crate::components::patterns::{Hero, StatRow};`.
- The two comment lines mark where to move the existing blocks verbatim. The update block keeps its own `<span class="about-version-value">` for the latest version.
- Update the browser step "navigate to About" to read the current version from `.about-rows .nd-stat-row-value` (the first one) instead of `.about-version-value`, and expect it to include `1.3.0` (the `get_app_version` fixture).
- `BrandMark` and the subtitle go. If `BrandMark` becomes unused anywhere (`grep -rn BrandMark src`), delete it from `src/components/branding.rs`.
- The hero's Doto value "BambuMate" is the app name. `.nd-hero-number`'s clamp size fits nine characters at 1280px; check this in the screenshot.

- [ ] **Step 2: CSS**

In `style/main.css`:
- Delete the `.about-header`, `.about-logo`, `.about-title`, `.about-subtitle` and `.about-version-card` rules, plus `.about-version-row` and `.about-version-label` if unused.
- Add:

```css
.about-rows {
    margin: var(--nd-space-lg) 0;
    max-width: 480px;
}

.about-links {
    margin-top: var(--nd-space-xl);
}
```

- Swap legacy tokens in the remaining `.about-*` rules for `--nd-*` tokens.
- Remove `margin: 0 auto` from `.about-page`, so it lines up with the hub header (wave 2 did the same for the other pages).

- [ ] **Step 3: Run everything and commit**

Run the full command list. All must pass in both engines.

```bash
git add src/pages/about.rs src/components/branding.rs style/main.css tests/webkit/app-flows.mjs
git commit -m "feat(settings): About leads with the app name; version and links as rows"
```

---

### Task 4: Setup wizard as a stepped full-screen flow

**Files:**
- Modify: `src/pages/setup_wizard.rs`, `style/main.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Produces: `fn wizard_counter(step: usize, total: usize) -> String`, returning `"02 / 05"` for step 1, in `setup_wizard.rs` with a unit test.

- [ ] **Step 1: The counter helper (TDD)**

Add a tests module to `src/pages/setup_wizard.rs`, if it doesn't have one, with:

```rust
    #[test]
    fn the_counter_is_two_digit_and_one_based() {
        assert_eq!(wizard_counter(0, 5), "01 / 05");
        assert_eq!(wizard_counter(4, 5), "05 / 05");
    }
```

Run it and see it fail. Then add, matching the type of `step`'s value:

```rust
/// Steps in the wizard (0-4).
const WIZARD_STEPS: usize = 5;

/// `02 / 05` for step index 1.
fn wizard_counter(step: usize, total: usize) -> String {
    format!("{:02} / {:02}", step + 1, total)
}
```

If `step` is `u8` or another integer type, convert with `as usize` at the call site.

- [ ] **Step 2: Header and steps**

- Replace the `wizard-header`'s `<h2>"BambuMate Setup"</h2>` with `<p class="nd-label">"BambuMate setup"</p>`.
- Replace the whole `wizard-steps-indicator` (dots and lines) with:

```rust
                    <div class="wizard-progress">
                        <span class="nd-mono wizard-counter">{move || wizard_counter(step.get() as usize, WIZARD_STEPS)}</span>
                        <SegmentedBar
                            percent=Signal::derive(move || ((step.get() as u32 + 1) * 100 / WIZARD_STEPS as u32))
                            total=WIZARD_STEPS as u32
                            label="Setup progress"
                        />
                    </div>
```

  Add `use crate::components::patterns::SegmentedBar;`.
- Change each step's question heading from `<h3>…</h3>` to `<h1 class="wizard-question">…</h1>`, keeping the text. The wizard is a full-screen overlay outside the app layout, so its question is the page's h1.
- Step 3's two conditional headings ("Local Server Configuration" / "Enter API Key") stay conditional, but become h1s.
- Inner `<h4>` card titles stay.
- `wizard-error` blocks become `<StateText kind=StateKind::Error message=… />` inside a `<div class="wizard-error">`, keeping the hook.

- [ ] **Step 3: CSS**

In `style/main.css`:
- `.wizard-container`: make it fill the overlay as a stepped page. `width: min(760px, 100% - 64px); max-height: none; height: 100%; display: flex; flex-direction: column; padding: var(--nd-space-2xl) 0; background: transparent; border: 0; box-shadow: none;`. Keep the overlay's own `--nd-black` background, or set `.wizard-overlay { background: var(--nd-black); }` so it's full-screen and solid.
- `.wizard-body`: `flex: 1; overflow-y: auto;`.
- `.wizard-question`: `font-family: var(--nd-font-body); font-size: 28px; font-weight: 400; color: var(--nd-text-display); margin-bottom: var(--nd-space-md);`.
- `.wizard-progress`: `display: flex; flex-direction: column; gap: var(--nd-space-sm); margin: var(--nd-space-md) 0 var(--nd-space-xl);`.
- Delete the `.wizard-steps-indicator`, `.wizard-dot*`, `.wizard-dot-line` and `.wizard-step h3` rules, plus `.wizard-header h2` if it's now unused.
- `layout.mjs` checks that `.wizard-overlay` covers the viewport; keep its `position: fixed; inset: 0` (or equivalent).

- [ ] **Step 4: Browser step**

Find the wizard steps in `app-flows.mjs`; they use a separate `wizard` page near the end. In the first wizard step that waits for `.wizard-overlay`, add:

```js
    const counter = (await wizard.locator(".wizard-counter").textContent()).trim();
    if (counter !== "01 / 05") throw new Error(`counter reads "${counter}"`);
    if ((await wizard.locator("h1.wizard-question").count()) !== 1) throw new Error("expected one wizard question h1");
```

Adapt the variable name to whatever that section uses for its page.

- [ ] **Step 5: Run everything and commit**

Run the full command list. All must pass in both engines.

```bash
git add src/pages/setup_wizard.rs style/main.css tests/webkit/app-flows.mjs
git commit -m "feat(setup): wizard is a stepped full-screen flow with a counter and segmented progress"
```

---

### Task 5: Printer touch-ups and a session HMS history

**Files:**
- Create: `src/printer/hms_history.rs`
- Modify: `src/printer/mod.rs`, `src/pages/printer.rs`, `style/printer.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Produces:
  - `hms_history::SeenError { code: String, text: Option<String>, first_seen_ms: f64 }`
  - `hms_history::merge_seen(history: &[SeenError], active: &[ErrorView], now_ms: f64) -> Vec<SeenError>`, which keeps the newest first, adds codes not already in the history, and caps the list at 20.
  - `PrinterShared.seen_errors: RwSignal<Vec<SeenError>>`.

- [ ] **Step 1: Write the failing tests**

Create `src/printer/hms_history.rs` with the tests, and add `pub mod hms_history;` to `src/printer/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn err(code: &str) -> ErrorView {
        ErrorView { kind: "hms".into(), code: code.into(), text: Some(format!("text {code}")), wiki_url: String::new() }
    }

    #[test]
    fn new_codes_go_first_and_repeats_are_kept_once() {
        let h = merge_seen(&[], &[err("A"), err("B")], 1.0);
        assert_eq!(h.iter().map(|e| e.code.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        let h = merge_seen(&h, &[err("B"), err("C")], 2.0);
        assert_eq!(h.iter().map(|e| e.code.as_str()).collect::<Vec<_>>(), ["C", "A", "B"]);
        assert_eq!(h[0].first_seen_ms, 2.0);
        assert_eq!(h[0].text.as_deref(), Some("text C"));
    }

    #[test]
    fn the_history_is_capped() {
        let mut h = Vec::new();
        for i in 0..30 {
            h = merge_seen(&h, &[err(&format!("E{i}"))], i as f64);
        }
        assert_eq!(h.len(), 20);
        assert_eq!(h[0].code, "E29");
    }
}
```

Run `cargo test --bin bambumate hms_history`.
Expected: compile errors.

- [ ] **Step 2: Implement**

```rust
//! HMS codes the printer reported this session, newest first, so Printer
//! can show what came and went. Session only; nothing is stored.

use crate::printer::types::ErrorView;

/// Most entries kept.
const CAP: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SeenError {
    pub code: String,
    pub text: Option<String>,
    pub first_seen_ms: f64,
}

pub fn merge_seen(history: &[SeenError], active: &[ErrorView], now_ms: f64) -> Vec<SeenError> {
    // New codes from this report, in the printer's order, ahead of
    // everything older.
    let mut out: Vec<SeenError> = active
        .iter()
        .filter(|e| !history.iter().any(|h| h.code == e.code))
        .map(|e| SeenError { code: e.code.clone(), text: e.text.clone(), first_seen_ms: now_ms })
        .collect();
    out.extend(history.iter().cloned());
    out.truncate(CAP);
    out
}
```

Run `cargo test --bin bambumate hms_history`.
Expected: 2 passed.

- [ ] **Step 3: Feed it**

In `src/printer/mod.rs`:
- Add `pub seen_errors: RwSignal<Vec<hms_history::SeenError>>` to `PrinterShared`, initialised empty in `new()`.
- In `PrinterEvents`, next to the last-print effect, add:

```rust
    let active_errors = Memo::new(move |_| shared.view.with(|v| v.errors.clone()));
    Effect::new(move |_| {
        let active = active_errors.get();
        if active.is_empty() {
            return;
        }
        let now = js_sys::Date::now();
        shared.seen_errors.update(|h| *h = hms_history::merge_seen(h, &active, now));
    });
```

  `ErrorView` derives `PartialEq`, which `Memo` needs.

- [ ] **Step 4: The page**

In `src/pages/printer.rs`:
- Replace the `<header class="pr-head">…</header>` block with:

```rust
            <PageHeader title="Printer">
                {move || header.get().map(|text| view! { <span class="pr-ident nd-mono">{text}</span> })}
            </PageHeader>
```

  `PageHeader`'s children go into its actions slot; the `.pr-ident` hook stays.
- In `Hero`:
  - Give the percent span the Doto class, so it becomes `<span class="pr-hero-percent nd-display">`, keeping its text and hook.
  - Replace the `pr-progress` div with `<SegmentedBar percent=Signal::derive(move || percent.get().unwrap_or(0).min(100)) total=12 label="Print progress" />`.
- In `Errors`, after the active list, add an "Earlier this session" list from `PrinterShared.seen_errors`, showing entries whose code isn't currently active:

```rust
            {move || {
                let shared = expect_context::<PrinterShared>();
                let active: Vec<String> = errors.with(|e| e.iter().map(|e| e.code.clone()).collect());
                let earlier: Vec<_> = shared.seen_errors.with(|h| h.iter().filter(|s| !active.contains(&s.code)).cloned().collect());
                (!earlier.is_empty()).then(|| view! {
                    <div class="pr-earlier">
                        <p class="nd-label">"Earlier this session"</p>
                        <ul class="pr-earlier-list">
                            {earlier.into_iter().map(|s| view! {
                                <li class="pr-earlier-item">
                                    <span class="nd-mono">{s.code}</span>
                                    {s.text.map(|t| view! { <span class="pr-earlier-text">{t}</span> })}
                                </li>
                            }).collect::<Vec<_>>()}
                        </ul>
                    </div>
                })
            }}
```

  Call `expect_context` once, outside the closure, and copy the signal in.
- Add imports for `PageHeader` and `SegmentedBar`.

In `style/printer.css`:
- Remove `.pr-head`, `.pr-progress` and `.pr-progress-fill` if unused.
- Add `.pr-earlier { margin-top: var(--nd-space-md); } .pr-earlier-list { list-style: none; } .pr-earlier-item { display: flex; gap: var(--nd-space-sm); color: var(--nd-text-secondary); font-size: 12px; }`.
- Make `.pr-hero-percent` sizing come from `.nd-display` plus a size such as `font-size: 56px` if the existing rule sets a mono font. Remove its `font-family` so Doto applies.

- [ ] **Step 5: Browser steps**

The printer steps check `.pr-hero-percent` innerText `6%` and the hero text. Keep them passing.

Add a step after the steps that emit states with errors:
- emit a view with `errors: []`;
- expect `.pr-earlier-item` to list `0300_0100_0001_0007`, because it was seen earlier in the run;
- then emit `PRINTER_VIEW` again, and expect that code back in the active list and not under "Earlier".

Also assert `.printer-page h1.nd-page-title` reads "Printer".

- [ ] **Step 6: Run everything and commit**

Run the full command list. All must pass in both engines.

```bash
git add src/printer/hms_history.rs src/printer/mod.rs src/pages/printer.rs style/printer.css tests/webkit/app-flows.mjs
git commit -m "feat(printer): page header, Doto percent, segmented progress, HMS codes seen earlier this session"
```

---

### Task 6: Slice touch-ups

**Files:**
- Modify: `src/pages/slice.rs`, `style/slice.css`, `tests/webkit/app-flows.mjs`

- [ ] **Step 1: The page**

In `src/pages/slice.rs`:
- Replace `<header class="sl-head"><h2>"Slice"</h2><p class="nd-label sl-version">…</p></header>` with a `PageHeader` that keeps the version line as its action:

```rust
            <PageHeader title="Slice">
                <p class="nd-label sl-version">
                    // the existing status match, unchanged
                </p>
            </PageHeader>
```

- `.sl-hero` (the print time) gets the Doto class, `class="sl-hero nd-display"`, in place of `nd-mono`. Keep its text and hook.
- Replace the job progress `<div class="sl-progress"><div class="sl-progress-bar" …></div></div>` with `<SegmentedBar percent=Signal::derive(move || percent() as u32) total=12 label="Slicing progress" />`. Adapt to `percent`'s actual type, keeping the value 0–100.
- Give `<table class="sl-compare-table">` the class `nd-table sl-compare-table`.
- Change the "Recent" `<h3 class="nd-label">` to `<h2 class="nd-label">`.
- Add the imports.

In `style/slice.css`:
- Remove `.sl-head`, `.sl-progress` and `.sl-progress-bar` if unused.
- Remove the `font-family` from `.sl-hero` so Doto applies. Keep its size, or set a hero size, for example `font-size: 56px`.
- Remove any `.sl-compare-table` header or cell styling that fights `.nd-table`, but keep the `.sl-delta` styling.

- [ ] **Step 2: Browser steps**

The slice steps read `.sl-hero` innerText (`"14m"`, `"2h 14m"`); keep them passing. Add an assertion to the "navigate to Slice" step that `.slice-page h1.nd-page-title` reads "Slice".

- [ ] **Step 3: Run everything and look**

Run the full command list. All must pass in both engines.

Then run `(cd tests/webkit && node screens.mjs ../..)` and look at:
- `dark-1280-analysis.png`
- `dark-1280-settings.png`
- `dark-1280-settings-health.png`
- `dark-1280-settings-about.png`
- `dark-1280-printer.png`
- `dark-1280-slice.png`
- `bambu-1280-settings.png`
- `dark-900-printer.png`

Check that each page has one hero and lines up with the header, and that nothing overflows at 900px. Fix any problems; don't commit the PNGs.

```bash
git add src/pages/slice.rs style/slice.css tests/webkit/app-flows.mjs
git commit -m "feat(slice): page header, Doto print time, segmented progress, nd-table compare"
```

---

## Self-review notes

How the plan covers the spec:

| Spec item | Covered by |
|---|---|
| Analyze (amended: quality word, toned; photo beside it; numbered and ordered defects) | Task 1 |
| Severity colours (four tiers distinct in colour and form; blue kept for links) | Task 1, with a contrast pair for the filled badge |
| Settings · General (Health hero; sections as label groups; printer and slicer settings restyled) | Task 2 |
| Settings · Health (summary hero; checks and repair panel kept) | Task 2 |
| Settings · About (app name in Doto; version and links as rows) | Task 3 |
| §5 Setup wizard (step counter, `SegmentedBar`, question as hero, Back/Continue pills, logic unchanged) | Task 4 |
| Printer touch-ups (`PageHeader`, `Hero` percent, `SegmentedBar`, HMS history) | Task 5 |
| Slice touch-ups (`PageHeader`, Doto hero, `SegmentedBar`, `.nd-table`) | Task 6 |
| Leftover: Print Analysis spinner | Task 1 (`StateText`) |
