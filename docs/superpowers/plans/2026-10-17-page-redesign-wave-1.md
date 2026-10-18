# Page Redesign Wave 1: Foundation, Seven Hubs and the Home Cockpit

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship wave 1 of the page redesign:
- the shared page patterns;
- the seven-hub rail, with tab routes and redirects for the old routes;
- the agent's route list;
- a keyboard-accessible STL indicator;
- the Home cockpit.

**Architecture:**
- **Shared patterns.** They are Leptos components in `src/components/patterns.rs`, styled once in `style/main.css`.
- **Hubs.** A hub is a thin `HubPage` wrapper: a `PageHeader` with tab pills above an existing page component. Pages are mounted unchanged under new routes.
- **Old routes.** These are caught by the router fallback, which redirects them with `replace: true` from one table in `src/routes.rs`.
- **Home.** It reads `PrinterShared`, plus three data calls, through pure helpers in `src/pages/home_state.rs`, so every decision is unit-tested.
- **Backend.** It gains only the `bm_navigate` route list and one read-only command, `latest_generated_preset`.

**Tech Stack:** Rust; Leptos 0.8 CSR (WASM, built by Trunk) with `leptos_router` 0.8; Tauri 2 backend (`src-tauri`); rusqlite; Playwright WebKit/Chromium tests in `tests/webkit/*.mjs`.

**Spec:** `docs/superpowers/specs/2026-10-17-page-redesign-design.md` (sections 1, 2, 3, 4 STL indicator, 6 wave 1, Testing).

## Global Constraints

- **Read-only printer.** Nothing in this piece sends a command to the printer. "Open printer" navigates; "resume on the printer" is advice text.
- **Mostly frontend.** There are two backend changes, both small:
  - the agent's `bm_navigate` route list;
  - one read-only command, `latest_generated_preset`, which returns the newest row of the `generated_presets` ledger for Home's "Recent" line.

  "Last print" is recorded in the frontend.
- **Both themes are first-class.** Every new text/background pair passes `contrast.mjs` in dark and light: 4.5:1 for text, 3:1 for disabled text.
- **Tokens only.** New CSS uses the `--nd-*` tokens from `style/tokens.css`. No new colours are introduced.
- **Font budget per screen.** At most two families (Space Grotesk and Space Mono), plus Doto for the single hero. At most three sizes and two weights.
- **Existing behaviour is kept.** Every feature on today's 11 pages is still reachable after the redesign. Nothing is removed.
- **Repo rules:**
  - Never commit `.omc/`, `.claude/` or `.superpowers/`.
  - Never run a bare `git stash`.
  - No attribution lines in commit messages.
  - The Claude product is labelled "Claude Agent", never "Claude Code".
  - Tests never launch the Bambu Studio GUI or call `open`.
  - The `claude-subscription` feature is never enabled in release builds.

## Commands used throughout

Run all of these from the repository root:

| What | Command |
|---|---|
| Frontend unit tests | `cargo test --bin bambumate` |
| Frontend WASM build check | `cargo check --target wasm32-unknown-unknown` |
| Backend tests | `cargo test --manifest-path src-tauri/Cargo.toml` |
| Backend lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` |
| Format check | `cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check` |
| Frontend bundle | `trunk build` (writes `dist/`; the browser tests need it) |
| Browser tests | `(cd tests/webkit && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)` |

Format a single Rust file with `rustfmt --edition 2021 <file>`. Never use `cargo fmt -- <file>`, which reformats the whole crate.

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/components/patterns.rs` | new | `PageHeader`, `Hero`, `SegmentedBar`, `StatRow`, `StateText`, `Tone`, and the pure helpers `segments_filled` and `state_text` |
| `src/routes.rs` | new | Hub tab lists, the old-route table, `legacy_target`, and the served-route list used by tests |
| `src/pages/hub.rs` | new | `HubPage`: the page header with tabs, wrapped around an existing page |
| `src/pages/home_state.rs` | new | Pure Home decisions: mode, labels, detail lines, active slot, "ago" text, sync and agent text |
| `src/pages/home.rs` | rewritten | The Home cockpit view |
| `src/printer/last_print.rs` | new | `LastPrint` and `track_print`, which notice when a print finishes |
| `style/home.css` | new | Home cockpit styles |
| `src/components/sidebar.rs` | modified | The seven hubs, with Settings pinned below a divider |
| `src/app.rs` | modified | Hub routes and a fallback that redirects old routes or shows "not found" |
| `src/components/stl_indicator.rs` | modified | The badge becomes an icon `<button>` with an aria-label, Escape-to-close and focus return |
| `src/components/icons.rs` | modified | Adds `Stl`; drops icons the rail no longer uses |
| `src/agent/drawer.rs` | modified | `AgentShared.prefill`: lets another page put a question into the composer |
| `src/pages/printer.rs` | modified | `SlotPicker` becomes `pub(crate)`; `refocus_card` finds `[data-slot-label]` |
| `src/printer/mod.rs` | modified | `PrinterShared.last_print`, fed by an effect in `PrinterEvents` |
| `src/commands.rs` | modified | Adds `GeneratedPreset` and `latest_generated_preset()` |
| `src/components/branding.rs` | modified | Removes `HeroArtwork`, which only the old Home used |
| `style/main.css` | modified | Adds the "Page patterns" section and `.nav-list-end`; updates STL button styles; removes old Home rules |
| `index.html` | modified | Links `style/home.css` |
| `src-tauri/src/agent/tools/app.rs` | modified | `ROUTES` gains the tab routes; the description lists them |
| `src-tauri/src/history/{types,store,ledger}.rs` | modified | `GeneratedPreset`, `latest_generated_preset` |
| `src-tauri/src/commands/preset_sync.rs`, `src-tauri/src/lib.rs` | modified | The `latest_generated_preset` command and its registration |
| `tests/webkit/{app-flows,layout,screens,contrast,fixtures}.mjs` | modified | The seven-hub rail, tab navigation, redirects, the STL keyboard check, the Home states, and the new contrast pairs |

---

### Task 1: Shared page patterns

**Files:**
- Create: `src/components/patterns.rs`
- Modify: `src/components/mod.rs`, `style/main.css` (append a section), `tests/webkit/contrast.mjs`

**Interfaces:**
- Consumes: `crate::routes::Tab`. Task 2 creates `src/routes.rs`; this task creates the file with only the `Tab` alias, and Task 2 fills in the rest.
- Produces:
  - `PageHeader(title: &'static str, tabs: &'static [Tab] = &[], children: Option<Children>)`
  - `Hero(label: Signal<String>, value: Signal<String>, unit: MaybeProp<String>, detail: MaybeProp<String>, tone: MaybeProp<Tone>)`
  - `SegmentedBar(percent: Signal<u32>, total: u32)`
  - `StatRow(label: &'static str, value: Signal<String>)`
  - `StateText(kind: StateKind, message: MaybeProp<String>)`
  - `enum Tone { Neutral, Signal, Warning, Accent }` with `key()`
  - `enum StateKind { Loading, Empty, Error }`
  - `fn segments_filled(percent: u32, total: u32) -> u32`
  - `fn state_text(kind: StateKind, message: Option<&str>) -> String`
  - CSS classes: `.nd-page-header`, `.nd-page-title`, `.nd-tabs`, `.nd-tab`, `.nd-hero`, `.nd-hero-label`, `.nd-hero-value`, `.nd-hero-number`, `.nd-hero-unit`, `.nd-hero-detail`, `.nd-segbar`, `.nd-seg`, `.nd-seg-on`, `.nd-stat-row`, `.nd-table`, `.nd-num`, `.nd-state`, `.nd-visually-hidden`, `.hub`

- [ ] **Step 1: Create `src/routes.rs` with just the tab alias, and register the module**

`src/routes.rs`:

```rust
//! The seven hubs' tabs and where the routes from before the hubs went
//! (spec 2026-10-17 §1).

/// A tab inside a hub: its label and its route.
pub type Tab = (&'static str, &'static str);
```

In `src/main.rs`, add `mod routes;` after `mod printer;`:

```rust
mod agent;
mod app;
mod commands;
mod components;
mod pages;
mod printer;
mod routes;
mod slicer;
mod theme;
```

- [ ] **Step 2: Write the failing tests**

Create `src/components/patterns.rs` with only the tests module, so the test run fails because the functions don't exist yet:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_fill_only_at_the_ends_when_the_percent_is_at_the_ends() {
        assert_eq!(segments_filled(0, 12), 0);
        assert_eq!(segments_filled(100, 12), 12);
        // 1% shows something; 99% never looks finished.
        assert_eq!(segments_filled(1, 12), 1);
        assert_eq!(segments_filled(99, 12), 11);
        assert_eq!(segments_filled(6, 12), 1);
        assert_eq!(segments_filled(42, 12), 5);
        assert_eq!(segments_filled(50, 12), 6);
        assert_eq!(segments_filled(250, 12), 12);
    }

    #[test]
    fn tiny_bars_do_not_panic() {
        assert_eq!(segments_filled(50, 0), 0);
        assert_eq!(segments_filled(99, 1), 0);
        assert_eq!(segments_filled(100, 1), 1);
    }

    #[test]
    fn state_text_is_bracketed() {
        assert_eq!(state_text(StateKind::Loading, None), "[LOADING…]");
        assert_eq!(state_text(StateKind::Loading, Some("x")), "[LOADING…]");
        assert_eq!(state_text(StateKind::Empty, None), "[EMPTY]");
        assert_eq!(state_text(StateKind::Empty, Some("Page not found")), "[EMPTY: Page not found]");
        assert_eq!(state_text(StateKind::Error, None), "[ERROR]");
        assert_eq!(state_text(StateKind::Error, Some("scan failed")), "[ERROR: scan failed]");
    }

    #[test]
    fn tones_have_distinct_keys() {
        let keys = [Tone::Neutral, Tone::Signal, Tone::Warning, Tone::Accent].map(Tone::key);
        assert_eq!(keys, ["neutral", "signal", "warning", "accent"]);
        assert_eq!(Tone::default(), Tone::Neutral);
    }
}
```

Add `pub mod patterns;` to `src/components/mod.rs`, keeping the list alphabetical: it goes between `pub mod icons;` and `pub mod preset_sync_panel;`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --bin bambumate patterns`
Expected: compile errors: `cannot find function segments_filled`, `cannot find type StateKind`, `Tone`.

- [ ] **Step 4: Implement the patterns**

Put this above the tests module in `src/components/patterns.rs`:

```rust
//! Shared page patterns (spec 2026-10-17 §2). Pages build from these and
//! do not restyle them locally; their CSS is the "Page patterns" section
//! of style/main.css.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::routes::Tab;

/// The colour a hero's value takes. Only the value is coloured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Signal,
    Warning,
    Accent,
}

impl Tone {
    /// `data-tone` on the hero.
    pub fn key(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Signal => "signal",
            Self::Warning => "warning",
            Self::Accent => "accent",
        }
    }
}

/// What a `StateText` line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    Loading,
    Empty,
    Error,
}

/// `[LOADING…]`, `[EMPTY]`, `[EMPTY: …]`, `[ERROR]` or `[ERROR: …]`.
pub fn state_text(kind: StateKind, message: Option<&str>) -> String {
    match (kind, message) {
        (StateKind::Loading, _) => "[LOADING…]".into(),
        (StateKind::Empty, None) => "[EMPTY]".into(),
        (StateKind::Empty, Some(m)) => format!("[EMPTY: {m}]"),
        (StateKind::Error, None) => "[ERROR]".into(),
        (StateKind::Error, Some(m)) => format!("[ERROR: {m}]"),
    }
}

/// How many of `total` segments to fill for `percent`. Rounds to the
/// nearest segment, but only 0% shows none and only 100% shows all.
pub fn segments_filled(percent: u32, total: u32) -> u32 {
    let p = percent.min(100);
    match total {
        0 => 0,
        1 => u32::from(p == 100),
        _ => match p {
            0 => 0,
            100 => total,
            _ => ((p * total + 50) / 100).clamp(1, total - 1),
        },
    }
}

/// The page's single `<h1>`, an optional row of actions, and an optional
/// row of tab pills that link to routes.
#[component]
pub fn PageHeader(
    title: &'static str,
    #[prop(optional)] tabs: &'static [Tab],
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let pathname = use_location().pathname;
    let tab_row = (!tabs.is_empty()).then(|| {
        let links = tabs
            .iter()
            .map(|&(label, href)| {
                let current = move || (pathname.get() == href).then_some("page");
                view! { <a href=href class="nd-tab" aria-current=current>{label}</a> }
            })
            .collect_view();
        view! { <nav class="nd-tabs" aria-label=format!("{title} sections")>{links}</nav> }
    });
    view! {
        <header class="nd-page-header">
            <div class="nd-page-header-row">
                <h1 class="nd-page-title">{title}</h1>
                {children.map(|c| view! { <div class="nd-page-actions">{c()}</div> })}
            </div>
            {tab_row}
        </header>
    }
}

/// The one big thing on a page: a mono label, a Doto value with an
/// optional unit, and an optional detail line. One per page.
#[component]
pub fn Hero(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] value: Signal<String>,
    #[prop(into, optional)] unit: MaybeProp<String>,
    #[prop(into, optional)] detail: MaybeProp<String>,
    #[prop(into, optional)] tone: MaybeProp<Tone>,
) -> impl IntoView {
    view! {
        <section class="nd-hero" data-tone=move || tone.get().unwrap_or_default().key()>
            <p class="nd-label nd-hero-label">{move || label.get()}</p>
            <p class="nd-hero-value">
                <span class="nd-display nd-hero-number">{move || value.get()}</span>
                {move || unit.get().map(|u| view! { <span class="nd-hero-unit">{u}</span> })}
            </p>
            {move || {
                detail
                    .get()
                    .filter(|d| !d.is_empty())
                    .map(|d| view! { <p class="nd-hero-detail">{d}</p> })
            }}
        </section>
    }
}

/// Discrete progress: `total` segments, filled per [`segments_filled`].
#[component]
pub fn SegmentedBar(#[prop(into)] percent: Signal<u32>, total: u32) -> impl IntoView {
    let filled = move || segments_filled(percent.get(), total);
    let segments = (0..total)
        .map(|i| view! { <span class="nd-seg" class:nd-seg-on=move || i < filled()></span> })
        .collect_view();
    view! {
        <div class="nd-segbar" role="progressbar" aria-valuemin="0" aria-valuemax="100"
            aria-valuenow=move || percent.get().min(100).to_string()>
            {segments}
        </div>
    }
}

/// A label on the left, a mono value right-aligned.
#[component]
pub fn StatRow(label: &'static str, #[prop(into)] value: Signal<String>) -> impl IntoView {
    view! {
        <div class="nd-stat-row">
            <span class="nd-stat-row-label">{label}</span>
            <span class="nd-stat-row-value">{move || value.get()}</span>
        </div>
    }
}

/// An inline `[LOADING…]`, `[EMPTY…]` or `[ERROR…]` line, in place of
/// spinners, toasts and empty-state paragraphs.
#[component]
pub fn StateText(kind: StateKind, #[prop(into, optional)] message: MaybeProp<String>) -> impl IntoView {
    let key = match kind {
        StateKind::Loading => "loading",
        StateKind::Empty => "empty",
        StateKind::Error => "error",
    };
    let role = if kind == StateKind::Error { "alert" } else { "status" };
    view! {
        <p class="nd-state" data-kind=key role=role>
            {move || state_text(kind, message.get().as_deref())}
        </p>
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --bin bambumate patterns`
Expected: 4 passed.

