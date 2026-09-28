# Nothing Design System and App Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the whole BambuMate app the Nothing design language: dark by default, Bambu green as the signal colour, soft flat components, and an icon rail that expands into a labeled sidebar. Page layouts stay as they are.

**Architecture:**
- `style/tokens.css` becomes the app-wide token source, keyed by `data-theme` on `<html>`.
- `style/main.css` remaps its 36 legacy variables onto tokens and restyles the shared component classes in place. Class names are unchanged, so pages need no markup edits.
- Page and component CSS drop hard-coded colours, gradients and shadows.
- The sidebar is rebuilt as a rail with inline-SVG monoline icons.
- A new WebKit contrast test guards both themes.

**Tech Stack:** Leptos 0.8 (CSR/WASM), Trunk, plain CSS custom properties, Playwright WebKit and Chromium tests.

**Spec:** `docs/superpowers/specs/2026-09-27-nothing-design-system-design.md`

## Global Constraints

- **Theme values:** `data-theme` values stay `dark` and `bambu` (light). `src/theme.rs` also maps `"light"` → `bambu`. Only a missing stored theme defaults to `dark`.
- **Dark tokens:**

  | Token | Value |
  |---|---|
  | page | `#000000` |
  | surface | `#111111` |
  | raised | `#1A1A1A` |
  | border | `#222222` |
  | border-visible | `#333333` |
  | disabled | `#666666` |
  | secondary | `#999999` |
  | primary | `#E8E8E8` |
  | display | `#FFFFFF` |
  | signal | `#00AE42` |
  | error | `#F0414A` |
  | warning | `#D4A843` |
  | interactive | `#5B9BF6` |

- **Light tokens:**

  | Token | Value |
  |---|---|
  | page | `#F5F5F5` |
  | surface | `#FFFFFF` |
  | raised | `#F0F0F0` |
  | border | `#E8E8E8` |
  | border-visible | `#CCCCCC` |
  | disabled | `#8A8A8A` |
  | secondary | `#666666` |
  | primary | `#1A1A1A` |
  | display | `#000000` |
  | signal | `#007A34` |
  | error | `#D71921` |
  | warning | `#7D5E0E` |
  | interactive | `#007AFF` |

- **Contrast:** at least 4.5:1 against page and surface for primary, secondary, signal, warning and error text. At least 3:1 for disabled.
- **No decoration:**
  - no shadows, no gradients, no blur;
  - no bounce or scale animations — only opacity and colour, over 150–250ms with `cubic-bezier(0.25,0.1,0.25,1)`;
  - no emoji as UI.
- **Radii:** buttons, toggles, segments and badges are pills (999px); inputs 10px; cards and modals 14px.
- **Fonts:** body is Space Grotesk. Labels and numbers are Space Mono, and labels are ALL CAPS. Doto is for hero numbers only. All fonts are bundled locally (OFL); nothing loads from a CDN.
- **Class names:** existing class names (`.sidebar`, `.nav-link`, `.btn*`, `.input`, `.card`, `.content`, …) must keep working. Tests and pages rely on them.
- **Rail:** 64px collapsed, 220px expanded. It overlays the content and never pushes it. It opens on hover after a 150ms delay, or immediately on `:focus-within`.
- **Scope:** do not change page layouts or page Rust code, except where a task names it.

## File Map

| File | Responsibility |
|---|---|
| `style/tokens.css` | Token values per theme, `@font-face` (+ Doto), `.nd` utility classes |
| `style/fonts/Doto-Variable.ttf` | Dot-matrix display font (OFL) |
| `style/main.css` | Legacy-variable remap, base styles, shared component styles, rail shell |
| `src/pages/*.css`, `src/components/*.css` | Page/component styles with hard-coded colours replaced by tokens |
| `src/theme.rs` | Theme normalization (adds `"light"`), with tests |
| `src/app.rs` | Default theme signal `dark` |
| `src/components/icons.rs` | Monoline SVG icon components |
| `src/components/sidebar.rs` | Rail/sidebar markup, `NAV_ITEMS`, active route |
| `tests/webkit/contrast.mjs` | WCAG contrast check of tokens in both themes |
| `tests/webkit/app-flows.mjs`, `tests/webkit/layout.mjs` | Rail behaviour steps |
| `.github/workflows/test.yml` | Runs `contrast.mjs` in the `webkit-ui` job |

---

### Task 1: App-wide tokens, dark default, Doto, contrast test

**Files:**
- Modify: `style/tokens.css`
- Create: `style/fonts/Doto-Variable.ttf`
- Modify: `src/theme.rs`, `src/app.rs`
- Create: `tests/webkit/contrast.mjs`

**Interfaces:**
- Produces:
  - CSS custom properties on `:root` for both themes: `--nd-black`, `--nd-surface`, `--nd-surface-raised`, `--nd-border`, `--nd-border-visible`, `--nd-text-disabled`, `--nd-text-secondary`, `--nd-text-primary`, `--nd-text-display`, `--nd-signal`, `--nd-accent`, `--nd-accent-subtle`, `--nd-success`, `--nd-warning`, `--nd-interactive`, `--nd-space-*`, `--nd-font-body`, `--nd-font-mono`, `--nd-font-display`, `--nd-ease`.
  - Utility classes `.nd-label`, `.nd-mono`, `.nd-display`, `.nd-dot-grid`, which work anywhere (no `.nd` ancestor needed).
  - In `src/theme.rs`: `normalize_theme("light") == "bambu"`.

- [ ] **Step 1: Write the failing contrast test**

Create `tests/webkit/contrast.mjs`:

