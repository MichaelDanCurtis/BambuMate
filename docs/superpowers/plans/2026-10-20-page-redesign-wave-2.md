# Page Redesign Wave 2: Filament and Profiles Hubs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the four pages inside the Filament and Profiles hubs as cockpit pages. Each leads with one hero and uses wave 1's shared patterns. The pages are:
- Filament · Search (`/filament`)
- Filament · Batch (`/filament/batch`)
- Profiles · Library (`/profiles`)
- Profiles · Compare (`/profiles/compare`)

**Architecture:**
- Two patterns join `src/components/patterns.rs`:
  - `HeroStats`: a hero with several Doto values side by side.
  - `StepStrip`: numbered steps, with earlier ones clickable.
- Each page's decisions live in a small pure module with unit tests. Its view composes the patterns.
- Handlers and data calls stay as they are; only the markup and the derived display change.
- The page `<h2>` and description lines go, because the hub's `PageHeader` already gives each page its h1 and tabs.
- Spinners and coloured message boxes become `StateText` lines.

**Tech Stack:** Rust, Leptos 0.8 CSR (WASM, built by Trunk), Playwright WebKit/Chromium tests in `tests/webkit/`.

**Spec:** `docs/superpowers/specs/2026-10-17-page-redesign-design.md`. Wave 2 covers the four §4 rows for Filament and Profiles, which were amended on 2026-10-20: Search uses a step strip, Batch has no live progress, Library shows only a "not syncing" dot, and Compare filters on the client.

## Global Constraints

- **Read-only printer.** Nothing in this piece sends a command to the printer.
- **Tokens only.** New CSS uses the `--nd-*` tokens from `style/tokens.css`. No new colours are introduced.
- **Both themes are first-class.** Every new text/background pair passes `contrast.mjs` in dark and light: 4.5:1 for text, 3:1 for disabled text.
- **Font budget per screen.** At most two families (Space Grotesk and Space Mono), plus Doto for the single hero. At most three sizes and two weights.
- **Existing behaviour is kept.** Every feature on these four pages is still reachable after the redesign. Nothing is removed.
- **Honest copy.** BambuMate only sees presets stuck out of sync, so no UI may claim a preset *is* synced.
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
- In `tests/webkit/app-flows.mjs`, `goTo(route, expect?)` and `openTab(hub, tab)` are defined near the top of `driveApp`. Use `textContent`, not `innerText`, for anything uppercased by CSS (`.nd-label`, the hero label).

## What wave 1 provides (on `main`)

All of these live in `src/components/patterns.rs`:
- `PageHeader`, rendered by the hub.
- `Hero(label: Signal<String>, value: Signal<String>, unit/detail: MaybeProp<String>, tone: MaybeProp<Tone>)`, rendering `.nd-hero > .nd-hero-label, .nd-hero-value > .nd-hero-number + .nd-hero-unit, .nd-hero-detail`.
- `SegmentedBar(percent: Signal<u32>, total: u32, label: &'static str)`.
- `StatRow(label: &'static str, value: Signal<String>)`, rendering `.nd-stat-row > .nd-stat-row-label + .nd-stat-row-value`.
- `StateText(kind: StateKind, message: MaybeProp<String>)`, rendering `p.nd-state[data-kind]` with `[LOADING…]` / `[EMPTY: …]` / `[ERROR: …]`.
- The `.nd-table` and `.nd-num` CSS.

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/components/patterns.rs` | modified | Adds `HeroStats`, `StepStrip`, `StepState` and `step_state` |
| `style/main.css` | modified | Pattern CSS for hero stats and steps |
| `src/pages/profile_numbers.rs` | new | `key_numbers(raw_json)`: a profile's headline numbers |
| `src/pages/profile_management.rs` + `.css` | modified | Library: hero, stat rows, not-syncing dots, pill actions, `.nd-table` fields |
| `src/pages/diff_view.rs` | new | `visible_categories`, `is_changed` |
| `src/pages/profile_diff.rs` + `.css` | modified | Compare: hero, a "Changed only" client-side filter, `.nd-table` |
| `src/pages/batch_view.rs` | new | `batch_line`, `batch_percent` |
| `src/pages/batch_generate.rs` + `.css` | modified | Batch: hero after a run, `.nd-table` results |
| `src/pages/filament_steps.rs` | new | `SearchStep`, `search_step`, `step_index`, `spec_label`, `spec_stats`, `SEARCH_STEPS` |
| `src/pages/filament_search.rs` + `.css` | modified | Search: search hero, then specs hero + step strip, `StateText` |
| `src/pages/mod.rs` | modified | Registers the four new modules |
| `tests/webkit/app-flows.mjs` | modified | Updated and new steps for all four pages |

---

### Task 1: `HeroStats` and `StepStrip` patterns

**Files:**
- Modify: `src/components/patterns.rs`, `style/main.css`

**Interfaces:**
- Produces:
  - `HeroStats(label: Signal<String>, stats: Signal<Vec<(String, String)>>, detail: MaybeProp<String>)`
  - `StepStrip(steps: &'static [&'static str], current: Signal<usize>, can_go_back: Signal<bool>, on_back: Callback<usize>)`
  - `enum StepState { Done, Current, Upcoming }` with `key()`
  - `fn step_state(index: usize, current: usize) -> StepState`
  - CSS: `.nd-hero-stats`, `.nd-hero-stat`, `.nd-hero-stat-value`, `.nd-steps`, `.nd-step[data-state]`, `.nd-step-btn`, `.nd-step-num`, `.nd-step-name`.

- [ ] **Step 1: Write the failing tests**

In the tests module of `src/components/patterns.rs`, add:

```rust
    #[test]
    fn steps_before_the_current_one_are_done() {
        assert_eq!(step_state(0, 1), StepState::Done);
        assert_eq!(step_state(1, 1), StepState::Current);
        assert_eq!(step_state(2, 1), StepState::Upcoming);
        assert_eq!(step_state(0, 0), StepState::Current);
    }

    #[test]
    fn step_states_have_distinct_keys() {
        let keys = [StepState::Done, StepState::Current, StepState::Upcoming].map(StepState::key);
        assert_eq!(keys, ["done", "current", "upcoming"]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin bambumate patterns`
Expected: compile errors: `StepState` and `step_state` are not found.

- [ ] **Step 3: Implement**

Add above the tests module in `src/components/patterns.rs`:

```rust
/// Where a step sits relative to the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Upcoming,
}

impl StepState {
    /// `data-state` on the step.
    pub fn key(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Current => "current",
            Self::Upcoming => "upcoming",
        }
    }
}

pub fn step_state(index: usize, current: usize) -> StepState {
    match index.cmp(&current) {
        std::cmp::Ordering::Less => StepState::Done,
        std::cmp::Ordering::Equal => StepState::Current,
        std::cmp::Ordering::Greater => StepState::Upcoming,
    }
}

/// A hero with several Doto values side by side, each under its own mono
/// label. Still the page's one hero.
#[component]
pub fn HeroStats(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] stats: Signal<Vec<(String, String)>>,
    #[prop(into, optional)] detail: MaybeProp<String>,
) -> impl IntoView {
    view! {
        <section class="nd-hero">
            <p class="nd-label nd-hero-label">{move || label.get()}</p>
            <div class="nd-hero-stats">
                {move || {
                    stats
                        .get()
                        .into_iter()
                        .map(|(name, value)| view! {
                            <div class="nd-hero-stat">
                                <span class="nd-label">{name}</span>
                                <span class="nd-display nd-hero-stat-value">{value}</span>
                            </div>
                        })
                        .collect_view()
                }}
            </div>
            {move || {
                detail
                    .get()
                    .filter(|d| !d.is_empty())
                    .map(|d| view! { <p class="nd-hero-detail">{d}</p> })
            }}
        </section>
    }
}