Run: `cargo check --target wasm32-unknown-unknown`
Expected: builds. Warnings that `PageHeader`, `Hero` and the others are unused are fine; later tasks use them.

- [ ] **Step 6: Add the pattern CSS**

Append to the end of `style/main.css`:

```css
/* ===== Page patterns (spec 2026-10-17 §2) =====
   Shared by every page through src/components/patterns.rs. Pages do not
   restyle these locally. */

.hub {
    max-width: 1040px;
}

.nd-page-header {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-md);
    margin-bottom: var(--nd-space-xl);
}

.nd-page-header-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--nd-space-md);
}

.nd-page-title {
    font-family: var(--nd-font-body);
    font-size: 28px;
    font-weight: 400;
    letter-spacing: -0.01em;
    color: var(--nd-text-display);
}

.nd-page-actions {
    display: flex;
    gap: var(--nd-space-sm);
}

.nd-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-sm);
}

.nd-tab {
    padding: 6px 14px;
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-pill);
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--nd-text-secondary);
    text-decoration: none;
    transition: color 150ms var(--nd-ease), border-color 150ms var(--nd-ease), background-color 150ms var(--nd-ease);
}

.nd-tab:hover {
    color: var(--nd-text-primary);
    border-color: var(--nd-text-secondary);
}

.nd-tab:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.nd-tab[aria-current="page"] {
    background-color: var(--nd-text-display);
    border-color: var(--nd-text-display);
    color: var(--nd-black);
}

.nd-hero {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-sm);
}

.nd-hero-value {
    display: flex;
    align-items: baseline;
    gap: var(--nd-space-xs);
}

.nd-hero-number {
    font-size: clamp(48px, 7vw, 88px);
}

.nd-hero-unit {
    font-family: var(--nd-font-mono);
    font-size: 20px;
    color: var(--nd-text-secondary);
}

.nd-hero-detail {
    font-family: var(--nd-font-mono);
    font-size: 12px;
    color: var(--nd-text-secondary);
}

.nd-hero[data-tone="signal"] .nd-hero-number {
    color: var(--nd-signal);
}

.nd-hero[data-tone="warning"] .nd-hero-number {
    color: var(--nd-warning);
}

.nd-hero[data-tone="accent"] .nd-hero-number {
    color: var(--nd-accent);
}

.nd-segbar {
    display: flex;
    gap: 3px;
}

.nd-seg {
    flex: 1;
    height: 8px;
    border-radius: 2px;
    background-color: var(--nd-border);
}

.nd-seg-on {
    background-color: var(--nd-signal);
}

.nd-stat-row {
    display: flex;
    justify-content: space-between;
    gap: var(--nd-space-md);
    padding: var(--nd-space-sm) 0;
    border-bottom: 1px solid var(--nd-border);
}

.nd-stat-row-label {
    color: var(--nd-text-secondary);
}

.nd-stat-row-value {
    font-family: var(--nd-font-mono);
    font-variant-numeric: tabular-nums;
    text-align: right;
    color: var(--nd-text-primary);
}

.nd-table {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
}

.nd-table th {
    padding: var(--nd-space-sm) var(--nd-space-md) var(--nd-space-sm) 0;
    border-bottom: 1px solid var(--nd-border-visible);
    font-family: var(--nd-font-mono);
    font-size: 11px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-align: left;
    text-transform: uppercase;
    color: var(--nd-text-secondary);
}

.nd-table td {
    padding: var(--nd-space-sm) var(--nd-space-md) var(--nd-space-sm) 0;
    border-bottom: 1px solid var(--nd-border);
    color: var(--nd-text-primary);
}

.nd-table .nd-num {
    font-family: var(--nd-font-mono);
    text-align: right;
}

.nd-state {
    font-family: var(--nd-font-mono);
    font-size: 12px;
    letter-spacing: 0.04em;
    color: var(--nd-text-secondary);
}

.nd-state[data-kind="error"] {
    color: var(--nd-accent);
}

.nd-visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
}
```

- [ ] **Step 7: Add the filled-pill pairs to the contrast check**

The active tab pill and `.btn-primary` draw `--nd-black` text on a filled `--nd-text-display` background, or on `--nd-text-primary` when hovered. In `tests/webkit/contrast.mjs`, add this after the `BACKGROUNDS` line:

```js
// Text drawn on a filled token: active tab pills and primary buttons
// (--nd-black on --nd-text-display, and on --nd-text-primary when hovered).
const FILLED = [
  ["--nd-black", "--nd-text-display", 4.5],
  ["--nd-black", "--nd-text-primary", 4.5],
];
```

Then, inside `run()`, after the closing brace of the `for (const [fg, min] of TEXT)` loop and still inside the `for (const theme of THEMES)` loop, add:

```js
    for (const [fg, bg, min] of FILLED) {
      const ratio = contrast(vars[fg], vars[bg]);
      const ok = ratio >= min;
      console.log(`  ${name} ${ok ? "OK  " : "FAIL"} ${theme} ${fg} on filled ${bg}: ${ratio.toFixed(2)}:1 (min ${min})`);
      if (!ok) failures.push(`${theme}: ${fg} on filled ${bg} = ${ratio.toFixed(2)}:1 < ${min}`);
    }
```

`vars` already holds all three tokens: `--nd-black` comes from `BACKGROUNDS`, and the other two from `TEXT`.

- [ ] **Step 8: Run the contrast and CSS checks**

Run: `(cd tests/webkit && node contrast.mjs ../.. && node css-compat.mjs ../..)`
Expected: `All token contrasts pass.`, including four new `on filled` lines per engine, all OK; css-compat reports no new failures.

- [ ] **Step 9: Commit**

```bash
git add src/main.rs src/routes.rs src/components/mod.rs src/components/patterns.rs style/main.css tests/webkit/contrast.mjs
git commit -m "feat(ui): shared page patterns (header, hero, segmented bar, stat row, state text)"
```

---

### Task 2: Seven hubs with tab routes and old-route redirects

**Files:**
- Modify: `src/routes.rs`, `src/components/sidebar.rs`, `src/app.rs`, `src/components/icons.rs`, `style/main.css`, `tests/webkit/app-flows.mjs`, `tests/webkit/layout.mjs`, `tests/webkit/screens.mjs`
- Create: `src/pages/hub.rs`
- Modify: `src/pages/mod.rs`

**Interfaces:**
- Consumes: `PageHeader`, `StateText` and `StateKind` from Task 1.
- Produces:
  - In `src/routes.rs`: `FILAMENT_TABS`, `PROFILES_TABS`, `SETTINGS_TABS` (`&[Tab]`); `LEGACY_REDIRECTS: &[(&str, &str)]`; `fn legacy_target(path: &str) -> Option<&'static str>`; and, under `#[cfg(test)]`, `PAGE_ROUTES`.
  - `HubPage(title, tabs, children)`.
  - The rail markup: `<ul class="nav-list">` (six hubs) followed by `<ul class="nav-list nav-list-end">` (Settings).
  - Tab links are `.nd-tabs a.nd-tab[href=…]`, with `aria-current="page"` on the active one.

- [ ] **Step 1: Write the failing route-table tests**

Replace `src/routes.rs` with:

```rust
//! The seven hubs' tabs and where the routes from before the hubs went
//! (spec 2026-10-17 §1).

/// A tab inside a hub: its label and its route.
pub type Tab = (&'static str, &'static str);

pub const FILAMENT_TABS: &[Tab] = &[("Search", "/filament"), ("Batch", "/filament/batch")];

pub const PROFILES_TABS: &[Tab] = &[("Library", "/profiles"), ("Compare", "/profiles/compare")];

pub const SETTINGS_TABS: &[Tab] = &[
    ("General", "/settings"),
    ("Health", "/settings/health"),
    ("About", "/settings/about"),
];

/// Routes from before the hubs, and the tab each one moved to. The router's
/// fallback redirects them (see `app.rs`), so links and the agent's saved
/// routes keep working.
pub const LEGACY_REDIRECTS: &[(&str, &str)] = &[
    ("/batch", "/filament/batch"),
    ("/compare", "/profiles/compare"),
    ("/health", "/settings/health"),
    ("/about", "/settings/about"),
];

/// Where an old route moved to; `None` for any other path.
pub fn legacy_target(path: &str) -> Option<&'static str> {
    LEGACY_REDIRECTS
        .iter()
        .find(|(old, _)| *old == path)
        .map(|(_, new)| *new)
}

/// Every route a page is served at. Mirrors the `<Route>` list in
/// `app.rs`; the tests below and the rail's tests check against it.
#[cfg(test)]
pub const PAGE_ROUTES: &[&str] = &[
    "/",
    "/printer",
    "/filament",
    "/filament/batch",
    "/profiles",
    "/profiles/compare",
    "/slice",
    "/analysis",
    "/settings",
    "/settings/health",
    "/settings/about",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_old_route_moves_to_a_served_tab() {
        for (old, new) in LEGACY_REDIRECTS {
            assert!(PAGE_ROUTES.contains(new), "{old} -> {new}, which is not served");
            assert!(!PAGE_ROUTES.contains(old), "{old} is still served directly");
        }
    }

    #[test]
    fn legacy_target_maps_old_routes_only() {
        assert_eq!(legacy_target("/batch"), Some("/filament/batch"));
        assert_eq!(legacy_target("/compare"), Some("/profiles/compare"));
        assert_eq!(legacy_target("/health"), Some("/settings/health"));
        assert_eq!(legacy_target("/about"), Some("/settings/about"));
        assert_eq!(legacy_target("/settings"), None);
        assert_eq!(legacy_target("/nope"), None);
    }

    #[test]
    fn every_tab_is_served_and_the_first_tab_is_the_hub_itself() {
        for (hub, tabs) in [
            ("/filament", FILAMENT_TABS),
            ("/profiles", PROFILES_TABS),
            ("/settings", SETTINGS_TABS),
        ] {
            assert_eq!(tabs[0].1, hub);
            for (_, href) in tabs {
                assert!(PAGE_ROUTES.contains(href), "{href} is not served");
                assert!(href.starts_with(hub), "{href} is not under {hub}");
            }
        }
    }
}
```

Replace the tests module at the bottom of `src/components/sidebar.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::PAGE_ROUTES;

    #[test]
    fn the_rail_has_the_seven_hubs_in_order() {
        let hrefs: Vec<&str> = NAV_ITEMS.iter().map(|i| i.href).collect();
        assert_eq!(
            hrefs,
            vec!["/", "/printer", "/filament", "/profiles", "/slice", "/analysis", "/settings"]
        );
        let labels: Vec<&str> = NAV_ITEMS.iter().map(|i| i.label).collect();
        assert_eq!(
            labels,
            vec!["Home", "Printer", "Filament", "Profiles", "Slice", "Analyze", "Settings"]
        );
    }

    #[test]
    fn only_settings_sits_below_the_divider() {
        let end: Vec<&str> = NAV_ITEMS
            .iter()
            .filter(|i| i.section == NavSection::End)
            .map(|i| i.href)
            .collect();
        assert_eq!(end, vec!["/settings"]);
    }

    #[test]
    fn every_rail_item_is_a_served_route() {
        for item in NAV_ITEMS {
            assert!(PAGE_ROUTES.contains(&item.href), "{} is not served", item.href);
        }
    }

    #[test]
    fn active_matching_is_exact_for_home_and_prefix_for_others() {
        assert!(is_active("/", "/"));
        assert!(!is_active("/profiles", "/"));
        assert!(is_active("/profiles", "/profiles"));
        assert!(is_active("/profiles/compare", "/profiles"));
        assert!(!is_active("/profilesx", "/profiles"));
        assert!(is_active("/filament/batch", "/filament"));
        assert!(is_active("/settings/about", "/settings"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin bambumate -- routes sidebar`
Expected: compile errors: `NavSection` is not found and `NavItem` has no field `section`. The routes tests can't run until the crate compiles.

- [ ] **Step 3: Rebuild the rail**

In `src/components/sidebar.rs`, replace everything from `pub struct NavItem {` through the end of the `NAV_ITEMS` constant with:

```rust
/// Which list an item sits in: the main rail, or pinned below the divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavSection {
    Main,
    End,
}

pub struct NavItem {
    pub href: &'static str,
    pub label: &'static str,
    pub icon: IconKind,
    pub section: NavSection,
}

/// The seven hubs (spec 2026-10-17 §1). Tabbed hubs keep their tabs in
/// `crate::routes`.
pub const NAV_ITEMS: &[NavItem] = &[
    NavItem {
        href: "/",
        label: "Home",
        icon: IconKind::Home,
        section: NavSection::Main,
    },
    NavItem {
        href: "/printer",
        label: "Printer",
        icon: IconKind::Printer,
        section: NavSection::Main,
    },
    NavItem {
        href: "/filament",
        label: "Filament",
        icon: IconKind::Create,
        section: NavSection::Main,
    },
    NavItem {
        href: "/profiles",
        label: "Profiles",
        icon: IconKind::Profiles,
        section: NavSection::Main,
    },
    NavItem {
        href: "/slice",
        label: "Slice",
        icon: IconKind::Slice,
        section: NavSection::Main,
    },
    NavItem {
        href: "/analysis",
        label: "Analyze",
        icon: IconKind::Analyze,
        section: NavSection::Main,
    },
    NavItem {
        href: "/settings",
        label: "Settings",
        icon: IconKind::Settings,
        section: NavSection::End,
    },
];
```

Then replace the body of `Sidebar()` from `let ff_ctx = …` to the end of the function with:

```rust
    let flags = use_context::<FeatureFlagsContext>()
        .expect("FeatureFlagsContext not provided")
        .flags;
    let update_info = use_context::<UpdateContext>()
        .expect("UpdateContext not provided")
        .update_info;
    let printer = use_context::<PrinterShared>().expect("PrinterShared not provided");
    // No dot until a printer is set up.
    let printer_dot = move || {
        printer
            .view
            .with(|v| v.configured.then(|| v.connection.dot()))
    };
    let pathname = use_location().pathname;
    // A navigation click completes the hover interaction. Collapse even if
    // the pointer stays on the rail, so it cannot cover the new page's controls.
    // Leaving the rail arms hover again; keyboard focus still expands it.
    let hover_dismissed = RwSignal::new(false);

    let render = move |item: &'static NavItem| {
        let (href, label, icon) = (item.href, item.label, item.icon);
        let active = move || is_active(&pathname.get(), href);
        let locked = move || href == "/analysis" && !flags.get().analysis_enabled;
        // About is a Settings tab now, so the update dot sits on Settings.
        let has_update =
            move || href == "/settings" && update_info.get().map(|i| i.has_update).unwrap_or(false);
        view! {
            <li class="nav-item" class:nav-item-locked=locked>
                <a href=href class="nav-link" class:active=active class:nav-link-locked=locked
                    on:click=move |_| hover_dismissed.set(true)
                    aria-current=move || if active() { Some("page") } else { None }
                    aria-label=move || has_update().then(|| format!("{label}, update available"))
                    title=move || locked().then(|| "Requires AI — enable in Settings".to_string())>
                    <span class="nav-icon"><Icon kind=icon /></span>
                    <span class="nav-label">{label}</span>
                    <Show when=locked>
                        <span class="nav-lock"><Icon kind=IconKind::Lock /></span>
                    </Show>
                    <Show when=has_update>
                        <span class="nav-update-dot" aria-hidden="true" title="Update available"></span>
                    </Show>
                    <Show when=move || href == "/printer">
                        {move || printer_dot().map(|state| view! {
                            <span class="printer-dot" data-state=state role="img"
                                aria-label=format!("Printer: {state}") title="Printer connection"></span>
                        })}
                    </Show>
                </a>
            </li>
        }
    };
    let main_items = NAV_ITEMS
        .iter()
        .filter(|i| i.section == NavSection::Main)
        .map(render)
        .collect_view();
    let end_items = NAV_ITEMS
        .iter()
        .filter(|i| i.section == NavSection::End)
        .map(render)
        .collect_view();

    view! {
        <nav class="sidebar" class:hover-dismissed=move || hover_dismissed.get()
            on:mouseleave=move |_| hover_dismissed.set(false) aria-label="Main">
            <div class="sidebar-header">
                <span class="rail-mark" aria-hidden="true"></span>
                <span class="sidebar-wordmark">"BAMBUMATE"</span>
            </div>
            <ul class="nav-list">{main_items}</ul>
            <ul class="nav-list nav-list-end">{end_items}</ul>
            <div class="sidebar-footer">
                <StlIndicator />
            </div>
        </nav>
    }
```

`render` captures only `Copy` values (signals, a memo and a closure over a `Copy` context), so it is `Copy` and can be passed to both `.map` calls.

- [ ] **Step 4: Drop the icons the rail no longer uses**

Run: `grep -rn "IconKind::Batch\|IconKind::Compare\|IconKind::Health\|IconKind::About" src`
Expected: matches only in `src/components/icons.rs`.

In `src/components/icons.rs`, delete the `Batch`, `Compare`, `Health` and `About` variants from `IconKind`, and delete their four arms from the `match kind` in `Icon`.

- [ ] **Step 5: Pin Settings below a divider**

In `style/main.css`, directly after the `.nav-list { … }` rule, add:

```css
/* Settings, pinned to the bottom of the rail below a divider. */
.nav-list-end {
    flex: none;
    border-top: 1px solid var(--nd-border);
}
```

`.nav-list` already has `flex: 1`, so the main list grows and pushes this one down.

- [ ] **Step 6: Add the hub wrapper**

Create `src/pages/hub.rs`:

```rust
//! A hub: one rail destination whose pages are tabs (spec 2026-10-17 §1).
//! Wave 1 mounts the existing pages unchanged under the shared header.

use leptos::prelude::*;

use crate::components::patterns::PageHeader;
use crate::routes::Tab;

#[component]
pub fn HubPage(title: &'static str, tabs: &'static [Tab], children: Children) -> impl IntoView {
    view! {
        <div class="hub" data-hub=title.to_ascii_lowercase()>
            <PageHeader title=title tabs=tabs />
            {children()}
        </div>
    }
}
```

Add `pub mod hub;` to `src/pages/mod.rs`, between `health` and `home`.

- [ ] **Step 7: Route the hubs and redirect old routes**

In `src/app.rs`, add these imports next to the existing ones:

```rust
use leptos_router::hooks::use_location;
use leptos_router::NavigateOptions;

use crate::components::patterns::{StateKind, StateText};
use crate::pages::hub::HubPage;
use crate::routes::{legacy_target, FILAMENT_TABS, PROFILES_TABS, SETTINGS_TABS};
```

Add this component above `pub fn App()`:

```rust
/// What the router shows when no route matches: a route from before the
/// hubs is replaced in history by the tab it moved to, so Back never lands
/// on it and bounces; anything else is "not found".
#[component]
fn NoRoute() -> impl IntoView {
    let path = use_location().pathname.get_untracked();
    match legacy_target(&path) {
        Some(to) => view! {
            <Redirect path=to options=NavigateOptions { replace: true, ..Default::default() } />
        }
        .into_any(),
        None => view! {
            <div class="page">
                <StateText kind=StateKind::Empty message="Page not found" />
                <a href="/" class="btn btn-secondary">"Back to Home"</a>
            </div>
        }
        .into_any(),
    }
}
```

Replace the whole `<Routes …>…</Routes>` block with:

```rust
                        <Routes fallback=|| view! { <NoRoute /> }>
                            <Route path=path!("/") view=HomePage />
                            <Route path=path!("/printer") view=PrinterPage />
                            <Route path=path!("/filament") view=|| view! {
                                <HubPage title="Filament" tabs=FILAMENT_TABS><FilamentSearchPage /></HubPage>
                            } />
                            <Route path=path!("/filament/batch") view=|| view! {
                                <HubPage title="Filament" tabs=FILAMENT_TABS><BatchGeneratePage /></HubPage>
                            } />
                            <Route path=path!("/profiles") view=|| view! {
                                <HubPage title="Profiles" tabs=PROFILES_TABS><ProfileManagementPage /></HubPage>
                            } />
                            <Route path=path!("/profiles/compare") view=|| view! {
                                <HubPage title="Profiles" tabs=PROFILES_TABS><ProfileDiffPage /></HubPage>
                            } />
                            <Route path=path!("/slice") view=SlicePage />
                            <Route path=path!("/analysis") view=PrintAnalysisPage />
                            <Route path=path!("/settings") view=|| view! {
                                <HubPage title="Settings" tabs=SETTINGS_TABS><SettingsPage /></HubPage>
                            } />
                            <Route path=path!("/settings/health") view=|| view! {
                                <HubPage title="Settings" tabs=SETTINGS_TABS><HealthPage /></HubPage>
                            } />
                            <Route path=path!("/settings/about") view=|| view! {
                                <HubPage title="Settings" tabs=SETTINGS_TABS><AboutPage /></HubPage>
                            } />
                        </Routes>
```

- [ ] **Step 8: Run the unit tests and the WASM check**

Run: `cargo test --bin bambumate`
Expected: all pass, including the 3 routes tests and 4 sidebar tests.

Run: `cargo check --target wasm32-unknown-unknown`
Expected: builds with no errors.

- [ ] **Step 9: Teach the browser tests the hubs**

These changes go in `tests/webkit/app-flows.mjs`.

(a) Directly after `await page.addInitScript(installTauriMock, FIXTURES);` in `driveApp`, add these helpers:

```js
  // Follows an in-app link without the rail: the router handles any
  // same-origin <a> click, which is what a link elsewhere in the app does.
  // Fails if the browser did a full page load instead.
  const goTo = async (route) => {
    await page.evaluate((r) => {
      window.__sameDocument = true;
      const a = document.createElement("a");
      a.href = r;
      document.body.append(a);
      a.click();
      a.remove();
    }, route);
    await page.waitForFunction(() => window.__sameDocument === true, null, { timeout: 2000 });
  };
  // Opens a hub from the rail, then one of its tabs.
  const openTab = async (hub, tab) => {
    await page.click(`nav.sidebar a[href="${hub}"]`);
    await page.click(`.nd-tabs a[href="${tab}"]`);
  };
```

(b) In the step "app boots and renders the shell", replace

```js
    if (links < 8) throw new Error(`only ${links} nav links rendered`);
```

with

```js
    if (links !== 7) throw new Error(`${links} nav links rendered, expected the 7 hubs`);
```

(c) In the step "navigation leaves the new page's left controls clickable", replace `await page.click('nav.sidebar a[href="/health"]');` with `await openTab("/settings", "/settings/health");`.

(d) In "navigate to Batch Generate", replace `await page.click('a[href="/batch"]');` with `await openTab("/filament", "/filament/batch");`.

(e) In "navigate to Compare Profiles", replace `await page.click('a[href="/compare"]');` with `await openTab("/profiles", "/profiles/compare");`.

(f) In "navigate to Health Check", replace `await page.click('a[href="/health"]');` with `await openTab("/settings", "/settings/health");`.

(g) Replace every remaining `await page.click('a[href="/about"]');` with `await goTo("/settings/about");`. There are 8 of them; the following `waitForSelector(".about-page", …)` lines stay as they are. These were "go somewhere quiet" moves, and `goTo` keeps them quiet: it mounts only the About tab, not Settings → General.

(h) Replace

```js
    // The picker's backdrop covers the rail, so navigate the way the agent would.
    await page.locator('a[href="/about"]').dispatchEvent("click");
```

with

```js
    // The picker's backdrop covers the rail, so follow a link the way one elsewhere in the app would.
    await goTo("/settings/about");
```

(i) In the slice step that checks `agent_set_app_state`, replace `return s?.route === "/about" && s.slice == null;` with `return s?.route === "/settings/about" && s.slice == null;`.

Run: `grep -n 'href="/about"\|href="/health"\|href="/batch"\|href="/compare"' tests/webkit/app-flows.mjs`
Expected: no output.

(j) Directly after the "navigate to About" step, add:

```js
  // -- hubs ------------------------------------------------------------------
  await step(run, page, "old routes land on their hub tab and Back skips them", async () => {
    for (const [old, tab, selector] of [
      ["/batch", "/filament/batch", ".batch-generate-page"],
      ["/compare", "/profiles/compare", ".profile-diff-page"],
      ["/health", "/settings/health", ".health-page"],
      ["/about", "/settings/about", ".about-page"],
    ]) {
      await page.click('nav.sidebar a[href="/slice"]');
      await page.waitForSelector(".slice-page", { timeout: 15000 });
      await goTo(old);
      await page.waitForFunction((t) => location.pathname === t, tab, { timeout: 5000 });
      await page.waitForSelector(selector, { timeout: 15000 });
      const current = await page.locator('.nd-tabs a[aria-current="page"]').getAttribute("href");
      if (current !== tab) throw new Error(`${old}: the current tab is ${current}`);
      await page.goBack();
      await page.waitForFunction(() => location.pathname === "/slice", null, { timeout: 5000 });
    }
  });

  await step(run, page, "switching tabs updates the URL and Back returns to the previous tab", async () => {
    await page.click('nav.sidebar a[href="/settings"]');
    await page.waitForSelector(".settings-page", { timeout: 15000 });
    await page.click('.nd-tabs a[href="/settings/about"]');
    await page.waitForFunction(() => location.pathname === "/settings/about", null, { timeout: 5000 });
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.goBack();
    await page.waitForFunction(() => location.pathname === "/settings", null, { timeout: 5000 });
    await page.waitForSelector(".settings-page", { timeout: 15000 });
    const h1 = (await page.locator("h1.nd-page-title").textContent()).trim();
    if (h1 !== "Settings") throw new Error(`h1 reads "${h1}"`);
  });

  await step(run, page, "an unknown route says so", async () => {
    await goTo("/nope");
    await page.waitForFunction(
      () => document.querySelector(".nd-state")?.textContent === "[EMPTY: Page not found]",
      null,
      { timeout: 5000 }
    );
    await page.click('nav.sidebar a[href="/"]');
  });
```

