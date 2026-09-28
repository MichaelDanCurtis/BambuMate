# Nothing Design System and App Shell

**Status:** Approved in brainstorming. Written spec pending the user's review.
**Date:** 2026-09-27
**Sub-project:** Redesign, piece 1 of 3:
1. design system and shell (this piece)
2. page-by-page redesigns
3. 3D dot-matrix viewport

**Branch:** `claude/nothing-design-system`, stacked on `claude/repo-review-updates-f456df` (PR #23), because it builds on the agent drawer's Nothing tokens.

## Goal

Give the whole app the Nothing design language in one step. It should be dark by default, use Bambu green as the signal colour, have soft flat components, and use a rail sidebar that expands on hover. No page's layout or code gets restructured.

The drawer's `.nd` tokens become the app-wide foundation. Pages keep their current layouts until piece 2 recomposes them.

## Decisions made in brainstorming

| Topic | Decision |
|---|---|
| Direction | **C: dark, with Bambu green as the signal colour.** Green marks active and good states, red marks problems, and amber marks warnings. |
| Default mode | Dark for new installs. Light stays available. An existing saved theme is kept. |
| Navigation | Icon rail by default. It expands into the full labeled sidebar on hover or keyboard focus, as an overlay. |
| Components | **Soft:** pill buttons, toggles and segments; round cards; a faint dot-grid texture on stat and hero cards. |
| Migration | **Option 1: remap.** Promote the tokens to `:root`, point legacy variables at them, and restyle the shared components. Every page picks up the look on day one. |

## 1. Tokens and themes

Tokens live in `style/tokens.css`. They move from the `.nd` scope to `:root`. The `--nd-*` names stay the source of truth; the drawer keeps working, and `.nd` becomes a harmless no-op scope.

| Token | Dark (default) | Light |
|---|---|---|
| `--nd-black` (page) | `#000000` | `#F5F5F5` |
| `--nd-surface` | `#111111` | `#FFFFFF` |
| `--nd-surface-raised` | `#1A1A1A` | `#F0F0F0` |
| `--nd-border` | `#222222` | `#E8E8E8` |
| `--nd-border-visible` | `#333333` | `#CCCCCC` |
| `--nd-text-disabled` | `#666666` | `#8A8A8A` |
| `--nd-text-secondary` | `#999999` | `#666666` |
| `--nd-text-primary` | `#E8E8E8` | `#1A1A1A` |
| `--nd-text-display` | `#FFFFFF` | `#000000` |
| `--nd-signal` (new: active, good, on) | `#00AE42` | `#007A34` |
| `--nd-accent` / error | `#F0414A` | `#D71921` |
| `--nd-warning` | `#D4A843` | `#7D5E0E` |
| `--nd-success` | = `--nd-signal` | = `--nd-signal` |
| `--nd-interactive` (links) | `#5B9BF6` | `#007AFF` |

Every text token was checked: at least 4.5:1 on page and surface (3:1 for disabled). Dark red is lightened to `#F0414A` (`#D71921` on black is 4.05:1). Light green, amber and disabled grey are darkened.

The **theme attribute** stays `data-theme` on `<html>` (`src/theme.rs`):
- `dark` selects the dark set;
- `bambu` (the legacy stored value) and `light` select the light set.

Settings shows **Dark / Light**. Only a missing stored value defaults to `dark`.

In `style/main.css`, **every legacy variable is remapped** to a token:

| Legacy | Maps to |
|---|---|
| `--bg-primary`, `--bg-sidebar` | `--nd-black` |
| `--bg-card`, `--bg-input`, `--bg-tertiary` | `--nd-surface` |
| `--bg-secondary`, `--bg-hover`, `--bg-active` | `--nd-surface-raised` |
| `--bg-active-hover` | `--nd-border` |
| `--border-primary` | `--nd-border-visible` |
| `--text-primary`, `--text-bright` | `--nd-text-primary` / `--nd-text-display` |
| `--text-secondary`, `--text-muted`, `--text-placeholder` | `--nd-text-secondary` / `--nd-text-disabled` |
| `--accent`, `--accent-hover` | `--nd-text-display` (primary action = white pill) |
| `--text-on-accent` | `--nd-black` |
| `--color-success`, `--success-text` | `--nd-signal` |
| `--color-danger`, `--error-text`, `--border-danger` | `--nd-accent` |
| `--color-warning` | `--nd-warning` |
| `--bg-success`, `--bg-danger`, `--error-bg`, `--bg-unknown` | `--nd-surface-raised` (colour goes on the value, not the background) |
| `--border-success`, `--error-border`, `--border-unknown` | `--nd-border-visible` |
| `--shadow-*` (all) | `none` |
| `--shadow-focus*` | `0 0 0 1px var(--nd-text-primary)` (a hairline focus ring, no glow) |

The 69 hard-coded colour literals in `src/pages/*.css` and `src/components/*.css` are replaced with tokens. Any gradients or shadows in page CSS are removed.

## 2. Typography

- **Body:** Space Grotesk, 300/400/500.
- **Labels:** Space Mono in ALL CAPS, 11px, 0.08em tracking.
- **Numbers and data values:** Space Mono.
- **Hero numbers only:** Doto (dot-matrix), 36px and up. It is vendored as `style/fonts/Doto-Variable.ttf` from google/fonts (OFL), alongside the existing Space Grotesk and Space Mono.
- The `body` font-family becomes Space Grotesk app-wide.
- Type scale: display 48–72, heading 24, subheading 18, body 16, body-sm 14, caption 12, label 11 (px). Page CSS keeps its current sizes unless a component class sets them. Scale adoption per page is piece 2.

## 3. Components (soft)

Shared component styles are restyled in place in `style/main.css` (and the component CSS files). Class names do not change.

| Component | Style |
|---|---|
| Buttons | Pill (999px). **Primary:** `--nd-text-display` fill with `--nd-black` text. **Secondary:** transparent with a `--nd-border-visible` outline. **Danger:** transparent, red text and outline. All three use Space Mono caps. Disabled is 40% opacity. |
| Inputs, selects, textareas, searchable select | 10px radius, `--nd-surface` fill, `--nd-border-visible` border. On focus the border becomes `--nd-text-primary`, with no glow. |
| Toggles and checkboxes styled as switches | Pill track. The knob or fill is `--nd-signal` when on. |
| Segmented controls and tabs | Pill group. The selected segment has a `--nd-text-display` fill. |
| Cards | 14px radius, `--nd-surface`, a 1px `--nd-border`, no shadow. **Stat and hero cards** get the dot-grid texture. List and form cards stay plain. |
| Tables | No zebra stripes. Rows are divided by hairline `--nd-border` lines. Headers use the label style. Numbers are right-aligned in mono. Status colour applies to the value only. |
| Badges and status badges | Mono caps pills with an outline. Status is shown by the text colour. |
| Progress | Segmented bars (discrete pill segments). |
| Modals and overlays | Flat `--nd-surface`, a 1px border, 14px radius. The backdrop is a translucent `--nd-black`, with no blur. |
| Notices | Inline status text, no toast styling. |
| Emoji in UI | Removed. The sidebar 🔒 and ✨ become line icons or a green dot (see Shell). No other emoji is used as UI. |
| Motion | 150–250ms `cubic-bezier(0.25,0.1,0.25,1)`, opacity and colour only, no bounce or scale. |

## 4. App shell

The sidebar is rebuilt in `src/components/sidebar.rs` and its CSS.

**Collapsed rail:**
- 64px wide, fixed to the left.
- Shows the dot-matrix logo mark, then one monoline icon per route: Home, Create Profile, Print Analysis, Profiles, Batch Generate, Compare Profiles, Settings, Health Check, About.
- Icons are inline SVG, 24×24 viewBox, 1.5px stroke, round caps, drawn in `currentColor`.

**Expanded sidebar:**
- 220px wide. It overlays the page content; it does not push it.
- Shows the "BAMBUMATE" wordmark in Doto and Space Mono caps labels beside the icons.
- It opens after a 150ms hover delay, or immediately on `:focus-within` for keyboard users. It closes when the pointer leaves or focus moves out.
- It is implemented in CSS only (`:hover` with a transition delay, and `:focus-within`). The labels are always in the DOM, so assistive tech reads them in either state and no `aria-expanded` sync is needed.

**States:**
- The active route shows the icon in `--nd-text-display` and a 2px `--nd-signal` tick on the rail edge.
- Print Analysis, when AI is off, shows a dimmed icon, a line-drawn lock, and the tooltip "Requires AI — enable in Settings".
- An available update shows an `--nd-signal` dot on the About icon.
- The STL indicator (the existing sidebar footer) moves to the rail bottom as an icon with a count dot.

**Layout:**
- `.content` gets a 64px left offset.
- The agent toggle pill stays top-right.
- The existing `.sidebar`, `.nav-link` and `.nav-list` classes remain so existing tests and selectors keep working. New elements are added for icons and labels.

## 5. Testing

**Existing suites must pass,** with updates only where the markup changed:
- `cargo test --bin bambumate`
- `cargo check --target wasm32-unknown-unknown`
- `cargo fmt --check`
- `trunk build`
- WebKit `css-compat.mjs`, `layout.mjs` and `app-flows.mjs`

**New flow steps in `app-flows.mjs`:**
- the rail is 64px wide;
- hovering widens it to 220px with labels visible, and the page content does not move;
- Tab focus into the rail expands it;
- the active route shows the signal tick.

**New contrast test:** `tests/webkit/contrast.mjs` loads `style/tokens.css` in WebKit and Chromium, switches `data-theme`, and computes the WCAG contrast of each text token against the page and surface backgrounds:
- ≥ 4.5:1 for primary and secondary text, signal, warning and error;
- ≥ 3:1 for disabled text.
It runs in the CI `webkit-ui` job next to the other WebKit suites.

**Visual review:** screenshots of every route in both themes at 1280×860 and 900×800, checked by eye before completion.

## Out of scope

- **Piece 2:** recomposing page layouts (hero element per page, asymmetry, the type scale per page), and the setup wizard's layout.
- **Piece 3:** the 3D viewport.
- Any new feature, including the filament advisor.

## Risks

| Risk | Mitigation |
|---|---|
| Page CSS that relies on legacy colours meaning something (e.g. a green background as "success") loses meaning when backgrounds go neutral. | The colour moves to the value text, and the visual review checks each page. |
| Dark by default may surprise existing users. | Only a missing stored value defaults to dark; an existing choice is kept. |
| The WebKit layout test checks overlay geometry, and the rail is a new overlay. | Add the rail to the layout checks. |