```js
// WCAG contrast of the design tokens in both themes, in real WebKit and
// Chromium. Reads the computed custom properties from style/tokens.css, so it
// checks what the app actually ships.
import { chromium, webkit } from "playwright";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const repoRoot = resolve(process.argv[2] ?? "../..");
const css = readFileSync(resolve(repoRoot, "style/tokens.css"), "utf8").replace(/@font-face\s*{[^}]*}/g, "");

const TEXT = [
  ["--nd-text-primary", 4.5],
  ["--nd-text-secondary", 4.5],
  ["--nd-text-display", 4.5],
  ["--nd-signal", 4.5],
  ["--nd-warning", 4.5],
  ["--nd-accent", 4.5],
  ["--nd-text-disabled", 3],
];
const BACKGROUNDS = ["--nd-black", "--nd-surface"];
const THEMES = ["dark", "bambu"];

function luminance(hex) {
  const n = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(n.slice(i, i + 2), 16) / 255);
  const lin = (c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a, b) {
  const [l1, l2] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

async function run(browserType, name) {
  const browser = await browserType.launch();
  const page = await browser.newPage();
  await page.setContent(`<!doctype html><html><head><style>${css}</style></head><body></body></html>`);
  const failures = [];
  for (const theme of THEMES) {
    const vars = await page.evaluate(
      ({ theme, names }) => {
        document.documentElement.setAttribute("data-theme", theme);
        const cs = getComputedStyle(document.documentElement);
        return Object.fromEntries(names.map((n) => [n, cs.getPropertyValue(n).trim()]));
      },
      { theme, names: [...TEXT.map(([n]) => n), ...BACKGROUNDS] }
    );
    for (const [fg, min] of TEXT) {
      for (const bg of BACKGROUNDS) {
        const f = vars[fg];
        const b = vars[bg];
        if (!/^#[0-9a-fA-F]{6}$/.test(f) || !/^#[0-9a-fA-F]{6}$/.test(b)) {
          failures.push(`${theme}: ${fg}=${f || "(unset)"} on ${bg}=${b || "(unset)"} is not a #rrggbb token`);
          continue;
        }
        const ratio = contrast(f, b);
        const ok = ratio >= min;
        console.log(`  ${name} ${ok ? "OK  " : "FAIL"} ${theme} ${fg} on ${bg}: ${ratio.toFixed(2)}:1 (min ${min})`);
        if (!ok) failures.push(`${theme}: ${fg} on ${bg} = ${ratio.toFixed(2)}:1 < ${min}`);
      }
    }
  }
  await browser.close();
  return failures;
}

const failures = [...(await run(webkit, "webkit")), ...(await run(chromium, "chromium"))];
if (failures.length) {
  console.error(`\n${failures.length} contrast failure(s):\n` + failures.join("\n"));
  process.exit(1);
}
console.log("\nAll token contrasts pass.");
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd tests/webkit && node contrast.mjs ../..` (Playwright is installed in `tests/webkit/node_modules`. If not, first run `npm install --no-audit --no-fund --no-save playwright@1.49.1 && npx playwright install webkit chromium`.)
Expected: FAIL. `--nd-signal` is unset, and today's tokens only apply under `.nd`, so `:root` reads empty.

- [ ] **Step 3: Vendor Doto**

```bash
curl -fsSL -o "style/fonts/Doto-Variable.ttf" "https://github.com/google/fonts/raw/main/ofl/doto/Doto%5BROND%2Cwght%5D.ttf"
file style/fonts/Doto-Variable.ttf
```

Expected: "TrueType Font data". If the URL 404s, look up the current filename at https://github.com/google/fonts/tree/main/ofl/doto. Doto is OFL, like the other fonts; `style/fonts/OFL.txt` already covers the OFL terms. Append a line to it naming Doto's copyright holder from the repo's `ofl/doto/OFL.txt`.

- [ ] **Step 4: Rewrite `style/tokens.css`**

Replace the whole file:

```css
/* Nothing design tokens: app-wide. Dark is the default; data-theme="bambu"
   (legacy value) or "light" selects the light set. Fonts are bundled (OFL,
   see style/fonts/OFL.txt): the app runs offline. */

@font-face {
    font-family: "Space Grotesk";
    src: url("fonts/SpaceGrotesk-Variable.ttf") format("truetype");
    font-weight: 300 700;
    font-display: swap;
}
@font-face {
    font-family: "Space Mono";
    src: url("fonts/SpaceMono-Regular.ttf") format("truetype");
    font-weight: 400;
    font-display: swap;
}
@font-face {
    font-family: "Space Mono";
    src: url("fonts/SpaceMono-Bold.ttf") format("truetype");
    font-weight: 700;
    font-display: swap;
}
@font-face {
    font-family: "Doto";
    src: url("fonts/Doto-Variable.ttf") format("truetype");
    font-weight: 100 900;
    font-display: swap;
}

:root,
[data-theme="dark"] {
    color-scheme: dark;
    --nd-black: #000000;
    --nd-surface: #111111;
    --nd-surface-raised: #1a1a1a;
    --nd-border: #222222;
    --nd-border-visible: #333333;
    --nd-text-disabled: #666666;
    --nd-text-secondary: #999999;
    --nd-text-primary: #e8e8e8;
    --nd-text-display: #ffffff;
    --nd-signal: #00ae42;
    --nd-warning: #d4a843;
    --nd-accent: #f0414a;
    --nd-interactive: #5b9bf6;
    --nd-dot: #1f1f1f;
}

[data-theme="bambu"],
[data-theme="light"] {
    color-scheme: light;
    --nd-black: #f5f5f5;
    --nd-surface: #ffffff;
    --nd-surface-raised: #f0f0f0;
    --nd-border: #e8e8e8;
    --nd-border-visible: #cccccc;
    --nd-text-disabled: #8a8a8a;
    --nd-text-secondary: #666666;
    --nd-text-primary: #1a1a1a;
    --nd-text-display: #000000;
    --nd-signal: #007a34;
    --nd-warning: #7d5e0e;
    --nd-accent: #d71921;
    --nd-interactive: #007aff;
    --nd-dot: #e4e4e4;
}