(k) In `tests/webkit/layout.mjs`, replace the `RAIL_OVERLAP_CASES` constant with the following. Each case's `path` lists the rail link first, then the tabs to click.

```js
// Each case: the links that open a page (the rail link, then any tab), how
// to surface a page-level z-indexed element near the rail (some need a click
// to open), and that element's selector.
const TAB_ROUTES = [
  ["/filament", "/filament"],
  ["/filament", "/filament/batch"],
  ["/profiles", "/profiles"],
  ["/profiles", "/profiles/compare"],
  ["/settings", "/settings"],
  ["/settings", "/settings/health"],
  ["/settings", "/settings/about"],
];
const RAIL_OVERLAP_CASES = [
  {
    route: "/filament",
    path: ["/filament"],
    reveal: async () => {},
    overlapSelector: ".search-container",
  },
  {
    route: "/profiles/compare",
    path: ["/profiles", "/profiles/compare"],
    reveal: async (page) => {
      await page.locator(".ss-display").first().click();
    },
    overlapSelector: ".ss-dropdown",
  },
  // The tab row sits right against the rail on every tabbed hub.
  ...TAB_ROUTES.map(([hub, tab]) => ({
    route: tab,
    path: hub === tab ? [hub] : [hub, tab],
    reveal: async () => {},
    overlapSelector: ".nd-tabs",
  })),
];
```

In `checkRailOverlap`, change the loop header to `for (const { route, path, reveal, overlapSelector } of RAIL_OVERLAP_CASES) {` and replace `await page.click(\`a[href="${route}"]\`);` with:

```js
      const [hub, ...tabs] = path;
      await page.click(`nav.sidebar a[href="${hub}"]`);
      for (const tab of tabs) await page.click(`.nd-tabs a[href="${tab}"]`);
```

(l) In `tests/webkit/screens.mjs`, replace the `ROUTES` line with:

```js
const ROUTES = [
  "/",
  "/printer",
  "/filament",
  "/filament/batch",
  "/profiles",
  "/profiles/compare",
  "/slice",
  "/analysis",
  "/settings",
  "/settings/health",
  "/settings/about",
];
// A tab route is reached through its hub: the hub's rail link, then the tab.
const HUB_OF = {
  "/filament/batch": "/filament",
  "/profiles/compare": "/profiles",
  "/settings/health": "/settings",
  "/settings/about": "/settings",
};
```

Replace `const name = route === "/" ? "home" : route.slice(1);` with `const name = route === "/" ? "home" : route.slice(1).replaceAll("/", "-");`.

Replace `await page.click(\`a.nav-link[href="${route}"]\`);` with:

```js
        const hub = HUB_OF[route];
        await page.click(`a.nav-link[href="${hub ?? route}"]`);
        if (hub) await page.click(`.nd-tabs a[href="${route}"]`);
```

- [ ] **Step 10: Run the browser tests**

Run: `trunk build && (cd tests/webkit && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected:
- layout prints `PASS: the expanded rail is never covered by page content in either engine.`, with OK lines for all 9 cases in both engines;
- contrast passes;
- app-flows reports every step passing in both engines, including the three new hub steps.

If a step that clicks through Settings fails because Settings → General now mounts first, use `goTo(...)` to reach the tab directly instead.

- [ ] **Step 11: Commit**

```bash
git add src/routes.rs src/pages/hub.rs src/pages/mod.rs src/components/sidebar.rs src/components/icons.rs src/app.rs style/main.css tests/webkit/app-flows.mjs tests/webkit/layout.mjs tests/webkit/screens.mjs
git commit -m "feat(ui): seven-hub rail with tab routes and redirects for the old routes"
```

---

### Task 3: The agent's route list

**Files:**
- Modify: `src-tauri/src/agent/tools/app.rs`

**Interfaces:**
- Consumes: the route list from Task 2. The old routes still resolve through the frontend's redirects.
- Produces: `bm_navigate` accepts the 11 page routes and the 4 old routes. The agent tool count is unchanged.

- [ ] **Step 1: Write the failing test**

In the tests module of `src-tauri/src/agent/tools/app.rs`, add after `navigate_rejects_unknown_routes_and_emits_known_ones`:

```rust
    #[tokio::test]
    async fn navigate_accepts_hub_tabs_and_the_old_routes() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        for route in [
            "/filament/batch",
            "/profiles/compare",
            "/settings/health",
            "/settings/about",
            "/batch",
            "/compare",
            "/health",
            "/about",
        ] {
            assert!(
                reg.call("bm_navigate", json!({ "route": route })).await.ok,
                "{route} was refused"
            );
        }
        assert!(!reg.call("bm_navigate", json!({"route":"/settings/nope"})).await.ok);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml navigate_accepts_hub_tabs`
Expected: FAIL with `/filament/batch was refused`.

- [ ] **Step 3: Extend the route list**

Replace the `ROUTES` constant with:

```rust
/// Pages the agent may show. The hub tabs come first; the routes from
/// before the hubs still work, because the app redirects them to their tab.
pub const ROUTES: &[&str] = &[
    "/",
    "/printer",
    "/filament",
    "/filament/batch",
    "/profiles",
    "/profiles/compare",
    "/slice",
    "/analysis",
    "/settings",
    "/settings/health",
    "/settings/about",
    "/batch",
    "/compare",
    "/health",
    "/about",
];
```

Replace the `bm_navigate` description string with:

```rust
            description: "Move the BambuMate UI to a page so the user can follow along. Routes: /, /printer, /filament, /filament/batch, /profiles, /profiles/compare, /slice, /analysis, /settings, /settings/health, /settings/about. The older /batch, /compare, /health and /about still work.",
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::tools`
Expected: all pass, including `total_tool_count_stays_small`; the counts stay at 22, 22 and 23.

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/tools/app.rs
git commit -m "feat(agent): bm_navigate knows the hub tab routes and keeps the old ones"
```

---

### Task 4: Keyboard-accessible STL indicator

**Files:**
- Modify: `src/components/stl_indicator.rs`, `src/components/icons.rs`, `style/main.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Produces:
  - `.stl-badge` becomes a `<button type="button">`, with `aria-expanded`, `aria-haspopup="true"` and `aria-label` set by `stl_label(n)`.
  - Escape anywhere inside `.stl-indicator` closes the dropdown and puts focus back on the button.
  - `fn stl_label(count: usize) -> String`.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `src/components/stl_indicator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_counts_the_waiting_files() {
        assert_eq!(stl_label(1), "1 STL file waiting");
        assert_eq!(stl_label(3), "3 STL files waiting");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --bin bambumate stl_indicator`
Expected: compile error: `cannot find function stl_label`.

- [ ] **Step 3: Add the STL icon**

In `src/components/icons.rs`, add `Stl,` to `IconKind` after `Slice,`, and add this arm to the `match kind` after the `IconKind::Slice` arm:

```rust
        IconKind::Stl => view! { <path d="M12 3.5l7.5 4.25v8.5L12 20.5l-7.5-4.25v-8.5z"/><path d="M4.5 7.75L12 12l7.5-4.25M12 12v8.5"/> }.into_any(),
```

- [ ] **Step 4: Make the badge a button**

In `src/components/stl_indicator.rs`:

(a) Add these imports:

```rust
use leptos::html;

use crate::components::icons::{Icon, IconKind};
```

(b) Add this function above `#[component] pub fn StlIndicator`:

```rust
/// The button's accessible name.
fn stl_label(count: usize) -> String {
    match count {
        1 => "1 STL file waiting".into(),
        n => format!("{n} STL files waiting"),
    }
}
```

(c) Inside `StlIndicator`, after `let slicer = expect_context::<SlicerShared>();`, add:

```rust
    let badge = NodeRef::<html::Button>::new();
    // Escape from anywhere in the list closes it and returns focus to the
    // button that opened it.
    let on_keydown = move |e: web_sys::KeyboardEvent| {
        if e.key() == "Escape" && expanded.get_untracked() {
            e.prevent_default();
            set_expanded.set(false);
            if let Some(b) = badge.get() {
                let _ = b.focus();
            }
        }
    };
```

(d) In the `view!`, replace

```rust
            <div class="stl-indicator">
                <div
                    class="stl-badge"
                    on:click=move |_| set_expanded.update(|e| *e = !*e)
                    title="STL files received"
                >
                    <span class="stl-badge-icon">"STL"</span>
                    <span class="stl-badge-count">{move || stl_files.get().len()}</span>
                </div>
```

with

```rust
            <div class="stl-indicator" on:keydown=on_keydown>
                <button
                    type="button"
                    class="stl-badge"
                    node_ref=badge
                    aria-haspopup="true"
                    aria-expanded=move || expanded.get().to_string()
                    aria-label=move || stl_files.with(|f| stl_label(f.len()))
                    title="STL files received"
                    on:click=move |_| set_expanded.update(|e| *e = !*e)
                >
                    <span class="stl-badge-icon" aria-hidden="true"><Icon kind=IconKind::Stl /></span>
                    <span class="stl-badge-count" aria-hidden="true">{move || stl_files.get().len()}</span>
                </button>
```

- [ ] **Step 5: Style the button**

In `style/main.css`:
- In the `.stl-badge` rule, add `background: transparent;` and `padding: 0;` after `cursor: pointer;`.
- Replace the `.stl-badge-icon { color: inherit; }` rule with:

```css
.stl-badge-icon {
    display: flex;
    color: inherit;
}

.stl-badge-icon svg {
    width: 16px;
    height: 16px;
}

.stl-badge:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}
```

- [ ] **Step 6: Run the unit test and the WASM check**

Run: `cargo test --bin bambumate stl_indicator`
Expected: 1 passed.

Run: `cargo check --target wasm32-unknown-unknown`
Expected: builds.

- [ ] **Step 7: Add the keyboard step to the browser tests**

In `tests/webkit/app-flows.mjs`, inside the `withFixtures({ list_received_stls: inbox }, …)` block and after the step "the STL list shows each file's auto-slice state", add:

```js
      await step(run, page, "the STL indicator opens and closes from the keyboard", async () => {
        const badge = page.locator(".stl-badge");
        // Start closed: the steps above leave the list open.
        if ((await badge.getAttribute("aria-expanded")) === "true") await badge.click();
        await page.waitForSelector(".stl-dropdown", { state: "detached", timeout: 5000 });
        const label = await badge.getAttribute("aria-label");
        if (label !== "1 STL file waiting") throw new Error(`aria-label "${label}"`);
        await badge.focus();
        await page.keyboard.press("Enter");
        await page.waitForSelector(".stl-dropdown", { timeout: 5000 });
        if ((await badge.getAttribute("aria-expanded")) !== "true") throw new Error("aria-expanded is not true");
        await page.keyboard.press("Escape");
        await page.waitForSelector(".stl-dropdown", { state: "detached", timeout: 5000 });
        const focused = await page.evaluate(() => document.activeElement?.classList.contains("stl-badge"));
        if (!focused) throw new Error("focus did not return to the STL button");
      });
```

- [ ] **Step 8: Run the browser tests**

Run: `trunk build && (cd tests/webkit && node layout.mjs ../.. && node app-flows.mjs ../..)`
Expected: all steps pass in both engines, including the new STL keyboard step and the two existing STL steps, which still click `.stl-badge`.

- [ ] **Step 9: Commit**

```bash
git add src/components/stl_indicator.rs src/components/icons.rs style/main.css tests/webkit/app-flows.mjs
git commit -m "feat(ui): STL indicator is an icon button that works from the keyboard"
```

---

### Task 5: `latest_generated_preset` command

**Files:**
- Modify: `src-tauri/src/history/types.rs`, `src-tauri/src/history/store.rs`, `src-tauri/src/history/ledger.rs`, `src-tauri/src/commands/preset_sync.rs`, `src-tauri/src/lib.rs`, `src/commands.rs`, `tests/webkit/fixtures.mjs`

**Interfaces:**
- Produces:
  - Backend: `history::GeneratedPreset { profile_name: String, created_at: i64 }` (Unix seconds); `RefinementHistory::latest_generated_preset()`; `ledger::latest_generated_preset()` and `ledger::latest_generated_preset_at(&Path)`.
  - Tauri command: `latest_generated_preset`.
  - Frontend: `commands::GeneratedPreset` (same fields); `commands::latest_generated_preset() -> Result<Option<GeneratedPreset>, String>`.
  - Fixture: `FIXTURES.latest_generated_preset`.

- [ ] **Step 1: Write the failing tests**

In the tests module of `src-tauri/src/history/store.rs`, add:

```rust
    #[test]
    fn latest_generated_preset_is_the_newest_row() {
        let (store, _dir) = create_test_store();
        assert_eq!(store.latest_generated_preset().unwrap(), None);
        store
            .record_generated_preset("/a.json", "P1", "Acme PLA")
            .unwrap();
        store
            .record_generated_preset("/b.json", "P2", "Acme PETG")
            .unwrap();
        // Same second: the row written last wins.
        let latest = store.latest_generated_preset().unwrap().unwrap();
        assert_eq!(latest.profile_name, "Acme PETG");
        assert!(latest.created_at > 0);
    }
```

In the tests module of `src-tauri/src/history/ledger.rs`, add:

```rust
    #[test]
    fn latest_generated_preset_without_a_database_is_none_and_creates_nothing() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("missing.db");
        assert_eq!(latest_generated_preset_at(&db).unwrap(), None);
        assert!(!db.exists());
    }

    #[test]
    fn latest_generated_preset_reads_the_ledger() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        record_new_preset_at(&db, &preset(tmp.path()));
        let latest = latest_generated_preset_at(&db).unwrap().unwrap();
        assert_eq!(latest.profile_name, "Acme PLA");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml latest_generated_preset`
Expected: compile errors: no method `latest_generated_preset`, no function `latest_generated_preset_at`.

- [ ] **Step 3: Implement the read**

Append to `src-tauri/src/history/types.rs`:

```rust
/// The preset BambuMate most recently wrote as new: the newest row of the
/// `generated_presets` ledger. Shown on Home's "Recent" line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedPreset {
    pub profile_name: String,
    /// Unix seconds.
    pub created_at: i64,
}
```

In `src-tauri/src/history/store.rs`:
- change `use rusqlite::{params, Connection};` to `use rusqlite::{params, Connection, OptionalExtension};`;
- add `GeneratedPreset` to the `use super::types::{…}` list;
- add this method to `impl RefinementHistory`, after `generated_preset_paths`:

```rust
    /// The newest ledger row, if any. Ties on `created_at` go to the row
    /// written last.
    pub fn latest_generated_preset(&self) -> Result<Option<GeneratedPreset>, String> {
        self.conn
            .query_row(
                "SELECT profile_name, created_at FROM generated_presets
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [],
                |row| {
                    Ok(GeneratedPreset {
                        profile_name: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        created_at: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    })
                },
            )
            .optional()
            .map_err(|e| format!("Failed to read the newest generated preset: {}", e))
    }
```

In `src-tauri/src/history/ledger.rs`:
- change `use super::RefinementHistory;` to `use super::{GeneratedPreset, RefinementHistory};`;
- add these after `record_new_preset`:

```rust
/// The preset BambuMate most recently wrote as new, from the database at
/// `db_path`. `None` when there is no database yet; it is not created.
pub fn latest_generated_preset_at(db_path: &Path) -> Result<Option<GeneratedPreset>, String> {
    if !db_path.exists() {
        return Ok(None);
    }
    RefinementHistory::new(db_path)?.latest_generated_preset()
}

/// The preset BambuMate most recently wrote as new, for Home's "Recent"
/// line. `None` with no app data directory.
pub fn latest_generated_preset() -> Result<Option<GeneratedPreset>, String> {
    match history_db_path() {
        Some(db) => latest_generated_preset_at(&db),
        None => Ok(None),
    }
}
```

- [ ] **Step 4: Add the command and register it**

In `src-tauri/src/commands/preset_sync.rs`, add `use crate::history::GeneratedPreset;` to the imports, and append:

```rust
/// The preset BambuMate most recently wrote as new (Home's "Recent" line).
/// Read-only.
#[tauri::command]
pub async fn latest_generated_preset() -> Result<Option<GeneratedPreset>, String> {
    tauri::async_runtime::spawn_blocking(crate::history::ledger::latest_generated_preset)
        .await
        .map_err(|e| format!("ledger read failed: {}", e))?
}
```

In `src-tauri/src/lib.rs`, add `commands::preset_sync::latest_generated_preset,` on the line after `commands::preset_sync::repair_preset_sync,`.

- [ ] **Step 5: Run the backend tests and lint**

Run: `cargo test --manifest-path src-tauri/Cargo.toml latest_generated_preset`
Expected: 3 passed.

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets`
Expected: all pass, no warnings. This includes `no_profile_write_bypasses_the_sync_helpers`; the new code only reads.

- [ ] **Step 6: Add the frontend wrapper and the fixture**

In `src/commands.rs`, directly after the `repair_preset_sync` function, add:

```rust
/// The preset BambuMate most recently wrote as new (Home's "Recent" line).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GeneratedPreset {
    pub profile_name: String,
    /// Unix seconds.
    pub created_at: i64,
}