/// Numbered steps of a flow. Steps before the current one are buttons back
/// to that step while `can_go_back` holds; the rest are plain text.
#[component]
pub fn StepStrip(
    steps: &'static [&'static str],
    #[prop(into)] current: Signal<usize>,
    #[prop(into)] can_go_back: Signal<bool>,
    #[prop(into)] on_back: Callback<usize>,
) -> impl IntoView {
    let items = steps
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let state = move || step_state(i, current.get());
            let number = format!("{:02}", i + 1);
            view! {
                <li class="nd-step" data-state=move || state().key()>
                    {move || {
                        let inner = view! {
                            <span class="nd-step-num">{number.clone()}</span>
                            <span class="nd-step-name">{name}</span>
                        };
                        if state() == StepState::Done && can_go_back.get() {
                            view! {
                                <button type="button" class="nd-step-btn"
                                    title=format!("Back to {name}")
                                    on:click=move |_| on_back.run(i)>
                                    {inner}
                                </button>
                            }
                            .into_any()
                        } else {
                            view! {
                                <span class="nd-step-btn"
                                    aria-current=move || (state() == StepState::Current).then_some("step")>
                                    {inner}
                                </span>
                            }
                            .into_any()
                        }
                    }}
                </li>
            }
        })
        .collect_view();
    view! { <ol class="nd-steps" aria-label="Steps">{items}</ol> }
}
```

- [ ] **Step 4: Add the CSS**

Append to `style/main.css`, after the "Page patterns" section:

```css
.nd-hero-stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-xl);
}

.nd-hero-stat {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-xs);
}

.nd-hero-stat-value {
    font-size: clamp(36px, 5vw, 56px);
}

.nd-steps {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-sm);
    list-style: none;
}

.nd-step-btn {
    display: inline-flex;
    gap: var(--nd-space-sm);
    align-items: baseline;
    padding: 6px 14px;
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-pill);
    background: transparent;
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--nd-text-disabled);
}

.nd-step[data-state="done"] .nd-step-btn {
    color: var(--nd-text-secondary);
    border-color: var(--nd-border-visible);
}

button.nd-step-btn {
    cursor: pointer;
}

button.nd-step-btn:hover {
    color: var(--nd-text-primary);
}

button.nd-step-btn:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.nd-step[data-state="current"] .nd-step-btn {
    color: var(--nd-black);
    background-color: var(--nd-text-display);
    border-color: var(--nd-text-display);
}
```

- [ ] **Step 5: Run the checks**

Run: `cargo test --bin bambumate patterns && cargo check --target wasm32-unknown-unknown && cargo fmt --check && (cd tests/webkit && node css-compat.mjs ../.. && node contrast.mjs ../..)`
Expected:
- the 2 new tests pass along with the existing pattern tests;
- the WASM build succeeds (unused-component warnings are fine until Task 5);
- css-compat and contrast pass. The current step is `--nd-black` on `--nd-text-display`, which is already covered by the FILLED pairs.

- [ ] **Step 6: Commit**

```bash
git add src/components/patterns.rs style/main.css
git commit -m "feat(ui): HeroStats and StepStrip patterns"
```

---

### Task 2: Profiles · Library

**Files:**
- Create: `src/pages/profile_numbers.rs`
- Modify: `src/pages/mod.rs`, `src/pages/profile_management.rs`, `src/pages/profile_management.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes: `Hero`, `StatRow`, `StateText` and `StateKind` from patterns; `commands::list_unsynced_presets()`, which returns `UnsyncedPresetList { presets: Vec<UnsyncedPreset { path, .. }>, .. }`.
- Produces:
  - `profile_numbers::KeyNumbers { nozzle, bed, flow, pressure_advance, max_volumetric }`, all `String`;
  - `profile_numbers::key_numbers(raw_json: &str) -> KeyNumbers`.

- [ ] **Step 1: Write the failing tests**