:root {
    --nd-accent-subtle: rgba(215, 25, 33, 0.15);
    --nd-success: var(--nd-signal);

    --nd-space-xs: 4px;
    --nd-space-sm: 8px;
    --nd-space-md: 16px;
    --nd-space-lg: 24px;
    --nd-space-xl: 32px;
    --nd-space-2xl: 48px;

    --nd-radius-pill: 999px;
    --nd-radius-input: 10px;
    --nd-radius-card: 14px;

    --nd-font-body: "Space Grotesk", system-ui, sans-serif;
    --nd-font-mono: "Space Mono", "SF Mono", ui-monospace, monospace;
    --nd-font-display: "Doto", "Space Mono", monospace;
    --nd-ease: cubic-bezier(0.25, 0.1, 0.25, 1);
}

/* Kept for the agent drawer, which scopes itself with .nd. */
.nd {
    font-family: var(--nd-font-body);
    color: var(--nd-text-primary);
}

.nd-label {
    font-family: var(--nd-font-mono);
    font-size: 11px;
    line-height: 1.2;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--nd-text-secondary);
}

.nd-mono {
    font-family: var(--nd-font-mono);
}

.nd-display {
    font-family: var(--nd-font-display);
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 0.95;
    color: var(--nd-text-display);
}

.nd-dot-grid {
    background-image: radial-gradient(circle, var(--nd-dot) 0.8px, transparent 1px);
    background-size: 10px 10px;
}
```

The controller has checked every text token for contrast. Dark red is `#F0414A` because `#D71921` on black is only 4.05:1. The light theme's green, amber and disabled grey are darker for the same reason.

- [ ] **Step 5: Make `"light"` an accepted theme value and default to dark**

Replace the `normalize_theme` doc comment and match in `src/theme.rs`:

```rust
/// Normalize stored or incoming theme values to the supported set.
/// "light" and the legacy "bambu" both select the light theme; anything else
/// unknown also falls back to light so a corrupt value never hides the UI.
pub fn normalize_theme(theme: &str) -> &'static str {
    match theme {
        "dark" => "dark",
        _ => "bambu",
    }
}
```

The match itself doesn't change, because `"light"` already falls to `bambu`. Append tests to `src/theme.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::normalize_theme;

    #[test]
    fn dark_stays_dark() {
        assert_eq!(normalize_theme("dark"), "dark");
    }

    #[test]
    fn light_and_legacy_values_select_the_light_theme() {
        assert_eq!(normalize_theme("light"), "bambu");
        assert_eq!(normalize_theme("bambu"), "bambu");
        assert_eq!(normalize_theme("something-else"), "bambu");
    }
}
```

In `src/app.rs`, change the initial theme signal from `signal(String::from("bambu"))` to `signal(String::from("dark"))`. A user with no stored theme then gets dark, and a stored value still overrides it on mount.

- [ ] **Step 6: Verify**

Run:
- `cargo test --bin bambumate theme` → 2 passed.
- `trunk build && ls dist/fonts` → lists `Doto-Variable.ttf` among the fonts.
- `cd tests/webkit && node contrast.mjs ../..` → "All token contrasts pass" in both engines.

If any light-theme ratio fails, report it (do not change the spec's values silently). The spec's light values were chosen to pass.

- [ ] **Step 7: Commit**

```bash
git add style/tokens.css style/fonts src/theme.rs src/app.rs tests/webkit/contrast.mjs
git commit -m "Make Nothing tokens app-wide, default to dark, add Doto and a contrast test"
```

---

### Task 2: Remap legacy variables and base styles

**Files:**
- Modify: `style/main.css` (the two theme blocks at the top, and the `body` rule)

**Interfaces:**
- Consumes: the Task 1 tokens.
- Produces: every legacy `--bg-*`, `--text-*`, `--border-*`, `--accent*`, `--color-*`, `--success-*`, `--error-*` and `--shadow-*` variable now resolves to a Nothing token in both themes.

- [ ] **Step 1: Replace the two theme blocks**

In `style/main.css`, delete everything from the `[data-theme="bambu"],` line through the closing `}` of the `[data-theme="dark"] { … }` block (currently lines 5–95). Put this in their place:

```css
/* Legacy variables, now aliases of the Nothing tokens in style/tokens.css.
   The tokens switch per theme, so this block is theme-independent. */