pub async fn latest_generated_preset() -> Result<Option<GeneratedPreset>, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({})).map_err(|e| e.to_string())?;

    let result = invoke("latest_generated_preset", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}
```

In `tests/webkit/fixtures.mjs`, inside `FIXTURES`, add this directly after the `list_unsynced_presets` entry:

```js
  latest_generated_preset: { profile_name: "Polymaker PETG Red", created_at: 1791158400 },
```

- [ ] **Step 7: Check the frontend builds**

Run: `cargo check --target wasm32-unknown-unknown && cargo test --bin bambumate`
Expected: builds; all tests pass. `latest_generated_preset` is unused until Task 8, so a dead-code warning is expected.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/history/types.rs src-tauri/src/history/store.rs src-tauri/src/history/ledger.rs src-tauri/src/commands/preset_sync.rs src-tauri/src/lib.rs src/commands.rs tests/webkit/fixtures.mjs
git commit -m "feat: read-only latest_generated_preset command for Home's Recent line"
```

---

### Task 6: Remember the last finished print

**Files:**
- Create: `src/printer/last_print.rs`
- Modify: `src/printer/mod.rs`

**Interfaces:**
- Produces:
  - `printer::last_print::LastPrint { name: String, finished_at_ms: f64, duration_ms: Option<f64> }`.
  - `PrintTracker` (`Default`, `Copy`).
  - `fn track_print(t: PrintTracker, gcode: Option<&str>, name: Option<&str>, percent: Option<u32>, now_ms: f64) -> (PrintTracker, Option<LastPrint>)`.
  - `PrinterShared.last_print: RwSignal<Option<LastPrint>>`. It is set by `PrinterEvents` and lives for the session only.

- [ ] **Step 1: Write the failing tests**

Create `src/printer/last_print.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn feed(steps: &[(&str, Option<u32>, f64)]) -> Option<LastPrint> {
        let mut t = PrintTracker::default();
        let mut out = None;
        for &(gcode, percent, now) in steps {
            let (next, done) = track_print(t, Some(gcode), Some("Benchy.3mf"), percent, now);
            t = next;
            out = done.or(out);
        }
        out
    }

    #[test]
    fn a_print_seen_from_its_start_records_its_duration() {
        let p = feed(&[("PREPARE", Some(0), 1_000.0), ("RUNNING", Some(50), 2_000.0), ("FINISH", Some(100), 7_000.0)]).unwrap();
        assert_eq!(p.name, "Benchy.3mf");
        assert_eq!(p.finished_at_ms, 7_000.0);
        assert_eq!(p.duration_ms, Some(6_000.0));
    }

    #[test]
    fn a_print_joined_midway_has_no_duration() {
        let p = feed(&[("RUNNING", Some(40), 1_000.0), ("FINISH", Some(100), 5_000.0)]).unwrap();
        assert_eq!(p.duration_ms, None);
    }

    #[test]
    fn a_pause_does_not_end_the_print() {
        let p = feed(&[("RUNNING", Some(0), 1_000.0), ("PAUSE", Some(30), 2_000.0), ("RUNNING", Some(31), 3_000.0), ("FINISH", Some(100), 4_000.0)]).unwrap();
        assert_eq!(p.duration_ms, Some(3_000.0));
    }

    #[test]
    fn only_running_to_finish_counts() {
        // Failed, never seen running, or idle throughout: nothing recorded.
        assert_eq!(feed(&[("RUNNING", Some(5), 1_000.0), ("FAILED", Some(5), 2_000.0)]), None);
        assert_eq!(feed(&[("FINISH", Some(100), 1_000.0)]), None);
        assert_eq!(feed(&[("IDLE", None, 1_000.0), ("IDLE", None, 2_000.0)]), None);
        // A finished print is reported once, not on every later FINISH report.
        assert_eq!(
            feed(&[("RUNNING", Some(5), 1_000.0), ("FINISH", Some(100), 2_000.0), ("FINISH", Some(100), 3_000.0)])
                .map(|p| p.finished_at_ms),
            Some(2_000.0)
        );
    }
}
```

Add `pub mod last_print;` to `src/printer/mod.rs` after `pub mod bridge;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin bambumate last_print`
Expected: compile errors: `PrintTracker`, `track_print` and `LastPrint` are not found.

- [ ] **Step 3: Implement the tracker**

Put this above the tests module in `src/printer/last_print.rs`:

```rust
//! Remembers the print that finished most recently, for Home's idle hero.
//! Session only: nothing is written to disk.

/// A print the app watched finish.
#[derive(Debug, Clone, PartialEq)]
pub struct LastPrint {
    /// The file name the printer reported. Untrusted: render it through
    /// `untrusted_text`.
    pub name: String,
    pub finished_at_ms: f64,
    /// Known only when the app saw the print start.
    pub duration_ms: Option<f64>,
}

/// What the tracker knows about the job in progress.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PrintTracker {
    /// When the current job was first seen active, if one is.
    active_since_ms: Option<f64>,
    /// Whether that first sighting was the job's start.
    saw_start: bool,
}

/// `gcode_state`s of a job that hasn't ended.
const ACTIVE: &[&str] = &["PREPARE", "SLICING", "RUNNING", "PAUSE"];

/// Feeds one status report to the tracker. Returns the finished print on a
/// transition from an active state to `FINISH`, and nothing otherwise.
pub fn track_print(
    t: PrintTracker,
    gcode: Option<&str>,
    name: Option<&str>,
    percent: Option<u32>,
    now_ms: f64,
) -> (PrintTracker, Option<LastPrint>) {
    let active = gcode.is_some_and(|g| ACTIVE.contains(&g));
    match t.active_since_ms {
        None if active => (
            PrintTracker {
                active_since_ms: Some(now_ms),
                saw_start: gcode == Some("PREPARE") || percent == Some(0),
            },
            None,
        ),
        Some(_) if active => (t, None),
        Some(since) if gcode == Some("FINISH") => (
            PrintTracker::default(),
            Some(LastPrint {
                name: name.unwrap_or_default().to_string(),
                finished_at_ms: now_ms,
                duration_ms: t.saw_start.then_some(now_ms - since),
            }),
        ),
        _ => (PrintTracker::default(), None),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --bin bambumate last_print`
Expected: 4 passed.

- [ ] **Step 5: Feed it from the printer events**

In `src/printer/mod.rs`:

(a) Add `use last_print::{track_print, LastPrint, PrintTracker};` under the existing `use types::…` line.

(b) Add a field to `PrinterShared`, after `view`:

```rust
    /// The print the app last watched finish; `None` until one does.
    /// Session only.
    pub last_print: RwSignal<Option<LastPrint>>,
```

(c) In `PrinterShared::new()`, add `last_print: RwSignal::new(None),` after `view: …`.

(d) In `PrinterEvents`, before the final `shared.refresh();`, add:

```rust
    // Notices a print finishing; see last_print::track_print. A missing
    // status report leaves the tracker as it was.
    let tracker = StoredValue::new(PrintTracker::default());
    let job = Memo::new(move |_| {
        shared.view.with(|v| {
            v.state
                .as_ref()
                .map(|s| (s.gcode_state.clone(), s.subtask_name.clone(), s.mc_percent))
        })
    });
    Effect::new(move |_| {
        let Some((gcode, name, percent)) = job.get() else {
            return;
        };
        let (next, finished) = track_print(
            tracker.get_value(),
            gcode.as_deref(),
            name.as_deref(),
            percent,
            js_sys::Date::now(),
        );
        tracker.set_value(next);
        if let Some(p) = finished {
            shared.last_print.set(Some(p));
        }
    });
```