Create `src/pages/profile_numbers.rs` with only the tests module, then add `pub mod profile_numbers;` to `src/pages/mod.rs`, keeping the list in alphabetical (rustfmt) order:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headline_numbers_come_from_the_first_array_value() {
        let n = key_numbers(
            r#"{"nozzle_temperature":["220","225"],"hot_plate_temp":["65"],
                "filament_flow_ratio":["0.98"],"pressure_advance":["0.04"],
                "filament_max_volumetric_speed":["12"]}"#,
        );
        assert_eq!(n.nozzle, "220°");
        assert_eq!(n.bed, "65 °C");
        assert_eq!(n.flow, "0.98");
        assert_eq!(n.pressure_advance, "0.04");
        assert_eq!(n.max_volumetric, "12 mm³/s");
    }

    #[test]
    fn missing_or_blank_values_show_a_dash() {
        let n = key_numbers(r#"{"nozzle_temperature":[""],"bed_temperature":"60"}"#);
        assert_eq!(n.nozzle, "—");
        // Falls back to bed_temperature when there is no hot_plate_temp.
        assert_eq!(n.bed, "60 °C");
        assert_eq!(n.flow, "—");
        assert_eq!(key_numbers("not json").nozzle, "—");
    }

    #[test]
    fn numbers_stored_as_numbers_are_read_too() {
        let n = key_numbers(r#"{"filament_flow_ratio":0.95,"nozzle_temperature":[215]}"#);
        assert_eq!(n.flow, "0.95");
        assert_eq!(n.nozzle, "215°");
    }
}
```

Run: `cargo test --bin bambumate profile_numbers`
Expected: compile errors: `key_numbers` is not found.

- [ ] **Step 2: Implement the helper**

Put this above the tests in `src/pages/profile_numbers.rs`:

```rust
//! A profile's headline numbers for the Library hero and stat rows (spec
//! 2026-10-17 §4). Bambu Studio stores most values as one-item string
//! arrays; a value the profile inherits from its parent is absent and shows
//! as a dash.

/// Shown for a value the profile doesn't set itself.
const MISSING: &str = "—";

#[derive(Debug, Clone, PartialEq)]
pub struct KeyNumbers {
    pub nozzle: String,
    pub bed: String,
    pub flow: String,
    pub pressure_advance: String,
    pub max_volumetric: String,
}

/// The first value of `key`, from a string, a number, or the first item of
/// an array of either; `None` when absent or blank.
fn first(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    let value = match obj.get(key)? {
        serde_json::Value::Array(items) => items.first()?.clone(),
        other => other.clone(),
    };
    let text = match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn key_numbers(raw_json: &str) -> KeyNumbers {
    let raw: serde_json::Value = serde_json::from_str(raw_json).unwrap_or_default();
    let empty = serde_json::Map::new();
    let obj = raw.as_object().unwrap_or(&empty);
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| MISSING.to_string());
    KeyNumbers {
        nozzle: or_dash(first(obj, "nozzle_temperature").map(|t| format!("{t}°"))),
        bed: or_dash(
            first(obj, "hot_plate_temp")
                .or_else(|| first(obj, "bed_temperature"))
                .map(|t| format!("{t} °C")),
        ),
        flow: or_dash(first(obj, "filament_flow_ratio")),
        pressure_advance: or_dash(first(obj, "pressure_advance")),
        max_volumetric: or_dash(
            first(obj, "filament_max_volumetric_speed").map(|v| format!("{v} mm³/s")),
        ),
    }
}
```

Run: `cargo test --bin bambumate profile_numbers`
Expected: 3 passed.

- [ ] **Step 3: Load which presets aren't syncing**

In `ProfileManagementPage`:

(a) Next to the other list state, add:

```rust
    // Paths `list_unsynced_presets` reports as not syncing; they get a
    // warning dot. Nothing marks a preset as synced, since BambuMate can't
    // confirm one reached the cloud.
    let (not_syncing, set_not_syncing) =
        signal::<std::collections::HashSet<String>>(Default::default());
```

(b) Inside `load_profiles`'s `spawn_local`, after the list is set, add:

```rust
            if let Ok(list) = commands::list_unsynced_presets().await {
                set_not_syncing.set(list.presets.into_iter().map(|p| p.path).collect());
            }
```

- [ ] **Step 4: Rebuild the view**

(a) Add these imports:

```rust
use crate::components::patterns::{Hero, StateKind, StateText, StatRow};
use crate::pages::profile_numbers::key_numbers;
```

(b) In the `view!`:

1. Delete the `<h2>"Profile Management"</h2>` and the `<p class="page-description">…</p>` that follows it. The hub header supplies the h1.

2. Replace the two action feedback blocks with:

```rust
            {move || action_error.get().map(|e| view! {
                <StateText kind=StateKind::Error message=e />
            })}
            {move || action_success.get().map(|s| view! {
                <p class="nd-state pm-done" role="status">{s}</p>
            })}
```

3. Replace the loading block (`<Show when=move || is_loading.get()>…</Show>`) with:

```rust
            <Show when=move || is_loading.get()>
                <StateText kind=StateKind::Loading />
            </Show>
```

   Replace the list-error block with `{move || list_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}`.

4. In the `<For>` children for the list, replace the item markup with the following. It keeps the `.profile-list-item` hook and adds the dot:

```rust
                                        let path_dot = path.clone();
                                        let syncing_problem = move || not_syncing.with(|s| s.contains(&path_dot));
                                        view! {
                                            <button
                                                type="button"
                                                class="profile-list-item"
                                                class:selected=is_selected
                                                on:click=move |_| select_profile(path_click.clone())
                                            >
                                                <span class="profile-item-name">{p.name.clone()}</span>
                                                <span class="profile-item-type nd-label">
                                                    {p.filament_type.clone().unwrap_or_default()}
                                                </span>
                                                <Show when=syncing_problem>
                                                    <span class="pm-sync-dot" role="img" aria-label="Not syncing"
                                                        title="Not syncing to your Bambu account — see Settings → Health"></span>
                                                </Show>
                                            </button>
                                        }
```

   Replace the "No profiles found" block with `<StateText kind=StateKind::Empty message="No profiles match" />`, keeping its `<Show when=…>`.

5. In the detail panel, replace the "Select a profile" and "Loading profile…" bodies with `StateText` (`Empty` with message `"Select a profile"`, and `Loading`). The detail error becomes `StateText` `Error`.

6. Replace the whole `Some(detail) => { view! { <div> … </div> }.into_any() }` arm's header (from `<div class="profile-detail-header">` to the closing `</div>` of `.profile-detail-actions`) with:

```rust
                                                    {
                                                        let numbers = key_numbers(&detail.raw_json);
                                                        let title = detail.name.clone().unwrap_or_else(|| "(unnamed)".to_string());
                                                        let subtitle = format!(
                                                            "{} · {} fields",
                                                            detail.filament_type.clone().unwrap_or_default(),
                                                            detail.field_count
                                                        );
                                                        view! {
                                                            <div class="pm-detail-head">
                                                                <Hero
                                                                    label=title
                                                                    value=numbers.nozzle.clone()
                                                                    detail=subtitle
                                                                />
                                                                <div class="pm-stats">
                                                                    <StatRow label="Bed" value=numbers.bed.clone() />
                                                                    <StatRow label="Flow ratio" value=numbers.flow.clone() />
                                                                    <StatRow label="Pressure advance" value=numbers.pressure_advance.clone() />
                                                                    <StatRow label="Max volumetric speed" value=numbers.max_volumetric.clone() />
                                                                </div>
                                                            </div>
                                                        }
                                                    }
                                                    <div class="pm-actions">
                                                        <button
                                                            class="btn btn-secondary"
                                                            on:click=move |_| {
                                                                let path = selected_path.get();
                                                                spawn_local(async move {
                                                                    match commands::launch_bambu_studio(None, path).await {
                                                                        Ok(_) => set_action_success.set(Some("Bambu Studio launched".to_string())),
                                                                        Err(e) => set_action_error.set(Some(e)),
                                                                    }
                                                                });
                                                            }
                                                        >
                                                            "Open in Bambu Studio"
                                                        </button>
                                                        <button
                                                            class="btn btn-secondary btn-edit-specs"
                                                            disabled=move || specs_loading.get()
                                                            on:click=move |_| open_specs_editor()
                                                        >
                                                            {move || if specs_loading.get() { "Loading…" } else { "Edit specs" }}
                                                        </button>
                                                        <button
                                                            class="btn btn-secondary"
                                                            on:click=move |_| {
                                                                let name = selected_detail.get()
                                                                    .and_then(|d| d.name.clone())
                                                                    .unwrap_or_default();
                                                                set_duplicate_name.set(format!("{} (copy)", name));
                                                                set_show_duplicate_input.set(true);
                                                                set_action_error.set(None);
                                                                set_action_success.set(None);
                                                            }
                                                        >
                                                            "Duplicate"
                                                        </button>
                                                        <button
                                                            class="btn btn-danger"
                                                            on:click=move |_| {
                                                                set_show_delete_confirm.set(true);
                                                                set_action_error.set(None);
                                                                set_action_success.set(None);
                                                            }
                                                        >
                                                            "Delete"
                                                        </button>
                                                    </div>
```

   `Hero`'s `label`, `value` and `detail` take `String`s through `into`. A `String` converts into `Signal<String>` and `MaybeProp<String>`.

7. Change the fields table's opening tag to `<table class="nd-table profile-fields">`. Change its key cell to `<td class="field-key nd-label">`, and leave the rest of the row (click-to-edit) unchanged.

(c) In `src/pages/profile_management.css`:
- Remove the rules for classes that no longer exist. Check each with `grep -n "<class>" src/pages/profile_management.rs` first: `.profile-detail-header`, `.profile-detail-title`, `.profile-detail-subtitle`, `.profile-detail-actions`, `.btn-icon` (only if no other file uses it: `grep -rn "btn-icon" src`), `.profile-loading`, `.profile-error`, `.profile-success`, `.profile-detail-empty`, `.profile-item-info` and `.profile-item-badge`.
- Add, using only `--nd-*` tokens:

```css
.profile-list-item {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px var(--nd-space-sm);
    width: 100%;
    padding: var(--nd-space-sm) var(--nd-space-md);
    background: transparent;
    border: 0;
    border-bottom: 1px solid var(--nd-border);
    color: var(--nd-text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
}

.profile-list-item.selected {
    background-color: var(--nd-surface-raised);
}

.profile-list-item:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: -2px;
}

.profile-item-type {
    grid-column: 1;
}

.pm-sync-dot {
    grid-column: 2;
    grid-row: 1 / span 2;
    align-self: center;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: var(--nd-warning);
}

.pm-detail-head {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-md);
    margin-bottom: var(--nd-space-lg);
}