:root {
    --bg-primary: var(--nd-black);
    --bg-sidebar: var(--nd-black);
    --bg-card: var(--nd-surface);
    --bg-input: var(--nd-surface);
    --bg-tertiary: var(--nd-surface);
    --bg-secondary: var(--nd-surface-raised);
    --bg-hover: var(--nd-surface-raised);
    --bg-active: var(--nd-surface-raised);
    --bg-active-hover: var(--nd-border);
    --bg-success: var(--nd-surface-raised);
    --bg-danger: var(--nd-surface-raised);
    --bg-danger-hover: var(--nd-border);
    --bg-unknown: var(--nd-surface-raised);

    --text-primary: var(--nd-text-primary);
    --text-bright: var(--nd-text-display);
    --text-secondary: var(--nd-text-secondary);
    --text-muted: var(--nd-text-secondary);
    --text-placeholder: var(--nd-text-disabled);
    --text-on-accent: var(--nd-black);

    --border-primary: var(--nd-border-visible);
    --border-success: var(--nd-border-visible);
    --border-danger: var(--nd-accent);
    --border-unknown: var(--nd-border-visible);

    --accent: var(--nd-text-display);
    --accent-hover: var(--nd-text-primary);
    --color-success: var(--nd-signal);
    --color-danger: var(--nd-accent);
    --color-warning: var(--nd-warning);
    --success-text: var(--nd-signal);
    --error-text: var(--nd-accent);
    --error-bg: var(--nd-surface-raised);
    --error-border: var(--nd-accent);

    --shadow-card: none;
    --shadow-brand: none;
    --shadow-button: none;
    --shadow-focus: 0 0 0 1px var(--nd-text-primary);
    --shadow-focus-strong: 0 0 0 1px var(--nd-text-display);
}
```

`--accent` now means "primary action" (the white or black pill), not green. Green is reserved for `--nd-signal` / `--color-success`.

- [ ] **Step 2: Flatten the base**

Replace the `body` rule's `font-family` and `background` lines:

```css
body {
    font-family: var(--nd-font-body);
    font-size: 14px;
    line-height: 1.5;
    color: var(--text-primary);
    background: var(--nd-black);
    overflow: hidden;
    height: 100vh;
}
```

In the `.sidebar` rule, delete the `box-shadow: inset -1px 0 0 rgba(255, 255, 255, 0.4);` line. Task 5 rebuilds the sidebar anyway.

- [ ] **Step 3: Verify**

Run:
- `trunk build` → succeeds.
- `grep -nE -- '--(bg|text|border|accent|color|success|error|shadow)[a-z-]*:\s*#' style/main.css` → no output (no legacy variable still holds a literal colour).
- `cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../..` → all pass.

- [ ] **Step 4: Commit**

```bash
git add style/main.css
git commit -m "Remap legacy theme variables onto the Nothing tokens"
```

---

### Task 3: Soft components

**Files:**
- Modify: `style/main.css`: rules for `.card`, `.btn`, `.btn:hover`, `.btn-primary`, `.btn-secondary`, `.btn-ghost`, `.btn-save`, `.btn-delete`, `.btn-danger`, `.btn-sm`, `.input`, `.input:focus`, `select.input`, `.status-badge`, `.status-pass`, `.status-fail`, `.status-unknown`, `.health-item`, `.health-summary`, `.toggle-input`, `.wizard-container`, `.hero-panel`
- Modify: `src/components/searchable_select.css`, `src/pages/profile_management.css` (`.modal-overlay`/`.modal`), `src/components/change_preview.css` (the overlay panel)

**Interfaces:**
- Produces: soft component styling on the existing classes, plus one new utility `.nd-stat`. `.nd-stat` is a card with the dot-grid texture; it is applied to `.hero-panel` in this task, and pages opt in later (piece 2).

- [ ] **Step 1: Replace the component rules in `style/main.css`**

Replace each listed rule's body with the version below. Keep each rule's position in the file. Delete the `.btn:hover:not(:disabled) { transform: … }` rule outright; it has no replacement.

```css
.card {
    background-color: var(--nd-surface);
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-card);
    padding: 22px;
}

.nd-stat,
.hero-panel {
    background-color: var(--nd-surface);
    background-image: radial-gradient(circle, var(--nd-dot) 0.8px, transparent 1px);
    background-size: 10px 10px;
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-card);
}

.btn {
    display: inline-block;
    padding: 9px 18px;
    font-family: var(--nd-font-mono);
    font-size: 12px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border: 1px solid transparent;
    border-radius: var(--nd-radius-pill);
    cursor: pointer;
    transition: background-color 150ms var(--nd-ease), border-color 150ms var(--nd-ease), color 150ms var(--nd-ease), opacity 150ms var(--nd-ease);
    text-decoration: none;
}

.btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
}

.btn-primary {
    background-color: var(--nd-text-display);
    border-color: var(--nd-text-display);
    color: var(--nd-black);
}

.btn-primary:hover:not(:disabled) {
    background-color: var(--nd-text-primary);
    border-color: var(--nd-text-primary);
}

.btn-secondary,
.btn-save {
    background-color: transparent;
    border-color: var(--nd-border-visible);
    color: var(--nd-text-primary);
}

.btn-secondary:hover:not(:disabled),
.btn-save:hover:not(:disabled) {
    border-color: var(--nd-text-primary);
    background-color: transparent;
}

.btn-ghost {
    background: transparent;
    color: var(--nd-text-secondary);
    border: 1px solid transparent;
    padding: 6px 12px;
    border-radius: var(--nd-radius-pill);
}

.btn-ghost:hover:not(:disabled) {
    color: var(--nd-text-primary);
    border-color: var(--nd-border-visible);
    background: transparent;
}

.btn-delete,
.btn-danger {
    background-color: transparent;
    border-color: var(--nd-accent);
    color: var(--nd-accent);
}

.btn-delete:hover:not(:disabled),
.btn-danger:hover:not(:disabled) {
    background-color: var(--nd-accent-subtle);
}

.input {
    flex: 1;
    padding: 10px 14px;
    font-family: var(--nd-font-body);
    font-size: 13px;
    background-color: var(--nd-surface);
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-input);
    color: var(--nd-text-primary);
    outline: none;
    transition: border-color 150ms var(--nd-ease);
}

.input:focus {
    border-color: var(--nd-text-primary);
    box-shadow: none;
}

.status-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-pill);
    background: transparent;
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
}

.status-pass { color: var(--nd-signal); background: transparent; }
.status-fail { color: var(--nd-accent); background: transparent; }
.status-unknown { color: var(--nd-text-secondary); background: transparent; }

.health-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    background-color: var(--nd-surface);
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-card);
}

.health-summary {
    margin-top: 8px;
    padding: 12px 16px;
    border-radius: var(--nd-radius-card);
    border: 1px solid var(--nd-border);
    font-family: var(--nd-font-mono);
    font-size: 13px;
    letter-spacing: 0.06em;
    text-align: center;
}

/* Soft pill switch built on the existing checkbox, no markup change. */
.toggle-input {
    -webkit-appearance: none;
    appearance: none;
    position: relative;
    flex: none;
    width: 34px;
    height: 20px;
    margin: 0;
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-pill);
    background: var(--nd-surface);
    cursor: pointer;
    transition: border-color 150ms var(--nd-ease), background-color 150ms var(--nd-ease);
}

.toggle-input::before {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: var(--nd-radius-pill);
    background: var(--nd-text-disabled);
    transition: left 150ms var(--nd-ease), background-color 150ms var(--nd-ease);
}

.toggle-input:checked {
    border-color: var(--nd-signal);
}