- [ ] **Step 6: Run the tests and the WASM check**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown`
Expected: all pass; builds.

- [ ] **Step 7: Commit**

```bash
git add src/printer/last_print.rs src/printer/mod.rs
git commit -m "feat(printer): remember the last print the app watched finish"
```

---

### Task 7: Home decisions as pure functions

**Files:**
- Create: `src/pages/home_state.rs`
- Modify: `src/pages/mod.rs`

**Interfaces:**
- Consumes:
  - `PrinterView`, `PrinterState`, `PrinterSummary`, `Nozzle`, `ErrorView`, `ConnectionState`, `format_remaining` and `untrusted_text` from `crate::printer::types`;
  - `LastPrint` (Task 6);
  - `commands::GeneratedPreset` (Task 5).
- Produces:
  - `enum HomeMode { Attention, Printing, Idle, NoPrinter { disconnected: bool } }`
  - `HOME_SEGMENTS: u32 = 12`
  - `home_mode`, `mode_key`
  - labels: `printing_label`, `idle_label`, `attention_label`, `no_printer_label`
  - hero text: `attention_word`, `attention_detail`, `percent_text`, `printing_detail`, `idle_word`
  - `active_slot`
  - footer and agent text: `format_ago`, `last_print_detail`, `recent_line`, `sync_line`, `profiles_detail`, `ask_about`

- [ ] **Step 1: Write the failing tests**

Create `src/pages/home_state.rs` with the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{Nozzle, PrinterState, PrinterSummary};

    fn connected(gcode: &str) -> PrinterView {
        PrinterView {
            configured: true,
            connection: ConnectionState::Connected,
            printer: Some(PrinterSummary {
                name: "Workshop H2D".into(),
                model: "H2D".into(),
                ..Default::default()
            }),
            state: Some(PrinterState {
                gcode_state: Some(gcode.into()),
                subtask_name: Some("Benchy".into()),
                mc_percent: Some(42),
                mc_remaining_time: Some(72),
                layer_num: Some(118),
                total_layer_num: Some(284),
                bed_temp: Some(65.0),
                nozzles: vec![Nozzle {
                    id: 0,
                    temp: Some(220.0),
                    ..Default::default()
                }],
                active_nozzle: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn error(code: &str, text: Option<&str>) -> ErrorView {
        ErrorView {
            kind: "hms".into(),
            code: code.into(),
            text: text.map(Into::into),
            wiki_url: String::new(),
        }
    }

    #[test]
    fn attention_beats_printing_beats_idle_beats_no_printer() {
        let mut v = connected("RUNNING");
        assert_eq!(home_mode(&v), HomeMode::Printing);
        v.errors.push(error("0300_0100", None));
        assert_eq!(home_mode(&v), HomeMode::Attention);
        assert_eq!(home_mode(&connected("PAUSE")), HomeMode::Attention);
        assert_eq!(home_mode(&connected("PREPARE")), HomeMode::Printing);
        assert_eq!(home_mode(&connected("IDLE")), HomeMode::Idle);
        assert_eq!(home_mode(&connected("FINISH")), HomeMode::Idle);
        let waiting = PrinterView { state: None, ..connected("IDLE") };
        assert_eq!(home_mode(&waiting), HomeMode::Idle);
        assert_eq!(home_mode(&PrinterView::default()), HomeMode::NoPrinter { disconnected: false });
    }

    #[test]
    fn a_disconnect_mid_print_never_shows_the_print() {
        let v = PrinterView {
            connection: ConnectionState::Unreachable,
            ..connected("RUNNING")
        };
        assert_eq!(home_mode(&v), HomeMode::NoPrinter { disconnected: true });
        assert_eq!(no_printer_label(&v), "[DISCONNECTED]");
        let connecting = PrinterView {
            connection: ConnectionState::Connecting,
            ..connected("RUNNING")
        };
        assert_eq!(no_printer_label(&connecting), "[CONNECTING…]");
        assert_eq!(no_printer_label(&PrinterView::default()), "BambuMate");
    }

    #[test]
    fn mode_keys() {
        assert_eq!(mode_key(HomeMode::Attention), "attention");
        assert_eq!(mode_key(HomeMode::Printing), "printing");
        assert_eq!(mode_key(HomeMode::Idle), "idle");
        assert_eq!(mode_key(HomeMode::NoPrinter { disconnected: true }), "no-printer");
    }

    #[test]
    fn labels_name_the_printer_once_and_skip_blanks() {
        let v = connected("RUNNING");
        assert_eq!(printing_label(&v), "Workshop H2D · Printing · Benchy");
        assert_eq!(idle_label(&v), "Workshop H2D · Idle");
        let mut e = connected("RUNNING");
        e.errors.push(error("0300_0100", Some("Heatbed")));
        assert_eq!(attention_label(&e), "Workshop H2D · Needs attention · 0300_0100");
        let mut unnamed = connected("IDLE");
        unnamed.printer.as_mut().unwrap().name = String::new();
        assert_eq!(idle_label(&unnamed), "H2D · Idle");
    }

    #[test]
    fn printing_hero_text() {
        let v = connected("RUNNING");
        assert_eq!(percent_text(&v), "42");
        assert_eq!(
            printing_detail(&v),
            "1 h 12 m left · Layer 118 / 284 · Nozzle 220 °C · Bed 65 °C"
        );
        let mut over = connected("RUNNING");
        over.state.as_mut().unwrap().mc_percent = Some(140);
        assert_eq!(percent_text(&over), "100");
        assert_eq!(percent_text(&PrinterView::default()), "—");
    }

    #[test]
    fn attention_hero_text() {
        let mut v = connected("RUNNING");
        v.errors.push(error("0300_0100", Some("The heatbed is too hot.")));
        assert_eq!(attention_word(&v), "ERROR");
        assert_eq!(attention_detail(&v), "The heatbed is too hot.");
        let mut no_text = connected("RUNNING");
        no_text.errors.push(error("0300_400C", None));
        assert_eq!(attention_detail(&no_text), "Look up 0300_400C on the Bambu wiki.");
        let paused = connected("PAUSE");
        assert_eq!(attention_word(&paused), "PAUSED");
        assert_eq!(attention_detail(&paused), "Paused. Check the printer, then resume the print there.");
    }

    #[test]
    fn idle_hero_word_waits_for_the_first_report() {
        assert_eq!(idle_word(&connected("IDLE")), "READY");
        assert_eq!(idle_word(&PrinterView { state: None, ..connected("IDLE") }), "WAITING");
    }

    #[test]
    fn the_slot_in_use_comes_from_tray_now() {
        let mut v = connected("RUNNING");
        assert_eq!(active_slot(&v), None);
        v.state.as_mut().unwrap().tray_now = Some(6);
        assert_eq!(active_slot(&v), Some((1, 2)));
        v.state.as_mut().unwrap().tray_now = Some(255);
        assert_eq!(active_slot(&v), None);
    }

    #[test]
    fn ago_text() {
        let now = 10_000_000_000.0;
        assert_eq!(format_ago(now, now - 30_000.0), "just now");
        assert_eq!(format_ago(now, now - 5.0 * 60_000.0), "5m ago");
        assert_eq!(format_ago(now, now - 3.0 * 3_600_000.0), "3h ago");
        assert_eq!(format_ago(now, now - 2.0 * 86_400_000.0), "2d ago");
        assert_eq!(format_ago(now, now + 60_000.0), "just now");
    }

    #[test]
    fn last_print_line() {
        let now = 10_000_000_000.0;
        let p = LastPrint {
            name: "Benchy".into(),
            finished_at_ms: now - 3.0 * 3_600_000.0,
            duration_ms: Some(118.0 * 60_000.0),
        };
        assert_eq!(last_print_detail(Some(&p), now), "Last print finished 3h ago · Benchy · 1 h 58 m");
        let unknown = LastPrint { duration_ms: None, ..p };
        assert_eq!(last_print_detail(Some(&unknown), now), "Last print finished 3h ago · Benchy");
        assert_eq!(last_print_detail(None, now), "");
    }

    #[test]
    fn footer_and_profile_lines() {
        let p = GeneratedPreset {
            profile_name: "Polymaker PETG Red".into(),
            created_at: 1_000_000,
        };
        assert_eq!(
            recent_line(&p, (1_000_000.0 + 2.0 * 86_400.0) * 1000.0),
            "Recent · Polymaker PETG Red · 2d ago"
        );
        assert_eq!(sync_line(0), "All presets synced");
        assert_eq!(sync_line(1), "1 preset unsynced");
        assert_eq!(sync_line(3), "3 presets unsynced");
        assert_eq!(profiles_detail(12, 3), "9 synced to your Bambu account");
        assert_eq!(profiles_detail(2, 5), "0 synced to your Bambu account");
    }

    #[test]
    fn the_agent_question_names_the_error() {
        let mut v = connected("RUNNING");
        v.errors.push(error("0300_0100", Some("Heatbed abnormal.")));
        assert_eq!(
            ask_about(&v),
            "My printer shows error 0300_0100: \"Heatbed abnormal.\". What does it mean, and how do I fix it?"
        );
        let mut bare = connected("RUNNING");
        bare.errors.push(error("0300_400C", None));
        assert_eq!(
            ask_about(&bare),
            "My printer shows error 0300_400C. What does it mean, and how do I fix it?"
        );
        assert_eq!(
            ask_about(&connected("PAUSE")),
            "My printer paused mid-print. What usually causes that, and what should I check?"
        );
    }
}
```

Add `pub mod home_state;` to `src/pages/mod.rs` after `pub mod home;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin bambumate home_state`
Expected: compile errors for every missing function and type.

- [ ] **Step 3: Implement the helpers**

Put this above the tests module in `src/pages/home_state.rs`:

```rust
//! What Home shows, worked out from the printer view and plain data so it
//! can be tested without a browser (spec 2026-10-17 §3).

use crate::commands::GeneratedPreset;
use crate::printer::last_print::LastPrint;
use crate::printer::types::{
    format_remaining, untrusted_text, ConnectionState, ErrorView, PrinterView,
};

/// Segments in Home's progress bar.
pub const HOME_SEGMENTS: u32 = 12;

/// Most characters shown of a printer name the printer reports.
const NAME_MAX: usize = 64;

/// Most characters shown of a file name the printer reports.
const FILE_MAX: usize = 120;

/// `gcode_state`s that mean a print is under way.
const PRINTING: &[&str] = &["PREPARE", "SLICING", "RUNNING"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeMode {
    /// Paused, or the printer reports an error.
    Attention,
    Printing,
    Idle,
    /// No printer set up (`disconnected: false`), or set up but not connected.
    NoPrinter { disconnected: bool },
}

/// Which hero Home leads with. Precedence: attention, printing, idle, no
/// printer. A printer that isn't connected never shows its last state.
pub fn home_mode(v: &PrinterView) -> HomeMode {
    if !v.configured {
        return HomeMode::NoPrinter { disconnected: false };
    }
    if v.connection != ConnectionState::Connected {
        return HomeMode::NoPrinter { disconnected: true };
    }
    let gcode = v.state.as_ref().and_then(|s| s.gcode_state.as_deref());
    if gcode == Some("PAUSE") || !v.errors.is_empty() {
        return HomeMode::Attention;
    }
    if gcode.is_some_and(|g| PRINTING.contains(&g)) {
        return HomeMode::Printing;
    }
    HomeMode::Idle
}

/// `data-mode` on the Home page.
pub fn mode_key(mode: HomeMode) -> &'static str {
    match mode {
        HomeMode::Attention => "attention",
        HomeMode::Printing => "printing",
        HomeMode::Idle => "idle",
        HomeMode::NoPrinter { .. } => "no-printer",
    }
}

/// The printer's name, else its model; cleaned for display.
fn printer_name(v: &PrinterView) -> String {
    v.printer
        .as_ref()
        .map(|p| {
            let name = if p.name.trim().is_empty() { &p.model } else { &p.name };
            untrusted_text(name, NAME_MAX)
        })
        .unwrap_or_default()
}

/// The file being printed; cleaned for display.
fn file_name(v: &PrinterView) -> String {
    v.state
        .as_ref()
        .and_then(|s| s.subtask_name.as_deref())
        .filter(|f| !f.is_empty())
        .map(|f| untrusted_text(f, FILE_MAX))
        .unwrap_or_default()
}

/// Joins the non-empty parts with ` · `.
fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn printing_label(v: &PrinterView) -> String {
    join(&[printer_name(v), "Printing".into(), file_name(v)])
}

pub fn idle_label(v: &PrinterView) -> String {
    join(&[printer_name(v), "Idle".into()])
}

pub fn attention_label(v: &PrinterView) -> String {
    let code = v.errors.first().map(|e| e.code.clone()).unwrap_or_default();
    join(&[printer_name(v), "Needs attention".into(), code])
}

/// The hero label with no live printer.
pub fn no_printer_label(v: &PrinterView) -> String {
    if !v.configured {
        "BambuMate".into()
    } else if v.connection == ConnectionState::Connecting {
        "[CONNECTING…]".into()
    } else {
        "[DISCONNECTED]".into()
    }
}

pub fn attention_word(v: &PrinterView) -> &'static str {
    match v.state.as_ref().and_then(|s| s.gcode_state.as_deref()) {
        Some("PAUSE") => "PAUSED",
        _ => "ERROR",
    }
}

pub fn attention_detail(v: &PrinterView) -> String {
    match v.errors.first() {
        Some(ErrorView { text: Some(t), .. }) => t.clone(),
        Some(e) => format!("Look up {} on the Bambu wiki.", e.code),
        None => "Paused. Check the printer, then resume the print there.".into(),
    }
}

/// The percentage, capped at 100; `—` with no report.
pub fn percent_text(v: &PrinterView) -> String {
    v.state
        .as_ref()
        .and_then(|s| s.mc_percent)
        .map(|p| p.min(100).to_string())
        .unwrap_or_else(|| "—".into())
}

/// `1 h 12 m left · Layer 118 / 284 · Nozzle 220 °C · Bed 65 °C`, leaving
/// out what the printer didn't report.
pub fn printing_detail(v: &PrinterView) -> String {
    let Some(s) = &v.state else {
        return String::new();
    };
    let nozzle = s
        .nozzles
        .iter()
        .find(|n| Some(n.id) == s.active_nozzle)
        .or_else(|| s.nozzles.first())
        .and_then(|n| n.temp);
    join(&[
        s.mc_remaining_time
            .map(|m| format!("{} left", format_remaining(m)))
            .unwrap_or_default(),
        match (s.layer_num, s.total_layer_num) {
            (Some(l), Some(t)) if t > 0 => format!("Layer {l} / {t}"),
            _ => String::new(),
        },
        nozzle.map(|t| format!("Nozzle {t:.0} °C")).unwrap_or_default(),
        s.bed_temp.map(|t| format!("Bed {t:.0} °C")).unwrap_or_default(),
    ])
}

/// `READY`, or `WAITING` until the first status report arrives.
pub fn idle_word(v: &PrinterView) -> &'static str {
    if v.state.is_some() {
        "READY"
    } else {
        "WAITING"
    }
}

/// The `(ams_id, tray_id)` feeding the print. `tray_now` counts trays across
/// the four-slot AMS units (0-15); higher values mean an external spool or
/// none, which Home doesn't outline.
pub fn active_slot(v: &PrinterView) -> Option<(u32, u32)> {
    match v.state.as_ref()?.tray_now? {
        n @ 0..=15 => Some((n / 4, n % 4)),
        _ => None,
    }
}

/// `just now`, `5m ago`, `3h ago` or `2d ago`.
pub fn format_ago(now_ms: f64, then_ms: f64) -> String {
    let secs = ((now_ms - then_ms) / 1000.0).max(0.0) as u64;
    match secs {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{}m ago", secs / 60),
        3_600..=86_399 => format!("{}h ago", secs / 3_600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

/// `Last print finished 3h ago · Benchy · 1 h 58 m`; empty with no print.
pub fn last_print_detail(p: Option<&LastPrint>, now_ms: f64) -> String {
    let Some(p) = p else {
        return String::new();
    };
    join(&[
        format!("Last print finished {}", format_ago(now_ms, p.finished_at_ms)),
        untrusted_text(&p.name, FILE_MAX),
        p.duration_ms
            .map(|d| format_remaining((d / 60_000.0).round() as u32))
            .unwrap_or_default(),
    ])
}

/// `Recent · Polymaker PETG Red · 2d ago`.
pub fn recent_line(p: &GeneratedPreset, now_ms: f64) -> String {
    format!(
        "Recent · {} · {}",
        p.profile_name,
        format_ago(now_ms, p.created_at as f64 * 1000.0)
    )
}

pub fn sync_line(unsynced: usize) -> String {
    match unsynced {
        0 => "All presets synced".into(),
        1 => "1 preset unsynced".into(),
        n => format!("{n} presets unsynced"),
    }
}

pub fn profiles_detail(total: usize, unsynced: usize) -> String {
    format!(
        "{} synced to your Bambu account",
        total.saturating_sub(unsynced)
    )
}

/// The question "What does this mean?" puts in the agent's composer.
pub fn ask_about(v: &PrinterView) -> String {
    match v.errors.first() {
        Some(ErrorView {
            code,
            text: Some(t),
            ..
        }) => format!("My printer shows error {code}: \"{t}\". What does it mean, and how do I fix it?"),
        Some(e) => format!(
            "My printer shows error {}. What does it mean, and how do I fix it?",
            e.code
        ),
        None => {
            "My printer paused mid-print. What usually causes that, and what should I check?".into()
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --bin bambumate home_state`
Expected: 12 passed.