.pm-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-sm);
    margin-bottom: var(--nd-space-lg);
}
```

- [ ] **Step 5: Update the browser steps**

In `tests/webkit/app-flows.mjs`, replace the body of the step "opening a profile shows its fields" with:

```js
    await page.locator(".profile-list-item").first().click();
    await page.waitForSelector(".profile-management-page .nd-hero", { timeout: 15000 });
    await page.waitForSelector("table.profile-fields", { timeout: 15000 });
    const title = (await page.locator(".pm-detail-head .nd-hero-label").textContent()).trim();
    const nozzle = (await page.locator(".pm-detail-head .nd-hero-number").textContent()).trim();
    if (nozzle !== "220°") throw new Error(`hero reads "${nozzle}"`);
    const subtitle = (await page.locator(".pm-detail-head .nd-hero-detail").textContent()).trim();
    if (!subtitle.includes("42 fields")) throw new Error(`detail reads "${subtitle}"`);
    const stats = await page.locator(".pm-stats .nd-stat-row").count();
    if (stats !== 4) throw new Error(`${stats} stat rows`);
    const keys = await page.locator("td.field-key").count();
    if (keys === 0) throw new Error("fields table rendered with no rows");
    const pills = await page.locator(".pm-actions .btn").count();
    if (pills !== 4) throw new Error(`${pills} action buttons`);
    return `${title}, ${keys} field rows`;
```

Directly after it, add:

```js
  await step(run, page, "a preset that isn't syncing gets a warning dot", async () => {
    const path = FIXTURES.list_profiles[0].path;
    await withFixtures(
      { list_unsynced_presets: { presets: [{ path, profile_name: "x", file_name: "x.json", source: "confirmed" }], bambu_studio_running: false } },
      async () => {
        await page.click('nav.sidebar a[href="/"]');
        await page.click('nav.sidebar a[href="/profiles"]');
        await page.waitForSelector(".profile-list-item .pm-sync-dot", { timeout: 15000 });
        const dots = await page.locator(".pm-sync-dot").count();
        if (dots !== 1) throw new Error(`${dots} dots, expected 1`);
        const label = await page.locator(".pm-sync-dot").getAttribute("aria-label");
        if (label !== "Not syncing") throw new Error(`dot label "${label}"`);
      }
    );
  });
```

`withFixtures` and `setFixture` are defined further down `driveApp` (after the slice section). If they aren't in scope at this point, move these two `const` declarations up to just after `openTab` and remove them from their old place. They depend only on `page`. Note any such move in your report.

- [ ] **Step 6: Run everything**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo fmt --check && trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: all pass in both engines.

- [ ] **Step 7: Commit**

```bash
git add src/pages/profile_numbers.rs src/pages/mod.rs src/pages/profile_management.rs src/pages/profile_management.css tests/webkit/app-flows.mjs
git commit -m "feat(profiles): Library leads with the profile's numbers; not-syncing dots; pill actions"
```

---

### Task 3: Profiles · Compare

**Files:**
- Create: `src/pages/diff_view.rs`
- Modify: `src/pages/mod.rs`, `src/pages/profile_diff.rs`, `src/pages/profile_diff.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes: `commands::{CompareResult, DiffCategory { category, diffs }, CompareProfileDiff { key, label, base_value, new_value }}`. All three derive `Clone`.
- Produces:
  - `diff_view::is_changed(&CompareProfileDiff) -> bool`
  - `diff_view::visible_categories(&[DiffCategory], changed_only: bool) -> Vec<DiffCategory>`

- [ ] **Step 1: Write the failing tests**

Create `src/pages/diff_view.rs` with only the tests module, and add `pub mod diff_view;` to `src/pages/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn diff(label: &str, a: &str, b: &str) -> CompareProfileDiff {
        CompareProfileDiff {
            key: label.to_lowercase(),
            label: label.into(),
            base_value: a.into(),
            new_value: b.into(),
        }
    }

    fn cats() -> Vec<DiffCategory> {
        vec![
            DiffCategory {
                category: "Temperature".into(),
                diffs: vec![diff("Nozzle", "220", "215"), diff("Bed", "60", "60")],
            },
            DiffCategory {
                category: "Cooling".into(),
                diffs: vec![diff("Fan", "100", "100")],
            },
        ]
    }

    #[test]
    fn changed_only_drops_identical_rows_and_empty_categories() {
        let shown = visible_categories(&cats(), true);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].category, "Temperature");
        assert_eq!(shown[0].diffs.len(), 1);
        assert_eq!(shown[0].diffs[0].label, "Nozzle");
    }

    #[test]
    fn showing_everything_keeps_every_row_in_order() {
        let shown = visible_categories(&cats(), false);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].diffs.len(), 2);
        assert!(is_changed(&shown[0].diffs[0]));
        assert!(!is_changed(&shown[0].diffs[1]));
    }
}
```

Run: `cargo test --bin bambumate diff_view`
Expected: compile errors: `visible_categories` and `is_changed` are not found.

- [ ] **Step 2: Implement**

Above the tests in `src/pages/diff_view.rs`:

```rust
//! What Compare shows (spec 2026-10-17 §4). Compare always fetches every
//! field and filters here, so "Changed only" works without comparing again.

pub use crate::commands::{CompareProfileDiff, DiffCategory};

pub fn is_changed(d: &CompareProfileDiff) -> bool {
    d.base_value != d.new_value
}

/// The categories to render: with `changed_only`, identical rows go, and so
/// does any category left empty.
pub fn visible_categories(categories: &[DiffCategory], changed_only: bool) -> Vec<DiffCategory> {
    categories
        .iter()
        .map(|c| DiffCategory {
            category: c.category.clone(),
            diffs: c
                .diffs
                .iter()
                .filter(|d| !changed_only || is_changed(d))
                .cloned()
                .collect(),
        })
        .filter(|c| !c.diffs.is_empty())
        .collect()
}
```

Run: `cargo test --bin bambumate diff_view`
Expected: 2 passed.

- [ ] **Step 3: Rebuild the page**

In `src/pages/profile_diff.rs`:

(a) Add these imports:

```rust
use crate::components::patterns::{Hero, StateKind, StateText};
use crate::pages::diff_view::{is_changed, visible_categories};
```

(b) Replace the `show_identical` signal with:

```rust
    // On by default; filters on the client, so toggling never re-compares.
    let (changed_only, set_changed_only) = signal(true);
```

(c) In `do_compare`:
- delete `let identical = show_identical.get();`;
- call `commands::compare_profiles(&a, &b, true)`, which always fetches every field.

(d) Replace the whole `view!` with:

```rust
    view! {
        <div class="page profile-diff-page">
            <style>{include_str!("profile_diff.css")}</style>

            <div class="diff-config">
                <div class="diff-pickers">
                    <div class="form-group">
                        <label class="nd-label">"Profile A"</label>
                        <SearchableSelect
                            id="profile-a"
                            placeholder="Search profiles..."
                            options=all_options
                            value=path_a
                            on_select=move |v| set_path_a.set(v)
                        />
                    </div>
                    <div class="form-group">
                        <label class="nd-label">"Profile B"</label>
                        <SearchableSelect
                            id="profile-b"
                            placeholder="Search profiles..."
                            options=all_options
                            value=path_b
                            on_select=move |v| set_path_b.set(v)
                        />
                    </div>
                </div>

                <div class="diff-actions">
                    <label class="checkbox-label">
                        <input
                            type="checkbox"
                            class="diff-changed-only"
                            prop:checked=move || changed_only.get()
                            on:change=move |ev| {
                                use wasm_bindgen::JsCast;
                                let checked = ev.target()
                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                    .map(|e| e.checked())
                                    .unwrap_or(true);
                                set_changed_only.set(checked);
                            }
                        />
                        " Changed only"
                    </label>
                    <Show when=move || is_loading.get()>
                        <StateText kind=StateKind::Loading />
                    </Show>
                    <button
                        class="btn btn-primary"
                        on:click=do_compare
                        disabled=move || is_comparing.get() || path_a.get().is_empty() || path_b.get().is_empty()
                    >
                        {move || if is_comparing.get() { "Comparing…" } else { "Compare" }}
                    </button>
                </div>
            </div>

            {move || compare_error.get().map(|e| view! {
                <StateText kind=StateKind::Error message=e />
            })}

            <Show when=move || compare_result.with(|r| r.is_none()) && compare_error.with(|e| e.is_none()) && !is_comparing.get()>
                <StateText kind=StateKind::Empty message="Pick two profiles to compare" />
            </Show>

            {move || compare_result.get().map(|r| {
                let label = format!("{} vs {}", r.profile_a_name, r.profile_b_name);
                let detail = format!("of {} settings", r.total_fields);
                let name_a = r.profile_a_name.clone();
                let name_b = r.profile_b_name.clone();
                let categories = r.categories.clone();
                view! {
                    <div class="diff-results">
                        <Hero
                            label=label
                            value=r.changed_fields.to_string()
                            unit=" differences"
                            detail=detail
                        />
                        <table class="nd-table diff-table">
                            <thead>
                                <tr>
                                    <th class="diff-col-setting">"Setting"</th>
                                    <th class="diff-col-value" title=name_a.clone()>{name_a.clone()}</th>
                                    <th class="diff-col-value" title=name_b.clone()>{name_b.clone()}</th>
                                </tr>
                            </thead>
                            {move || visible_categories(&categories, changed_only.get()).into_iter().map(|cat| {
                                let name = cat.category.clone();
                                let toggle = name.clone();
                                let open_check = name.clone();
                                let rows_check = name.clone();
                                let changes = cat.diffs.iter().filter(|d| is_changed(d)).count();
                                let count_label = if changes == 1 { "1 change".to_string() } else { format!("{changes} changes") };
                                let rows: Vec<_> = cat.diffs.into_iter().map(|d| {
                                    let class = if is_changed(&d) { "diff-row changed" } else { "diff-row identical" };
                                    view! {
                                        <tr class=class>
                                            <td class="diff-key">{d.label}</td>
                                            <td class="diff-val diff-val-a nd-num">{d.base_value}</td>
                                            <td class="diff-val diff-val-b nd-num">{d.new_value}</td>
                                        </tr>
                                    }
                                }).collect();
                                view! {
                                    <tbody class="diff-category-group">
                                        <tr class="diff-category-row">
                                            <td colspan="3">
                                                <button
                                                    type="button"
                                                    class="diff-category-header"
                                                    aria-expanded=move || (!collapsed.get().contains(&open_check)).to_string()
                                                    on:click=move |_| toggle_category(toggle.clone())
                                                >
                                                    <span class="diff-category-name nd-label">{name}</span>
                                                    <span class="diff-category-count">{count_label}</span>
                                                </button>
                                            </td>
                                        </tr>
                                        <Show when=move || !collapsed.get().contains(&rows_check)>
                                            {rows.clone()}
                                        </Show>
                                    </tbody>
                                }
                            }).collect::<Vec<_>>()}
                        </table>
                    </div>
                }
            })}
        </div>
    }
```

If `r.categories.clone()` and `r.changed_fields` need `r` to be borrowed rather than moved, adjust the bindings; the behaviour must stay the same. The old summary line ("X of Y fields differ"), the `<h2>`, the description and the `▶` arrows go.

(e) In `src/pages/profile_diff.css`:
- Remove the rules for classes that no longer exist (check each with `grep`): `.diff-summary`, `.diff-summary-text`, `.diff-error`, `.diff-category-arrow`.
- Keep the `.diff-table` WebKit border-collapse fix: the browser test measures row height.
- Add:

```css
.diff-results {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-lg);
    margin-top: var(--nd-space-lg);
}

.diff-row.changed td {
    color: var(--nd-text-display);
}

.diff-row.identical td {
    color: var(--nd-text-disabled);
}

.diff-category-header {
    display: flex;
    gap: var(--nd-space-md);
    align-items: baseline;
    width: 100%;
    padding: var(--nd-space-sm) 0;
    background: transparent;
    border: 0;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
}

.diff-category-header:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.diff-category-count {
    font-family: var(--nd-font-mono);
    font-size: 11px;
    color: var(--nd-text-secondary);
}
```

- [ ] **Step 4: Update the browser steps**

In `tests/webkit/app-flows.mjs`, replace the body of "compare renders a categorised diff" with:

```js
    await page.locator(".diff-actions button.btn-primary").first().click();
    await page.waitForSelector(".diff-results", { timeout: 20000 });
    const hero = (await page.locator(".diff-results .nd-hero-number").textContent()).trim();
    if (hero !== "3") throw new Error(`hero reads "${hero}"`);
    const calls = await callsOf("compare_profiles");
    if (calls.at(-1)?.args?.showIdentical !== true) {
      throw new Error(`compare was not asked for every field: ${JSON.stringify(calls.at(-1)?.args)}`);
    }
    const categories = await page.locator(".diff-category-name").count();
    const rows = await page.locator("tr.diff-row.changed").count();
    if (categories !== 2) throw new Error(`expected 2 categories, got ${categories}`);
    if (rows !== 3) throw new Error(`expected 3 changed rows, got ${rows}`);
    return `${hero} differences, ${categories} categories`;
```

`callsOf` is defined in the slice section, below this step. If it isn't in scope here, move its one-line `const` up next to `openTab`, as in Task 2, and note it. The wrapper sends `showIdentical` (`CompareProfilesArgs` in `src/commands.rs` is camelCase).

After "diff table rows have real height", add:

```js
  await step(run, page, "Changed only hides identical rows without comparing again", async () => {
    const withIdentical = {
      ...FIXTURES.compare_profiles,
      categories: [
        ...FIXTURES.compare_profiles.categories,
        { category: "Cooling", diffs: [{ key: "fan_max_speed", label: "Fan Max", base_value: "100", new_value: "100" }] },
      ],
    };
    await withFixtures({ compare_profiles: withIdentical }, async () => {
      await page.locator(".diff-actions button.btn-primary").first().click();
      await page.waitForFunction(() => document.querySelectorAll("tr.diff-row").length === 3, null, { timeout: 5000 });
      const before = (await callsOf("compare_profiles")).length;
      await page.locator(".diff-changed-only").uncheck();
      await page.waitForSelector("tr.diff-row.identical", { timeout: 5000 });
      if ((await page.locator(".diff-category-name").count()) !== 3) throw new Error("Cooling category not shown");
      if ((await callsOf("compare_profiles")).length !== before) throw new Error("toggling re-compared");
      await page.locator(".diff-changed-only").check();
      await page.waitForSelector("tr.diff-row.identical", { state: "detached", timeout: 5000 });
    });
  });
```