.toggle-input:checked::before {
    left: 17px;
    background: var(--nd-signal);
}

.toggle-input:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.wizard-container {
    width: 600px;
    max-width: 90vw;
    background-color: var(--nd-surface);
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-card);
    display: flex;
    flex-direction: column;
    overflow: hidden;
}
```

In `select.input`, change the SVG arrow's fill from `%238892a4` to `%23999999`. That is the dropdown arrow drawn in secondary grey.

- [ ] **Step 2: Restyle the overlays and dropdowns in the component and page CSS**

- In `src/pages/profile_management.css` and `src/components/change_preview.css`, set the overlay backdrop to `background-color: rgba(0, 0, 0, 0.6);`. Remove any `backdrop-filter` and `box-shadow`. Give the dialog panel `border: 1px solid var(--nd-border); border-radius: var(--nd-radius-card); background: var(--nd-surface);`.
- In `src/components/searchable_select.css`, the trigger follows the `.input` look (10px radius, `--nd-border-visible`). The dropdown panel gets `border: 1px solid var(--nd-border-visible); border-radius: var(--nd-radius-input); background: var(--nd-surface); box-shadow: none;`. The highlighted option gets `background: var(--nd-surface-raised)`.
- Keep the overlays' `top/right/bottom/left` longhands exactly as they are: `layout.mjs` depends on them.

- [ ] **Step 3: Verify**

Run:
- `trunk build` → succeeds.
- `cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..` → all pass in both engines.

`app-flows.mjs` has one known expected failure from this redesign: the agent drawer's invalid-profile notice colour assertion (`rgb(215, 25, 33)`). That assertion is updated in Task 6. If it is the only failure, note it in the report and continue.

- [ ] **Step 4: Commit**

```bash
git add style/main.css src/pages/profile_management.css src/components/change_preview.css src/components/searchable_select.css
git commit -m "Restyle shared components in the soft Nothing style"
```

---

### Task 4: Replace hard-coded colours in page and component CSS

**Files:**
- Modify: every `src/pages/*.css` and `src/components/*.css` that contains colour literals, gradients or shadows, and `style/main.css` outside the token alias block.

**Interfaces:**
- Produces: no page or component CSS contains a hex, `rgb(`/`rgba(` or `hsl(` colour literal, `linear-gradient`/`radial-gradient`, `box-shadow` other than `none`, or `backdrop-filter`. There are three exceptions:
  - the overlay backdrop `rgba(0, 0, 0, 0.6)`;
  - the `.nd-stat`/`.hero-panel` dot-grid `radial-gradient`;
  - the `select.input` data-URI arrow.

- [ ] **Step 1: List the offenders**

```bash
grep -nE '#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(|linear-gradient|radial-gradient|box-shadow|backdrop-filter' src/pages/*.css src/components/*.css style/main.css | grep -v 'var(--' | grep -v 'box-shadow: none'
```

- [ ] **Step 2: Replace each with a token, by meaning**

| Literal is used as | Replace with |
|---|---|
| page or app background | `var(--nd-black)` |
| card, panel or input background | `var(--nd-surface)` |
| hover, selected or tinted background (green/red/amber tints included) | `var(--nd-surface-raised)` |
| divider or subtle border | `var(--nd-border)` |
| visible or input border | `var(--nd-border-visible)` |
| main text | `var(--nd-text-primary)` |
| headings or emphasized text | `var(--nd-text-display)` |
| secondary or muted text | `var(--nd-text-secondary)` |
| placeholder or disabled | `var(--nd-text-disabled)` |
| green, "good", success, active | `var(--nd-signal)` |
| red, error, danger, destructive | `var(--nd-accent)` |
| amber, yellow, warning | `var(--nd-warning)` |
| blue link | `var(--nd-interactive)` |
| any gradient used as a background | the flat token for its base colour |
| any `box-shadow` / `backdrop-filter` | delete the declaration (focus rings: `box-shadow: 0 0 0 1px var(--nd-text-primary)`) |

- Where a status tint was carried by a background (for example a green-tinted "success" box), move the meaning to the text: put the value colour on the text, and set the background to `var(--nd-surface-raised)`.
- Diff views are the only intentional exception: added lines stay tinted with `color-mix(in srgb, var(--nd-signal) 12%, transparent)` and removed lines with `color-mix(in srgb, var(--nd-accent) 12%, transparent)`, so diffs stay readable.
  - Before relying on `color-mix`, check `css-compat.mjs` output. `color-mix` needs Safari 16.2+.
  - If css-compat flags it, fall back to `var(--nd-surface-raised)` plus a 2px left border in the status colour.

- [ ] **Step 3: Verify**

Run:
- The Step 1 command → only the three allowed exceptions, plus any `color-mix` diff tints, remain.
- `trunk build` → succeeds.
- `cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node app-flows.mjs ../..` → pass, apart from the known Task 6 assertion.

- [ ] **Step 4: Commit**

```bash
git add src/pages/*.css src/components/*.css style/main.css
git commit -m "Replace hard-coded colours, gradients and shadows with tokens"
```

---

### Task 5: Rail sidebar with monoline icons

**Files:**
- Create: `src/components/icons.rs`
- Modify: `src/components/mod.rs` (add `pub mod icons;`)
- Modify: `src/components/sidebar.rs`
- Modify: `style/main.css`: `.app-layout`, `.sidebar*`, `.nav-*`, `.content`, `.sidebar-footer`, `.stl-*`

**Interfaces:**
- Produces:
  - `crate::components::icons::{Icon, IconKind}`, where `IconKind` has the variants `Home, Create, Analyze, Profiles, Batch, Compare, Settings, Health, About, Lock`, and `#[component] Icon(kind: IconKind)`.
  - `crate::components::sidebar::{NavItem, NAV_ITEMS, is_active}`.
- DOM contract for the Task 6 tests:
  - `nav.sidebar` (64px collapsed, 220px on hover or `:focus-within`)
  - `.nav-link` with an `.active` class and `aria-current="page"` on the current route
  - `.nav-icon`, `.nav-label` and `.nav-lock`
  - `.nav-update-dot`
  - `.content` with a fixed 64px left offset

- [ ] **Step 1: Write the failing tests**

Replace `src/components/sidebar.rs` with the tests module only for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_has_exactly_one_nav_item() {
        let hrefs: Vec<&str> = NAV_ITEMS.iter().map(|i| i.href).collect();
        assert_eq!(
            hrefs,
            vec!["/", "/filament", "/analysis", "/profiles", "/batch", "/compare", "/settings", "/health", "/about"]
        );
    }

    #[test]
    fn active_matching_is_exact_for_home_and_prefix_for_others() {
        assert!(is_active("/", "/"));
        assert!(!is_active("/profiles", "/"));
        assert!(is_active("/profiles", "/profiles"));
        assert!(is_active("/profiles/abc", "/profiles"));
        assert!(!is_active("/profilesx", "/profiles"));
    }
}
```

Run: `cargo test --bin bambumate sidebar`. Expected: compile errors, because `NAV_ITEMS` and `is_active` are not found.

- [ ] **Step 2: Create the icons**

Create `src/components/icons.rs`:

```rust
//! Monoline icons: 24×24, 1.5px stroke, round caps, drawn in currentColor.

use leptos::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Home,
    Create,
    Analyze,
    Profiles,
    Batch,
    Compare,
    Settings,
    Health,
    About,
    Lock,
}

#[component]
pub fn Icon(kind: IconKind) -> impl IntoView {
    let body = match kind {
        IconKind::Home => view! { <path d="M4 11l8-6 8 6v8a1 1 0 0 1-1 1h-4v-5h-6v5H5a1 1 0 0 1-1-1z"/> }.into_any(),
        IconKind::Create => view! { <circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="2.5"/><path d="M12 4v3M12 17v3"/> }.into_any(),
        IconKind::Analyze => view! { <rect x="3.5" y="6" width="17" height="13" rx="2"/><circle cx="12" cy="12.5" r="3.5"/><path d="M8 6l1.5-2h5L16 6"/> }.into_any(),
        IconKind::Profiles => view! { <path d="M5 6h14M5 12h14M5 18h9"/> }.into_any(),
        IconKind::Batch => view! { <rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/> }.into_any(),
        IconKind::Compare => view! { <path d="M8 4v16M16 4v16M4 8h4M16 16h4"/> }.into_any(),
        IconKind::Settings => view! { <circle cx="12" cy="12" r="3"/><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1"/> }.into_any(),
        IconKind::Health => view! { <path d="M3 12h4l2-5 4 10 2-5h6"/> }.into_any(),
        IconKind::About => view! { <circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8v.01"/> }.into_any(),
        IconKind::Lock => view! { <rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/> }.into_any(),
    };
    view! {
        <svg class="nd-icon" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor"
            stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {body}
        </svg>
    }
}
```

If Leptos's `view!` rejects bare SVG child elements outside an `<svg>` root, build each variant's children as a `String` of path markup and render it with `<svg … inner_html=markup>`. Record that as a deviation.

Add `pub mod icons;` to `src/components/mod.rs`.

- [ ] **Step 3: Implement the rail**

Replace `src/components/sidebar.rs` (keep the tests module from Step 1 at the bottom):

```rust
use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::app::{FeatureFlagsContext, UpdateContext};
use crate::components::icons::{Icon, IconKind};
use crate::components::stl_indicator::StlIndicator;

pub struct NavItem {
    pub href: &'static str,
    pub label: &'static str,
    pub icon: IconKind,
}

pub const NAV_ITEMS: &[NavItem] = &[
    NavItem { href: "/", label: "Home", icon: IconKind::Home },
    NavItem { href: "/filament", label: "Create Profile", icon: IconKind::Create },
    NavItem { href: "/analysis", label: "Print Analysis", icon: IconKind::Analyze },
    NavItem { href: "/profiles", label: "Profiles", icon: IconKind::Profiles },
    NavItem { href: "/batch", label: "Batch Generate", icon: IconKind::Batch },
    NavItem { href: "/compare", label: "Compare Profiles", icon: IconKind::Compare },
    NavItem { href: "/settings", label: "Settings", icon: IconKind::Settings },
    NavItem { href: "/health", label: "Health Check", icon: IconKind::Health },
    NavItem { href: "/about", label: "About", icon: IconKind::About },
];

/// Home matches only "/"; every other item also matches its sub-paths.
pub fn is_active(path: &str, href: &str) -> bool {
    if href == "/" {
        path == "/"
    } else {
        path == href || path.starts_with(&format!("{href}/"))
    }
}

#[component]
pub fn Sidebar() -> impl IntoView {
    let ff_ctx = use_context::<FeatureFlagsContext>().expect("FeatureFlagsContext not provided");
    let update_ctx = use_context::<UpdateContext>().expect("UpdateContext not provided");
    let pathname = use_location().pathname;

    let items = NAV_ITEMS
        .iter()
        .map(|item| {
            let (href, label, icon) = (item.href, item.label, item.icon);
            let active = move || is_active(&pathname.get(), href);
            let locked = move || href == "/analysis" && !ff_ctx.flags.get().analysis_enabled;
            let has_update =
                move || href == "/about" && update_ctx.update_info.get().map(|i| i.has_update).unwrap_or(false);
            view! {
                <li class="nav-item" class:nav-item-locked=locked>
                    <a href=href class="nav-link" class:active=active class:nav-link-locked=locked
                        aria-current=move || if active() { Some("page") } else { None }
                        title=move || if locked() { "Requires AI — enable in Settings".to_string() } else { label.to_string() }>
                        <span class="nav-icon"><Icon kind=icon /></span>
                        <span class="nav-label">{label}</span>
                        <Show when=locked>
                            <span class="nav-lock"><Icon kind=IconKind::Lock /></span>
                        </Show>
                        <Show when=has_update>
                            <span class="nav-update-dot" title="Update available"></span>
                        </Show>
                    </a>
                </li>
            }
        })
        .collect_view();

    view! {
        <nav class="sidebar" aria-label="Main">
            <div class="sidebar-header">
                <span class="rail-mark" aria-hidden="true"></span>
                <span class="sidebar-wordmark">"BAMBUMATE"</span>
            </div>
            <ul class="nav-list">{items}</ul>
            <div class="sidebar-footer">
                <StlIndicator />
            </div>
        </nav>
    }
}
```

The labels are always present in the DOM, so screen readers announce them in either state; no `aria-expanded` sync is needed. If `BrandMark` becomes unused only in this file, leave `branding.rs` alone: the About page still uses it.

- [ ] **Step 4: Rail CSS**

In `style/main.css`:
- Replace the rules for `.app-layout`, `.sidebar`, `.sidebar-header`, `.sidebar-brand`, `.sidebar-title`, `.sidebar-subtitle`, `.nav-list`, `.nav-item`, `.nav-link` (all states), `.nav-link.active`, `.content`, `.sidebar-footer`, `.nav-link-locked` and `.nav-lock-icon` with the block below.
- Delete `.sidebar-brand`, `.sidebar-title`, `.sidebar-subtitle` and `.nav-lock-icon` if nothing else uses them (check with grep).
- Keep `.content`'s existing padding and overflow declarations; only add the left offset.

```css
.app-layout {
    height: 100vh;
    width: 100vw;
}

.sidebar {
    position: fixed;
    top: 0;
    bottom: 0;
    left: 0;
    z-index: 45;
    width: 64px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    background-color: var(--nd-black);
    border-right: 1px solid var(--nd-border);
    -webkit-user-select: none;
    user-select: none;
    transition: width 200ms var(--nd-ease) 0ms;
}

.sidebar:hover {
    width: 220px;
    transition-delay: 150ms;
}

.sidebar:focus-within {
    width: 220px;
    transition-delay: 0ms;
}

.sidebar-header {
    height: 64px;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 20px;
    flex: none;
}

.rail-mark {
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    background-image: radial-gradient(circle, var(--nd-text-display) 1.2px, transparent 1.4px);
    background-size: 6px 6px;
}

.sidebar-wordmark,
.nav-label {
    white-space: nowrap;
    opacity: 0;
    transition: opacity 150ms var(--nd-ease);
}

.sidebar:hover .sidebar-wordmark,
.sidebar:hover .nav-label,
.sidebar:focus-within .sidebar-wordmark,
.sidebar:focus-within .nav-label {
    opacity: 1;
    transition-delay: 150ms;
}

.sidebar:focus-within .sidebar-wordmark,
.sidebar:focus-within .nav-label {
    transition-delay: 0ms;
}

.sidebar-wordmark {
    font-family: var(--nd-font-display);
    font-weight: 700;
    font-size: 18px;
    letter-spacing: -0.01em;
    color: var(--nd-text-display);
}

.nav-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 0;
    flex: 1;
}

.nav-item {
    position: relative;
}

.nav-link {
    position: relative;
    display: flex;
    align-items: center;
    gap: 14px;
    height: 44px;
    padding: 0 22px;
    color: var(--nd-text-secondary);
    text-decoration: none;
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    transition: color 150ms var(--nd-ease);
}

.nav-link:hover,
.nav-link:focus-visible {
    color: var(--nd-text-primary);
    outline: none;
}

.nav-link.active {
    color: var(--nd-text-display);
}

.nav-link.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 12px;
    width: 2px;
    height: 20px;
    background: var(--nd-signal);
}

.nav-icon {
    flex: none;
    display: inline-flex;
    width: 20px;
    height: 20px;
}

.nav-link-locked {
    color: var(--nd-text-disabled);
}

.nav-lock {
    display: inline-flex;
    width: 14px;
    height: 14px;
    margin-left: auto;
}

.nav-lock .nd-icon {
    width: 14px;
    height: 14px;
}

.nav-update-dot {
    position: absolute;
    left: 38px;
    top: 12px;
    width: 6px;
    height: 6px;
    border-radius: var(--nd-radius-pill);
    background: var(--nd-signal);
}

.content {
    margin-left: 64px;
    height: 100vh;
}

.sidebar-footer {
    flex: none;
    padding: 12px 14px 16px;
}
```

- Restyle the STL indicator in place so it fits the 64px rail:
  - `.stl-badge` becomes a 36×28px pill (`border: 1px solid var(--nd-border-visible)`, `border-radius: var(--nd-radius-pill)`) with centred Space Mono 10px text.
  - `.stl-badge-count` becomes a small `--nd-signal` count bubble on the top-right corner.
  - `.stl-dropdown` opens to the right of the rail: `position: fixed; left: 72px; bottom: 16px;` with `--nd-surface`, a 1px `--nd-border-visible` border, a 14px radius, no shadow, and `z-index: 46`.
- Remove the earlier `.content` rules' `margin-left`/`flex` layout declarations if any conflict. `.content` must not change width when the rail expands.

- [ ] **Step 5: Verify**

Run:
- `cargo test --bin bambumate sidebar` → 2 passed.
- `cargo fmt --check && cargo check --target wasm32-unknown-unknown` → clean.
- `trunk build` → succeeds.
- `cd tests/webkit && node app-flows.mjs ../..` → passes, apart from the known Task 6 assertion. The existing flow clicks `a[href="/filament"]` and similar links, which still exist.

- [ ] **Step 6: Commit**

```bash
git add src/components/icons.rs src/components/mod.rs src/components/sidebar.rs style/main.css
git commit -m "Replace the sidebar with a Nothing icon rail that expands on hover"
```

---

### Task 6: Tests for the rail, token-based colour assertions, CI

**Files:**
- Modify: `tests/webkit/app-flows.mjs`
- Modify: `.github/workflows/test.yml` (`webkit-ui` job)

**Interfaces:**
- Consumes: the Task 5 DOM contract and the Task 1 `contrast.mjs`.

- [ ] **Step 1: Make the drawer notice assertion token-based**

In `tests/webkit/app-flows.mjs`, the step "invalid-profile warning is shown in accent" compares against `rgb(215, 25, 33)`. The dark default now uses `#F0414A`. Replace the literal comparison with the computed token:

```js
    const { color, accent } = await page.locator(".ag-notice.ag-error").last().evaluate((el) => {
      const probe = document.createElement("span");
      probe.style.color = "var(--nd-accent)";
      el.parentElement.appendChild(probe);
      const accent = getComputedStyle(probe).color;
      probe.remove();
      return { color: getComputedStyle(el).color, accent };
    });
    if (color !== accent) throw new Error(`notice color ${color}, accent ${accent}`);
```

- [ ] **Step 2: Add rail steps**

Add these steps right after "startup queries the backend":

```js
  await step(run, page, "rail is collapsed to 64px with icons only", async () => {
    const w = await page.locator("nav.sidebar").evaluate((el) => el.getBoundingClientRect().width);
    if (Math.round(w) !== 64) throw new Error(`rail width ${w}`);
    const labelOpacity = await page.locator(".nav-label").first().evaluate((el) => getComputedStyle(el).opacity);
    if (labelOpacity !== "0") throw new Error(`label opacity ${labelOpacity}`);
  });

  await step(run, page, "hovering expands the rail over the content", async () => {
    const before = await page.locator(".content").evaluate((el) => el.getBoundingClientRect().left);
    await page.hover("nav.sidebar");
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219, null, { timeout: 3000 });
    const after = await page.locator(".content").evaluate((el) => el.getBoundingClientRect().left);
    if (before !== after) throw new Error(`content moved from ${before} to ${after}`);
    await page.mouse.move(900, 400);
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width <= 65, null, { timeout: 3000 });
  });

  await step(run, page, "keyboard focus expands the rail", async () => {
    await page.focus('nav.sidebar a[href="/profiles"]');
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219, null, { timeout: 3000 });
    await page.evaluate(() => document.activeElement.blur());
  });

  await step(run, page, "active route shows the signal tick", async () => {
    const current = await page.locator('nav.sidebar a[aria-current="page"]').getAttribute("href");
    if (current !== "/") throw new Error(`aria-current on ${current}`);
    const tick = await page.locator('nav.sidebar a[aria-current="page"]').evaluate((el) => getComputedStyle(el, "::before").backgroundColor);
    if (!tick || tick === "rgba(0, 0, 0, 0)") throw new Error(`no tick colour (${tick})`);
  });
```

- [ ] **Step 3: Run the contrast test in CI**

In `.github/workflows/test.yml`, in the `webkit-ui` job, add this directly after the "Overlay geometry" step:

```yaml
      - name: Token contrast
        working-directory: tests/webkit
        run: node contrast.mjs ../..
```

- [ ] **Step 4: Verify**

Run:
- `trunk build && cd tests/webkit && node app-flows.mjs ../.. && node contrast.mjs ../.. && node layout.mjs ../.. && node css-compat.mjs ../..` → all pass in both engines, with 0 pageerror.
- `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/test.yml'))"` → no error.

- [ ] **Step 5: Commit**

```bash
git add tests/webkit/app-flows.mjs .github/workflows/test.yml
git commit -m "Test the rail and token colours; run the contrast check in CI"
```

---

### Task 7: Screenshot review of every route, both themes

**Files:**
- Create: `tests/webkit/screens.mjs`
- Modify: `.gitignore` (add `tests/webkit/screens/`)
- Modify: whichever CSS files the review shows need fixing

**Interfaces:**
- Produces: `node tests/webkit/screens.mjs <repoRoot>`, which writes `tests/webkit/screens/<theme>-<width>-<route>.png` for 9 routes × 2 themes × 2 widths.

- [ ] **Step 1: Write the screenshot script**

Create `tests/webkit/screens.mjs`. Reuse `serve()`, `installTauriMock` and `FIXTURES` from the patterns in `app-flows.mjs`. The simplest way is to export them from `app-flows.mjs` behind an `if (import.meta.url === …)` main guard. If that is awkward, copy the ~40 lines, with a comment pointing at the original.

For each theme in `["dark", "bambu"]`, width in `[1280, 900]` (height 860), and route in `["/", "/filament", "/analysis", "/profiles", "/batch", "/compare", "/settings", "/health", "/about"]`:
1. Launch WebKit with that viewport.
2. Add the mock, and add an init script that makes `get_preference("theme")` return the theme. Do this by setting `FIXTURES.get_preference = theme` for that run: the app reads the stored theme through `get_preference`, so this is enough.
3. `goto(baseUrl)`, wait for `.sidebar`, click the route's nav link, wait 600 ms, then `screenshot({ path, fullPage: true })`.

- [ ] **Step 2: Review and fix**

Run `trunk build && cd tests/webkit && node screens.mjs ../..`, then open every PNG (Read them). Fix, in CSS only, anything that:
- makes text unreadable;
- leaves a leftover bright or green legacy background, a gradient or a shadow;
- lets the rail overlap page content at rest;
- clips a component at 900 px width;
- breaks the Nothing rules in the Global Constraints.

Page layouts that merely look "old" are out of scope (piece 2). Record each fix with its before and after screenshot names in the report.

- [ ] **Step 3: Verify and commit**

Run: `cd tests/webkit && node app-flows.mjs ../.. && node contrast.mjs ../.. && node layout.mjs ../.. && node css-compat.mjs ../..` → all pass.

```bash
git add tests/webkit/screens.mjs .gitignore src style
git commit -m "Add a route screenshot script and fix issues found in the visual review"
```