- [ ] **Step 5: Commit**

```bash
git add src/pages/home_state.rs src/pages/mod.rs
git commit -m "feat(home): cockpit decisions as tested pure functions"
```

---

### Task 8: The Home cockpit

**Files:**
- Rewrite: `src/pages/home.rs`
- Create: `style/home.css`
- Modify:
  - `index.html`;
  - `src/agent/drawer.rs`;
  - `src/pages/printer.rs`;
  - `src/components/branding.rs`;
  - `style/main.css`, to remove the old Home rules;
  - `tests/webkit/app-flows.mjs`.

**Interfaces:**
- Consumes:
  - from Task 1: `Hero`, `SegmentedBar`, `StateText`, `StateKind`, `Tone`;
  - from Task 7: every `home_state` function;
  - from Task 6: `PrinterShared.last_print`;
  - from Task 5: `commands::latest_generated_preset`;
  - existing: `commands::list_profiles`, `commands::list_unsynced_presets`, `AgentRefresh`, `FeatureFlagsContext`.
- Produces:
  - `AgentShared.prefill: RwSignal<Option<String>>`;
  - `pub(crate) SlotPicker`;
  - Home markup:
    - `.home-page[data-mode]`;
    - `.hm-hero` holding a `.nd-hero`;
    - `.hm-actions`, `.hm-ask`, `.hm-connect`;
    - `.hm-ams` with `.hm-slot[data-slot-label][data-status][data-in-use]` and `.hm-slot-main` inside;
    - `.hm-foot` with `.hm-recent` and `.hm-sync`.

- [ ] **Step 1: Let pages put a question in the agent composer**

In `src/agent/drawer.rs`:

(a) Add a field to `AgentShared`, after `slice`:

```rust
    /// A question another page wants in the composer (Home's "What does
    /// this mean?"). The drawer moves it into the draft and clears it; the
    /// user still presses send.
    pub prefill: RwSignal<Option<String>>,
```

(b) In `impl Default for AgentShared`, add `prefill: RwSignal::new(None),`.

(c) In `AgentDrawer`, change the destructure to:

```rust
    let AgentShared {
        chat,
        open,
        provider,
        slice,
        prefill,
    } = expect_context::<AgentShared>();
```

(d) Directly after `let draft = RwSignal::new(String::new());`, add:

```rust
    Effect::new(move |_| {
        if let Some(text) = prefill.get() {
            draft.set(text);
            prefill.set(None);
        }
    });
```

- [ ] **Step 2: Share the printer's slot picker**

In `src/pages/printer.rs`:

(a) Change `fn SlotPicker(` to `pub(crate) fn SlotPicker(`.

(b) In `SlotCard`'s `view!`, add `data-slot-label=move || item.with(|s| s.label.clone())` to the `<div class="pr-slot" …>` element, next to the existing `data-label`.

(c) Replace `refocus_card` with:

```rust
/// Puts focus back on the slot a picker was opened from, on whichever page
/// it was opened (Printer or Home): the first button inside the element
/// whose `data-slot-label` matches.
fn refocus_card(label: String) {
    set_timeout(
        move || {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Ok(cards) = doc.query_selector_all("[data-slot-label]") else {
                return;
            };
            for i in 0..cards.length() {
                let Some(card) = cards
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                if card.get_attribute("data-slot-label").as_deref() == Some(label.as_str()) {
                    if let Some(button) = card
                        .query_selector("button")
                        .ok()
                        .flatten()
                        .and_then(|b| b.dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        let _ = button.focus();
                    }
                    return;
                }
            }
        },
        Duration::ZERO,
    );
}
```

Run: `cargo test --bin bambumate printer && cargo check --target wasm32-unknown-unknown`
Expected: all pass; builds.

- [ ] **Step 3: Rewrite Home**

Replace the whole of `src/pages/home.rs` with:

```rust
//! Home: a cockpit that leads with whatever matters now (spec 2026-10-17
//! §3). The printer decides the hero (see `home_state::home_mode`); below
//! it come the AMS slots and a footer with recent work and sync status.
//! Read-only: nothing here sends a command to the printer.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::agent::drawer::{AgentRefresh, AgentShared};
use crate::app::FeatureFlagsContext;
use crate::commands::{self, GeneratedPreset};
use crate::components::patterns::{Hero, SegmentedBar, StateKind, StateText, Tone};
use crate::pages::home_state::{
    active_slot, ask_about, attention_detail, attention_label, attention_word, home_mode,
    idle_label, idle_word, last_print_detail, mode_key, no_printer_label, percent_text,
    printing_detail, printing_label, profiles_detail, recent_line, sync_line, HomeMode,
    HOME_SEGMENTS,
};
use crate::pages::printer::SlotPicker;
use crate::printer::types::{swatch_color, untrusted_text, ConnectionState, SlotView};
use crate::printer::PrinterShared;

/// Most characters shown of a filament type the printer reports.
const MATERIAL_MAX: usize = 16;

#[component]
pub fn HomePage() -> impl IntoView {
    let printer = expect_context::<PrinterShared>();
    let view = printer.view;
    let last_print = printer.last_print;
    let flags = expect_context::<FeatureFlagsContext>().flags;
    let agent = expect_context::<AgentShared>();
    let refresh = expect_context::<AgentRefresh>().0;
    // Picks up anything missed while away.
    printer.refresh();

    let mode = Memo::new(move |_| view.with(home_mode));
    let connected = Memo::new(move |_| {
        view.with(|v| v.configured && v.connection == ConnectionState::Connected)
    });
    let slot_labels = Memo::new(move |_| {
        view.with(|v| v.slots.iter().map(|s| s.label.clone()).collect::<Vec<_>>())
    });
    let in_use = Memo::new(move |_| view.with(active_slot));
    let picking = RwSignal::new(Option::<SlotView>::None);

    // `None` until each call answers. They run on mount and again when the
    // agent changes profiles; never on a timer.
    let profiles = RwSignal::new(None::<Result<usize, String>>);
    let unsynced = RwSignal::new(None::<Result<usize, String>>);
    let recent = RwSignal::new(None::<Result<Option<GeneratedPreset>, String>>);
    Effect::new(move |_| {
        refresh.track();
        // Home can go away while these are in flight, so each write is a
        // try_set, which does nothing once the signal is disposed.
        spawn_local(async move {
            profiles.try_set(Some(commands::list_profiles().await.map(|l| l.len())));
        });
        spawn_local(async move {
            unsynced.try_set(Some(
                commands::list_unsynced_presets()
                    .await
                    .map(|l| l.presets.len()),
            ));
        });
        spawn_local(async move {
            recent.try_set(Some(commands::latest_generated_preset().await));
        });
    });

    let ask = move |_| {
        agent.prefill.set(Some(view.with_untracked(ask_about)));
        agent.open.set(true);
    };

    let hero = move || match mode.get() {
        HomeMode::Attention => view! {
            <Hero
                label=Signal::derive(move || view.with(attention_label))
                value=Signal::derive(move || view.with(|v| attention_word(v).to_string()))
                detail=Signal::derive(move || view.with(attention_detail))
                tone=Tone::Accent
            />
            <div class="hm-actions">
                <a href="/printer" class="btn btn-primary">"Open printer"</a>
                <button type="button" class="btn btn-secondary hm-ask" on:click=ask>
                    "What does this mean?"
                </button>
            </div>
        }
        .into_any(),
        HomeMode::Printing => view! {
            <Hero
                label=Signal::derive(move || view.with(printing_label))
                value=Signal::derive(move || view.with(percent_text))
                unit="%"
                detail=Signal::derive(move || view.with(printing_detail))
            />
            <SegmentedBar
                percent=Signal::derive(move || {
                    view.with(|v| v.state.as_ref().and_then(|s| s.mc_percent).unwrap_or(0))
                })
                total=HOME_SEGMENTS
            />
            <div class="hm-actions">
                <a href="/printer" class="btn btn-secondary">"Printer →"</a>
            </div>
        }
        .into_any(),
        HomeMode::Idle => view! {
            <Hero
                label=Signal::derive(move || view.with(idle_label))
                value=Signal::derive(move || view.with(|v| idle_word(v).to_string()))
                detail=Signal::derive(move || {
                    last_print.with(|p| last_print_detail(p.as_ref(), js_sys::Date::now()))
                })
            />
            <div class="hm-actions">
                <a href="/slice" class="btn btn-primary">"Slice a model"</a>
                <a href="/filament" class="btn btn-secondary">"New filament profile"</a>
                <Show when=move || flags.get().analysis_enabled>
                    <a href="/analysis" class="btn btn-secondary">"Analyze last print"</a>
                </Show>
            </div>
        }
        .into_any(),
        HomeMode::NoPrinter { disconnected } => view! {
            <Hero
                label=Signal::derive(move || view.with(no_printer_label))
                value=Signal::derive(move || match profiles.get() {
                    Some(Ok(n)) => n.to_string(),
                    Some(Err(_)) => "—".to_string(),
                    None => "…".to_string(),
                })
                unit=" profiles"
                detail=Signal::derive(move || match (profiles.get(), unsynced.get()) {
                    (Some(Ok(total)), Some(Ok(u))) => profiles_detail(total, u),
                    _ => String::new(),
                })
            />
            {move || match profiles.get() {
                Some(Err(e)) => Some(view! { <StateText kind=StateKind::Error message=e /> }),
                _ => None,
            }}
            <div class="hm-actions">
                <a href="/filament" class="btn btn-primary">"New filament profile"</a>
                <a href="/slice" class="btn btn-secondary">"Slice a model"</a>
            </div>
            <p class="hm-connect-line">
                {if disconnected {
                    "The printer is offline. "
                } else {
                    "Connect your printer to see live status and the AMS here. "
                }}
                <a href="/settings#printer" class="hm-connect">
                    {if disconnected { "Check Settings → Printer →" } else { "Connect →" }}
                </a>
            </p>
        }
        .into_any(),
    };

    view! {
        <div class="page home-page" data-mode=move || mode_key(mode.get())>
            <h1 class="nd-visually-hidden">"Home"</h1>
            <section class="hm-hero">{hero}</section>

            <Show when=move || connected.get() && slot_labels.with(|l| !l.is_empty())>
                <section class="hm-ams" aria-label="AMS">
                    <p class="nd-label">"AMS"</p>
                    <div class="hm-slots">
                        <For
                            each=move || slot_labels.get()
                            key=|label| label.clone()
                            children=move |label| {
                                let item = Memo::new(move |_| {
                                    view.with(|v| {
                                        v.slots.iter().find(|s| s.label == label).cloned().unwrap_or_default()
                                    })
                                });
                                view! { <HomeSlot item=item in_use=in_use mode=mode picking=picking /> }
                            }
                        />
                    </div>
                </section>
            </Show>

            <footer class="hm-foot">
                <div class="hm-recent">
                    {move || match recent.get() {
                        Some(Ok(Some(p))) => Some(view! {
                            <span>{recent_line(&p, js_sys::Date::now())}</span>
                        }.into_any()),
                        Some(Err(e)) => Some(view! { <StateText kind=StateKind::Error message=e /> }.into_any()),
                        _ => None,
                    }}
                </div>
                <div class="hm-sync">
                    {move || match unsynced.get() {
                        Some(Ok(0)) => view! { <span class="hm-sync-ok">{sync_line(0)}</span> }.into_any(),
                        Some(Ok(n)) => view! {
                            <a href="/settings/health" class="hm-sync-warn">{sync_line(n)}</a>
                        }.into_any(),
                        Some(Err(e)) => view! { <StateText kind=StateKind::Error message=e /> }.into_any(),
                        None => view! { <StateText kind=StateKind::Loading /> }.into_any(),
                    }}
                </div>
            </footer>

            {move || picking.get().map(|slot| view! { <SlotPicker item=slot picking=picking /> })}
        </div>
    }
}

/// One AMS slot. Clicking it opens the same preset picker as the Printer
/// page. The slot feeding the print is outlined: signal while printing,
/// accent when the printer needs attention.
#[component]
fn HomeSlot(
    item: Memo<SlotView>,
    in_use: Memo<Option<(u32, u32)>>,
    mode: Memo<HomeMode>,
    picking: RwSignal<Option<SlotView>>,
) -> impl IntoView {
    let outline = move || {
        let here = item.with(|s| Some((s.ams_id, s.tray_id))) == in_use.get();
        match (here, mode.get()) {
            (true, HomeMode::Printing) => Some("signal"),
            (true, HomeMode::Attention) => Some("accent"),
            _ => None,
        }
    };
    view! {
        <div class="hm-slot"
            data-slot-label=move || item.with(|s| s.label.clone())
            data-status=move || item.with(|s| s.status.key())
            data-in-use=outline>
            <button type="button" class="hm-slot-main" title="Choose the preset loaded here"
                on:click=move |_| picking.set(Some(item.get_untracked()))>
                <span class="hm-swatch" style:background=move || item.with(|s| swatch_color(&s.tray.tray_color))></span>
                <span class="hm-slot-label nd-mono">{move || item.with(|s| s.label.clone())}</span>
                <span class="hm-material">
                    {move || item.with(|s| {
                        if s.tray.empty { "—".to_string() } else { untrusted_text(&s.tray.tray_type, MATERIAL_MAX) }
                    })}
                </span>
                <span class="hm-slot-status">{move || item.with(|s| s.status.badge())}</span>
            </button>
        </div>
    }
}
```