- [ ] **Step 5: Run everything**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo fmt --check && trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: all pass in both engines. `layout.mjs`'s `/profiles/compare` case still finds `.ss-dropdown`.

- [ ] **Step 6: Commit**

```bash
git add src/pages/diff_view.rs src/pages/mod.rs src/pages/profile_diff.rs src/pages/profile_diff.css tests/webkit/app-flows.mjs
git commit -m "feat(profiles): Compare leads with the difference count; Changed only filters on the client"
```

---

### Task 4: Filament · Batch

**Files:**
- Create: `src/pages/batch_view.rs`
- Modify: `src/pages/mod.rs`, `src/pages/batch_generate.rs`, `src/pages/batch_generate.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes: `commands::{BatchProgress { total, completed, succeeded, failed, results }, BatchEntry { filament_name, brand, material, success, profile_name, error }}`.
- Produces:
  - `batch_view::batch_line(&BatchProgress) -> String`
  - `batch_view::batch_percent(&BatchProgress) -> u32`
  - `batch_view::BATCH_SEGMENTS: u32 = 12`

- [ ] **Step 1: Write the failing tests**

Create `src/pages/batch_view.rs` with the tests module, and add `pub mod batch_view;` to `src/pages/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn run(total: usize, succeeded: usize) -> BatchProgress {
        BatchProgress {
            total,
            completed: total,
            succeeded,
            failed: total - succeeded,
            results: vec![],
        }
    }

    #[test]
    fn the_line_says_what_failed() {
        assert_eq!(batch_line(&run(3, 3)), "All 3 generated");
        assert_eq!(batch_line(&run(3, 2)), "1 failed");
        assert_eq!(batch_line(&run(5, 1)), "4 failed");
    }

    #[test]
    fn percent_is_the_share_that_succeeded() {
        assert_eq!(batch_percent(&run(3, 2)), 66);
        assert_eq!(batch_percent(&run(4, 4)), 100);
        assert_eq!(batch_percent(&run(0, 0)), 0);
    }
}
```

Run: `cargo test --bin bambumate batch_view`
Expected: compile errors.

- [ ] **Step 2: Implement**

Above the tests:

```rust
//! What Batch shows after a run (spec 2026-10-17 §4). The backend returns
//! the whole batch at once, so there is no live progress.

pub use crate::commands::BatchProgress;

/// Segments in the results bar.
pub const BATCH_SEGMENTS: u32 = 12;

pub fn batch_line(p: &BatchProgress) -> String {
    match p.failed {
        0 => format!("All {} generated", p.total),
        n => format!("{n} failed"),
    }
}

pub fn batch_percent(p: &BatchProgress) -> u32 {
    if p.total == 0 {
        0
    } else {
        (p.succeeded * 100 / p.total) as u32
    }
}
```

Run: `cargo test --bin bambumate batch_view`
Expected: 2 passed.

- [ ] **Step 3: Rebuild the page**

In `src/pages/batch_generate.rs`:

(a) Add these imports:

```rust
use crate::components::patterns::{Hero, SegmentedBar, StateKind, StateText};
use crate::pages::batch_view::{batch_line, batch_percent, BATCH_SEGMENTS};
```

(b) In the `view!`:
- Delete the `<h2>` and the description.
- Make the form labels `class="nd-label"`.
- Replace the error block with `{move || gen_error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}`.
- Inside the brand `Show`, replace `<div class="batch-error">{err}</div>` with `<StateText kind=StateKind::Error message=err />`.
- Replace the spinner block with:

```rust
            <Show when=move || is_generating.get()>
                <StateText kind=StateKind::Loading message="Generating profiles" />
            </Show>