- [ ] **Step 4: Add the Home styles and link them**

Create `style/home.css`:

```css
/* Home cockpit (spec 2026-10-17 §3). Shared pieces (hero, segmented bar,
   state text) are styled in the "Page patterns" section of main.css. */

.home-page {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-xl);
    max-width: 1040px;
}

.hm-hero {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-md);
}

.hm-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-sm);
}

.hm-connect-line {
    font-size: 13px;
    color: var(--nd-text-secondary);
}

.hm-connect {
    color: var(--nd-interactive);
}

.hm-ams {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-sm);
}

.hm-slots {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: var(--nd-space-sm);
}

.hm-slot-main {
    width: 100%;
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 2px var(--nd-space-sm);
    padding: var(--nd-space-sm) 10px;
    background: transparent;
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-input);
    color: var(--nd-text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color 150ms var(--nd-ease);
}

.hm-slot-main:hover {
    border-color: var(--nd-border-visible);
}

.hm-slot-main:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.hm-slot[data-in-use="signal"] .hm-slot-main {
    border-color: var(--nd-signal);
}

.hm-slot[data-in-use="accent"] .hm-slot-main {
    border-color: var(--nd-accent);
}

.hm-swatch {
    grid-row: span 2;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1px solid var(--nd-border-visible);
}

.hm-slot-label {
    font-size: 11px;
    color: var(--nd-text-secondary);
}

.hm-material {
    grid-column: 2;
    font-family: var(--nd-font-mono);
    font-size: 12px;
}

.hm-slot-status {
    grid-column: 1 / -1;
    font-size: 11px;
    color: var(--nd-text-secondary);
}

.hm-slot[data-status="unassigned"] .hm-slot-status,
.hm-slot[data-status="different"] .hm-slot-status {
    color: var(--nd-warning);
}

.hm-foot {
    display: flex;
    justify-content: space-between;
    gap: var(--nd-space-md);
    padding-top: var(--nd-space-sm);
    border-top: 1px solid var(--nd-border);
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--nd-text-secondary);
}

.hm-sync-ok {
    color: var(--nd-signal);
}

.hm-sync-warn {
    color: var(--nd-warning);
}
```

In `index.html`, add `<link data-trunk rel="css" href="style/home.css" />` on the line after the `style/slice.css` link.

- [ ] **Step 5: Remove what only the old Home used**

(a) Run: `grep -rn "HeroArtwork" src`
Expected: matches only in `src/components/branding.rs`. Delete the `HeroArtwork` component from that file, along with any imports that become unused.

(b) Run: `grep -rln 'hero-panel\|hero-copy\|hero-eyebrow\|hero-description\|hero-actions\|hero-artwork\|how-it-works\|"steps"\|"step"\|step-number\|step-content\|card-grid' src`
Expected: no output, because the rewritten `home.rs` uses none of them.

For each class with no remaining user, delete its rules from `style/main.css`. That is every rule whose selector starts with `.hero-`, plus `.how-it-works`, `.steps`, `.step`, `.step-number`, `.step-content` and `.card-grid`. Change the shared `.nd-stat, .hero-panel { … }` rule to `.nd-stat { … }`.

If the grep shows a class still in use somewhere else (for example, `.step` in the setup wizard), keep that class's rules.

- [ ] **Step 6: Build and run the unit tests**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo fmt --check`
Expected: all pass; builds; no unused-import warnings in `home.rs` or `branding.rs`.

- [ ] **Step 7: Add the Home states to the browser tests**

In `tests/webkit/app-flows.mjs`:

(a) Add `PRINTER_SLOTS,` to the import list from `./fixtures.mjs`, between `PRINTER_SERIAL` and `PRINTER_UNCONFIGURED`.

(b) Directly before the comment `// Put the canned answers back so later steps start from the defaults.`, add:

```js
  // -- home cockpit -----------------------------------------------------------
  // textContent, not innerText: the hero label and the footer are uppercased by CSS.
  const homeMode = (mode) => page.waitForSelector(`.home-page[data-mode="${mode}"]`, { timeout: 5000 });
  const homeText = (sel) => page.locator(sel).first().evaluate((el) => el.textContent.replace(/\s+/g, " ").trim());
  const RUNNING_CLEAN = { ...PRINTER_VIEW, errors: [] };
  const FINISHED = { ...RUNNING_CLEAN, state: { ...PRINTER_VIEW.state, gcode_state: "FINISH", mc_percent: 100 } };

  await step(run, page, "home leads with a printer error", async () => {
    await setFixture("printer_view", PRINTER_VIEW);
    await page.click('nav.sidebar a[href="/"]');
    await emitEvent("printer://state", PRINTER_VIEW);
    await homeMode("attention");
    const hero = await homeText(".home-page .nd-hero");
    for (const want of ["ERROR", "Needs attention · 0300_0100_0001_0007", "The heatbed temperature is abnormal"]) {
      if (!hero.includes(want)) throw new Error(`hero lacks "${want}": ${hero}`);
    }
  });

  await step(run, page, "What does this mean? puts the error in the agent composer", async () => {
    const starts = (await callsOf("agent_start")).length + (await callsOf("agent_send")).length;
    await page.click(".hm-ask");
    await page.waitForSelector(".agent-drawer.open", { timeout: 5000 });
    await page.waitForFunction(
      () => document.querySelector(".ag-input")?.value.includes("0300_0100_0001_0007"),
      null,
      { timeout: 5000 }
    );
    // Nothing is sent until the user presses send.
    if ((await callsOf("agent_start")).length + (await callsOf("agent_send")).length !== starts) {
      throw new Error("the question was sent without the user");
    }
    await page.fill(".ag-input", "");
    await page.click(".agent-toggle");
    await page.waitForSelector(".agent-drawer.open", { state: "detached", timeout: 5000 });
  });

  await step(run, page, "home shows the print in progress", async () => {
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
    const number = await homeText(".home-page .nd-hero-number");
    if (number !== "6") throw new Error(`hero number "${number}"`);
    const hero = await homeText(".home-page .nd-hero");
    if (!hero.includes("9 h 09 m left · Layer 1 / 200 · Nozzle 245 °C · Bed 70 °C")) throw new Error(hero);
    const on = await page.locator(".home-page .nd-seg-on").count();
    if (on !== 1) throw new Error(`${on} segments filled for 6%`);
  });

  await step(run, page, "home AMS slots open the preset picker", async () => {
    const slots = await page.locator(".hm-slot").count();
    if (slots !== PRINTER_SLOTS.length) throw new Error(`${slots} slots, expected ${PRINTER_SLOTS.length}`);
    await page.click('.hm-slot[data-slot-label="A3"] .hm-slot-main');
    await page.waitForSelector(".pr-picker", { timeout: 5000 });
    await page.click(".pr-picker-cancel");
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
  });

  await step(run, page, "home is ready after a print finishes", async () => {
    await emitEvent("printer://state", FINISHED);
    await homeMode("idle");
    const number = await homeText(".home-page .nd-hero-number");
    if (number !== "READY") throw new Error(`hero number "${number}"`);
    const hero = await homeText(".home-page .nd-hero");
    if (!hero.includes("Last print finished just now · T-pose - slim H2D dual AMS riser")) throw new Error(hero);
  });

  await step(run, page, "home never shows a stale percentage after a disconnect", async () => {
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
    await emitEvent("printer://connection", { state: "unreachable" });
    await homeMode("no-printer");
    const hero = await homeText(".home-page .nd-hero");
    if (!hero.includes("[DISCONNECTED]")) throw new Error(hero);
    const number = await homeText(".home-page .nd-hero-number");
    if (number !== String(FIXTURES.list_profiles.length)) throw new Error(`hero number "${number}"`);
    if ((await page.locator(".hm-ams").count()) !== 0) throw new Error("AMS shown while disconnected");
    await emitEvent("printer://connection", { state: "connected" });
    await homeMode("printing");
  });

  await step(run, page, "home footer shows recent work and sync status", async () => {
    // Earlier steps (Health repair) may have changed these answers; start from the defaults
    // and remount Home so it reads them.
    await setFixture("list_unsynced_presets", FIXTURES.list_unsynced_presets);
    await setFixture("latest_generated_preset", FIXTURES.latest_generated_preset);
    await page.click('nav.sidebar a[href="/printer"]');
    await page.click('nav.sidebar a[href="/"]');
    await page.waitForSelector(".hm-recent span", { timeout: 5000 });
    await page.waitForSelector(".hm-sync a.hm-sync-warn", { timeout: 5000 });
    const recent = await homeText(".hm-recent");
    if (!recent.startsWith("Recent · Polymaker PETG Red · ")) throw new Error(`recent reads "${recent}"`);
    const sync = page.locator(".hm-sync a.hm-sync-warn");
    const text = (await sync.textContent()).trim();
    if (text !== "2 presets unsynced") throw new Error(`sync reads "${text}"`);
    if ((await sync.getAttribute("href")) !== "/settings/health") throw new Error("sync link goes elsewhere");
  });

  await step(run, page, "a failed scan shows inline and the rest of Home renders", async () => {
    await withFixtures({ list_unsynced_presets: { __reject: "scan failed" } }, async () => {
      await page.click('nav.sidebar a[href="/printer"]');
      await page.click('nav.sidebar a[href="/"]');
      await page.waitForFunction(
        () => document.querySelector(".hm-sync .nd-state")?.textContent === "[ERROR: scan failed]",
        null,
        { timeout: 5000 }
      );
      if ((await page.locator(".home-page .nd-hero").count()) !== 1) throw new Error("hero missing");
    });
  });

  await step(run, page, "home with no printer leads with profiles and offers to connect", async () => {
    await emitEvent("printer://state", PRINTER_UNCONFIGURED);
    await homeMode("no-printer");
    const label = await homeText(".home-page .nd-hero-label");
    if (label !== "BambuMate") throw new Error(`label reads "${label}"`);
    if ((await page.locator(".hm-connect").getAttribute("href")) !== "/settings#printer") throw new Error("connect link");
    await emitEvent("printer://state", PRINTER_VIEW);
    await homeMode("attention");
  });
```

`callsOf`, `setFixture`, `withFixtures` and `emitEvent` are already defined earlier in `driveApp`.

- [ ] **Step 8: Run the full suite**

Run:

```bash
cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check && cargo test --bin bambumate && cargo test --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets && cargo check --target wasm32-unknown-unknown
```

Expected: everything passes, with no warnings from clippy.

Run: `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: every check passes in both engines, including the 9 new Home steps.

- [ ] **Step 9: Look at Home in both themes**

Run: `trunk build && (cd tests/webkit && node screens.mjs ../..)`
Then open these files in `tests/webkit/screens/`:
- `dark-1280-home.png`
- `bambu-1280-home.png`
- `dark-900-home.png`
- `dark-1280-settings-about.png`

Check that:
- Home leads with one large Doto element;
- the AMS row wraps rather than overflowing at 900px;
- the footer sits on one line;
- the Settings tab row shows the active pill.

Fix any layout problem in `style/home.css` or the pattern CSS, then rerun Step 8.

- [ ] **Step 10: Commit**

```bash
git add src/pages/home.rs style/home.css index.html src/agent/drawer.rs src/pages/printer.rs src/components/branding.rs style/main.css tests/webkit/app-flows.mjs
git commit -m "feat(home): cockpit Home with live printer hero, AMS slots and sync footer"
```

---

## Self-review notes

How the plan covers the spec:

| Spec section | Where it's covered |
|---|---|
| §1 rail | Task 2 |
| §1 redirects, replace-in-history | Task 2 (`NoRoute`) |
| §1 agent routes | Task 3 |
| §1 feature flag (locked, unchanged) | Task 2 keeps `locked` |
| §2 patterns | Task 1 |
| §2 table | Task 1 CSS (`.nd-table`) |
| §2 pill buttons | Already pills; the Profiles `.btn-icon` is wave 2 |
| §3 Home states and precedence | Tasks 7 and 8 |
| §3 slot outline | Tasks 7 and 8 (`active_slot`, `data-in-use`) |
| §3 "What does this mean?" prefill | Task 8 |
| §3 data rules (mount + `AgentRefresh`, no timer) | Task 8 |
| §3 last print | Task 6 |
| §3 inline errors | Task 8 |
| §4 STL indicator | Task 4 |
| Error handling: disconnect | Tasks 7 and 8 |
| Error handling: unknown route | Task 2 (`NoRoute`) |
| Testing: unit | Tasks 1, 2, 4, 6, 7 |
| Testing: backend | Tasks 3 and 5 |
| Testing: browser | Tasks 2, 4, 8 |
| Testing: contrast | Task 1 |

The severity colours and Analyze changes are wave 3.

Types used across tasks:
- `Tab` is `(&'static str, &'static str)` everywhere.
- `GeneratedPreset { profile_name, created_at: i64 }` matches on both sides of the IPC boundary.
- `LastPrint.duration_ms` is `Option<f64>`, which `last_print_detail` reads.
- `HomeMode::NoPrinter { disconnected }` is matched by name in `home.rs`.