```

- Replace the results block with the following. It keeps the `.batch-results`, `.batch-table` and `tr.row-fail` hooks:

```rust
            {move || result.get().map(|r| {
                let label = format!("Batch · {}", selected_brand.get_untracked());
                let percent = batch_percent(&r);
                view! {
                    <div class="batch-results">
                        <Hero
                            label=label
                            value=format!("{}/{}", r.succeeded, r.total)
                            detail=batch_line(&r)
                            tone=if r.failed > 0 { crate::components::patterns::Tone::Warning } else { crate::components::patterns::Tone::Signal }
                        />
                        <SegmentedBar percent=percent total=BATCH_SEGMENTS label="Profiles generated" />
                        <table class="nd-table batch-table">
                            <thead>
                                <tr>
                                    <th>"Status"</th>
                                    <th>"Filament"</th>
                                    <th>"Material"</th>
                                    <th>"Profile"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {r.results.iter().map(|entry| {
                                    let status = if entry.success { "OK" } else { "FAIL" };
                                    let class = if entry.success { "row-success" } else { "row-fail" };
                                    let name = entry.profile_name.clone().unwrap_or_default();
                                    let err = entry.error.clone().unwrap_or_default();
                                    view! {
                                        <tr class=class>
                                            <td class="status-cell nd-label">{status}</td>
                                            <td>{entry.filament_name.clone()}</td>
                                            <td class="nd-label">{entry.material.clone()}</td>
                                            <td>
                                                {name}
                                                {(!err.is_empty()).then(|| view! {
                                                    <span class="error-hint" title=err.clone()>{format!(" ({})", err)}</span>
                                                })}
                                            </td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>
                    </div>
                }
            })}
```

   `SegmentedBar`'s `percent` is a `Signal<u32>`, so pass `percent` (a `u32`, converted by `into`).

(c) In `src/pages/batch_generate.css`:
- Remove the rules for `.batch-summary`, `.batch-stat`, `.batch-success`, `.batch-fail`, `.batch-error`, `.loading-spinner` and `.spinner`, but only where they're defined in this file and unused elsewhere (`grep -rn` each class in `src/`).
- Add:

```css
.batch-results {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-md);
    margin-top: var(--nd-space-lg);
}

.row-success .status-cell {
    color: var(--nd-signal);
}

.row-fail .status-cell {
    color: var(--nd-accent);
}

.error-hint {
    color: var(--nd-text-secondary);
}
```

- [ ] **Step 4: Update the browser step**

In "batch run reports per-filament results", replace the summary line and the return statement with:

```js
    const hero = (await page.locator(".batch-results .nd-hero-number").textContent()).trim();
    if (hero !== "2/3") throw new Error(`hero reads "${hero}"`);
    const detail = (await page.locator(".batch-results .nd-hero-detail").textContent()).trim();
    if (detail !== "1 failed") throw new Error(`detail reads "${detail}"`);
    const on = await page.locator(".batch-results .nd-seg-on").count();
    if (on !== 8) throw new Error(`${on} segments for 2 of 3`);
```

and keep the `tr.row-fail` count check. End the step with `return \`${hero}, ${detail}\`;`. 66% of 12 segments rounds to 8.

- [ ] **Step 5: Run everything**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo fmt --check && trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: all pass in both engines.

- [ ] **Step 6: Commit**

```bash
git add src/pages/batch_view.rs src/pages/mod.rs src/pages/batch_generate.rs src/pages/batch_generate.css tests/webkit/app-flows.mjs
git commit -m "feat(filament): Batch leads with succeeded out of total"
```

---

### Task 5: Filament · Search

**Files:**
- Create: `src/pages/filament_steps.rs`
- Modify: `src/pages/mod.rs`, `src/pages/filament_search.rs`, `src/pages/filament_search.css`, `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes:
  - From Task 1: `HeroStats`, `StepStrip`.
  - From patterns: `StateText` and `StateKind`.
  - `commands::FilamentSpecs`, with fields used here: `brand`, `material`, `nozzle_temperature`, `nozzle_temp_max`, `hot_plate_temp`, `bed_temp_max`, `filament_flow_ratio`.
- Produces, in `filament_steps`:
  - `enum SearchStep { Search, Specs, Tune, Install }`
  - `SEARCH_STEPS: &[&str] = &["Specs", "Tune", "Install"]`
  - `fn search_step(has_specs, merging, editing, has_results, has_installs) -> SearchStep`
  - `fn step_index(SearchStep) -> usize`
  - `fn spec_label(&FilamentSpecs) -> String`
  - `fn spec_stats(&FilamentSpecs) -> Vec<(String, String)>`

- [ ] **Step 1: Write the failing tests**

Create `src/pages/filament_steps.rs` with the tests module, and add `pub mod filament_steps;` to `src/pages/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn specs(json: serde_json::Value) -> FilamentSpecs {
        let mut base = serde_json::json!({
            "serial": "", "brand": "Polymaker", "material": "PLA",
            "source_url": "", "extraction_confidence": 0.9
        });
        base.as_object_mut().unwrap().extend(json.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn the_step_follows_the_flow() {
        assert_eq!(search_step(false, false, false, false, false), SearchStep::Search);
        assert_eq!(search_step(true, false, false, false, false), SearchStep::Specs);
        assert_eq!(search_step(true, true, false, false, false), SearchStep::Tune);
        assert_eq!(search_step(true, false, true, false, false), SearchStep::Tune);
        assert_eq!(search_step(true, false, true, true, false), SearchStep::Install);
        assert_eq!(search_step(true, false, false, false, true), SearchStep::Install);
    }

    #[test]
    fn step_indexes_match_the_strip() {
        assert_eq!(SEARCH_STEPS, &["Specs", "Tune", "Install"]);
        assert_eq!(step_index(SearchStep::Specs), 0);
        assert_eq!(step_index(SearchStep::Tune), 1);
        assert_eq!(step_index(SearchStep::Install), 2);
        assert_eq!(step_index(SearchStep::Search), 0);
    }

    #[test]
    fn hero_stats_prefer_the_printing_values() {
        let s = specs(serde_json::json!({
            "nozzle_temperature": 220, "nozzle_temp_max": 230,
            "hot_plate_temp": 65, "bed_temp_max": 70, "filament_flow_ratio": 0.98
        }));
        assert_eq!(spec_label(&s), "Polymaker · PLA");
        assert_eq!(
            spec_stats(&s),
            vec![
                ("Nozzle".to_string(), "220°".to_string()),
                ("Bed".to_string(), "65°".to_string()),
                ("Flow".to_string(), "0.98".to_string()),
            ]
        );
    }

    #[test]
    fn hero_stats_fall_back_to_the_ranges_then_a_dash() {
        let s = specs(serde_json::json!({ "nozzle_temp_max": 230, "bed_temp_max": 70 }));
        assert_eq!(
            spec_stats(&s),
            vec![
                ("Nozzle".to_string(), "230°".to_string()),
                ("Bed".to_string(), "70°".to_string()),
                ("Flow".to_string(), "—".to_string()),
            ]
        );
        let unbranded = specs(serde_json::json!({ "brand": "" }));
        assert_eq!(spec_label(&unbranded), "PLA");
    }
}
```

Run: `cargo test --bin bambumate filament_steps`
Expected: compile errors.

- [ ] **Step 2: Implement**

Above the tests:

```rust
//! Where Filament Search is in its flow, and what its hero shows (spec
//! 2026-10-17 §4, amended 2026-10-20: a step strip instead of collapsible
//! sections). Pure, so it is tested without a browser.

use crate::commands::FilamentSpecs;

/// The steps the strip shows, in order.
pub const SEARCH_STEPS: &[&str] = &["Specs", "Tune", "Install"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStep {
    /// No filament picked yet: the search box is the hero.
    Search,
    /// The filament card and the base-profile choice.
    Specs,
    /// The optional merge, then the editor.
    Tune,
    /// The preview, then the install result.
    Install,
}

pub fn search_step(
    has_specs: bool,
    merging: bool,
    editing: bool,
    has_results: bool,
    has_installs: bool,
) -> SearchStep {
    if !has_specs {
        SearchStep::Search
    } else if has_results || has_installs {
        SearchStep::Install
    } else if merging || editing {
        SearchStep::Tune
    } else {
        SearchStep::Specs
    }
}

/// The step's position in `SEARCH_STEPS`; Search sits before the strip and
/// reads as the first step.
pub fn step_index(step: SearchStep) -> usize {
    match step {
        SearchStep::Search | SearchStep::Specs => 0,
        SearchStep::Tune => 1,
        SearchStep::Install => 2,
    }
}

/// `Polymaker · PLA`, leaving out what's blank.
pub fn spec_label(s: &FilamentSpecs) -> String {
    [s.brand.trim(), s.material.trim()]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Nozzle, bed and flow for the hero: the printing values when known, else
/// the top of the range, else a dash.
pub fn spec_stats(s: &FilamentSpecs) -> Vec<(String, String)> {
    let temp = |v: Option<u16>| v.map(|t| format!("{t}°")).unwrap_or_else(|| "—".into());
    vec![
        ("Nozzle".into(), temp(s.nozzle_temperature.or(s.nozzle_temp_max))),
        ("Bed".into(), temp(s.hot_plate_temp.or(s.bed_temp_max))),
        (
            "Flow".into(),
            s.filament_flow_ratio
                .map(|f| format!("{f:.2}"))
                .unwrap_or_else(|| "—".into()),
        ),
    ]
}
```

Run: `cargo test --bin bambumate filament_steps`
Expected: 4 passed.

- [ ] **Step 3: Restructure the page**

In `src/pages/filament_search.rs`:

(a) Add these imports:

```rust
use crate::components::patterns::{HeroStats, StateKind, StateText, StepStrip};
use crate::pages::filament_steps::{search_step, spec_label, spec_stats, step_index, SearchStep, SEARCH_STEPS};
```

(b) After `on_skip_merge`, add:

```rust
    // Where the flow is; drives the hero and the step strip.
    let step = Memo::new(move |_| {
        search_step(
            current_specs.with(|s| s.is_some()),
            show_merge_screen.get(),
            show_editor.get(),
            !generate_results.with(|r| r.is_empty()),
            !install_results.with(|r| r.is_empty()),
        )
    });

    // Back from the strip. Not after an install has run: there is nothing
    // to go back to then.
    let go_back = move |index: usize| {
        if !install_results.with_untracked(|r| r.is_empty()) {
            return;
        }
        match index {
            0 => {
                cancel_generate();
                set_show_merge_screen.set(false);
                set_show_editor.set(false);
            }
            1 => cancel_generate(),
            _ => {}
        }
    };
```

`cancel_generate` clears the results, and the editor stays open because `show_editor` is still true, so step 1 lands on the editor.

(c) At the top of the `view!`:
- Delete the `<h2>"Filament Search"</h2>` and the description paragraph.
- Wrap the existing `catalog-status` div and the whole `search-container` div so they render only at the Search step, inside a search hero:

```rust
            <Show when=move || step.get() == SearchStep::Search>
                <section class="nd-hero fs-search-hero">
                    <p class="nd-label nd-hero-label">
                        {move || if filament_ai_enabled.get() { "Find a filament" } else { "Find a filament · web only" }}
                    </p>
                    // the existing <div class="search-container">…</div>, unchanged
                    // the existing <div class="catalog-status">…</div>, unchanged, moved below the search box
                </section>
            </Show>
```

   Keep both blocks' markup and handlers exactly as they are; only their position changes. Add the class `fs-hero-input` to the main search `<input class="search-input" …>`, so its class becomes `"search-input fs-hero-input"`. Do not add it to the URL input.

- Directly after that `Show`, add the specs hero and step strip:

```rust
            {move || current_specs.get().map(|s| view! {
                <div class="fs-specs-hero">
                    <HeroStats label=spec_label(&s) stats=spec_stats(&s) />
                    <div class="fs-hero-actions">
                        <button type="button" class="btn btn-secondary fs-search-another"
                            on:click=move |_| reset_search()>
                            "Search another"
                        </button>
                    </div>
                    <StepStrip
                        steps=SEARCH_STEPS
                        current=Signal::derive(move || step_index(step.get()))
                        can_go_back=Signal::derive(move || install_results.with(|r| r.is_empty()))
                        on_back=Callback::new(move |i: usize| go_back(i))
                    />
                </div>
            })}
```

(d) Replace the "Fetching indicator" block with:

```rust
            <Show when=move || is_fetching.get()>
                <StateText kind=StateKind::Loading message="Fetching specifications" />
            </Show>
```

   Replace the fetch error's `<div class="error-message"><strong>"Error: "</strong>{e}</div>` with `<StateText kind=StateKind::Error message=e />`.

   Replace the "Generating indicator" block with `<Show when=move || is_generating.get()><StateText kind=StateKind::Loading message="Generating profiles" /></Show>`.

   Replace the per-nozzle generation errors and the install errors (`<div class="error-message"><strong>{…}</strong>{e}</div>`) with `<StateText kind=StateKind::Error message=format!("Generation failed ({label}): {e}") />` and `…format!("Installation failed ({name}): {e}")…` respectively.

(e) Make these headings mono labels: `<h4>"Base Profile (Installed in Bambu Studio)"</h4>` becomes `<p class="nd-label">"Base profile (installed in Bambu Studio)"</p>`, and the merge screen's `<h3>"Compare & Merge Settings"</h3>` becomes `<p class="nd-label">"Compare and merge settings"</p>`. In `.multi-profile-title`, change the `<h3>` element to `<p class="nd-label multi-profile-title">`, keeping the text. In `.multi-profile-table`, add `nd-table` to its class.

(f) In `src/pages/filament_search.css`:
- Remove `.loading-spinner`, `.spinner`, `.error-message` and the `.page-description` overrides if they are now unused (`grep -rn` each in `src/`).
- Add:

```css
.fs-search-hero {
    gap: var(--nd-space-md);
}

.fs-hero-input {
    font-size: 24px;
    padding: 14px 20px;
}

.fs-specs-hero {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-md);
    margin-bottom: var(--nd-space-xl);
}

.fs-hero-actions {
    display: flex;
    gap: var(--nd-space-sm);
}
```

   If `.search-input` already sets `font-size` with higher specificity, raise `.fs-hero-input` to `.search-bar .fs-hero-input`. Check the result in the screenshots in Step 6.

- [ ] **Step 4: Update and add browser steps**

The existing search steps (fill `.search-input`, pick a `.suggestion-item`, `.filament-card`, `.base-profiles-section`, `.filament-card-generate-btn`, `.editor-section`) must keep passing unchanged.

The "catalog status" step at the start of the filament section waits for `.catalog-status`, which is still rendered at the Search step. If an earlier step left specs loaded so that `.catalog-status` isn't shown, make the step call `reset_search` first by clicking `.fs-search-another` when it is present. Note this in your report.

After "Generate opens the specs editor", add:

```js
    await step(run, page, "the filament hero and step strip follow the flow", async () => {
      const stats = await page.locator(".fs-specs-hero .nd-hero-stat-value").allTextContents();
      if (stats.join(" ") !== "220° 65° 0.98") throw new Error(`hero reads "${stats.join(" ")}"`);
      const current = await page.locator('.nd-step[data-state="current"] .nd-step-name').textContent();
      if (current !== "Tune") throw new Error(`current step "${current}"`);
      if ((await page.locator(".search-container").count()) !== 0) throw new Error("search box still shown after picking");
      await page.click('.nd-step[data-state="done"] button.nd-step-btn');
      await page.waitForSelector(".filament-card", { timeout: 5000 });
      const back = await page.locator('.nd-step[data-state="current"] .nd-step-name').textContent();
      if (back !== "Specs") throw new Error(`after Back the current step is "${back}"`);
      // Return to the editor for the steps that follow.
      await page.click(".filament-card-generate-btn");
      await page.waitForSelector(".editor-section", { timeout: 20000 });
    });
```

Then add a step at the end of the filament section, before the next `// --` section. Check first that no later step depends on the specs staying loaded, and say so in your report.

```js
    await step(run, page, "Search another brings the search box back", async () => {
      await page.click(".fs-search-another");
      await page.waitForSelector(".fs-search-hero .search-input", { timeout: 5000 });
      if ((await page.locator(".fs-specs-hero").count()) !== 0) throw new Error("specs hero still shown");
    });
```

If a later step in the filament section expects the specs or the editor to still be there, put this step after it instead.

- [ ] **Step 5: Run everything**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo fmt --check && trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: all pass in both engines. `layout.mjs`'s `/filament` case still finds `.search-container` on a fresh load, because no filament is picked.

- [ ] **Step 6: Look at all four pages**

Run: `(cd tests/webkit && node screens.mjs ../..)`

Open these screenshots in `tests/webkit/screens/`:
- `dark-1280-filament.png`
- `bambu-1280-filament.png`
- `dark-1280-filament-batch.png`
- `dark-1280-profiles.png`
- `dark-1280-profiles-compare.png`
- `dark-900-profiles.png`

Each page should have one hero, and nothing should overflow at 900px. Fix layout problems in the page CSS, then rerun Step 5. Do not commit the PNGs.

- [ ] **Step 7: Commit**

```bash
git add src/pages/filament_steps.rs src/pages/mod.rs src/pages/filament_search.rs src/pages/filament_search.css tests/webkit/app-flows.mjs
git commit -m "feat(filament): Search leads with the search box, then the filament's numbers and a step strip"
```

---

## Self-review notes

How the plan covers the amended §4 rows:

| Page | Covered by |
|---|---|
| Search: search hero, then three Doto stats, step strip with back, "Search another", StateText | Task 5 |
| Batch: hero after a run, SegmentedBar, `.nd-table`, Loading while running | Task 4 |
| Library: nozzle hero, four stat rows, not-syncing dot only, pill actions, `.nd-table` fields | Task 2 |
| Compare: difference-count hero, "Changed only" on by default and client-side, lit/disabled rows, `.nd-table` | Task 3 |

The wave 1 deferred item "Profiles header `.btn-icon` buttons are not pills" is closed by Task 2.

Types used across tasks:
- `HeroStats`/`StepStrip` (Task 1) are used with the same prop names in Task 5.
- `key_numbers`, `visible_categories`, `batch_line` and `batch_percent` are each used only by their own page.
- `SegmentedBar`'s `label` prop is required since wave 1's fix wave; both uses here pass it.
