# Page Redesign: Cockpit Pages and Seven Hubs

**Status:** Approved in brainstorming (2026-10-17). Written spec pending the user's review.
**Sub-project:** Redesign, piece 2 of 3:
1. design system and shell (PR #25, merged);
2. page-by-page redesigns (this piece);
3. 3D dot-matrix viewport.

**Branch:** `claude/page-redesign`, off `main` at d88e0d0 (v0.3.0). Each wave gets its own PR.

## Goal

Rebuild every page in the Nothing design language as a cockpit. Each page leads with the one thing that matters right now, shown large, and a page may change *what* it shows, not just how it looks. At the same time, cut the navigation rail from 11 items to 7 hubs.

## Decisions made in brainstorming

| Topic | Decision |
|---|---|
| Depth | **C: rethink as a cockpit.** Pages may change their content, not only their layout. |
| Navigation | **B: 7 hubs.** Pages that do the same job merge into one hub with tabs. Old routes redirect. |
| Home | One hero that adapts to four states, in this order of precedence: needs attention, printing, idle, no printer. Below it: AMS (when connected), then a footer strip with recent work and preset sync status. |
| Other hubs | Each gets one "big thing" as its hero (section 4). All six sketches were approved as shown. |
| Delivery | Shared patterns first, then three waves, one PR each (section 6). |

## Constraints

- **Read-only printer.** Nothing in this piece sends a command to the printer. "Open printer" navigates; "resume on the printer" is advice text.
- **Mostly frontend.** There are two backend changes, both small:
  - the agent's `bm_navigate` route list;
  - one read-only command, `latest_generated_preset`, which returns the newest row of the `generated_presets` ledger for Home's "Recent" line. The frontend has no other way to know which preset BambuMate made last: `list_profiles` carries no dates.

  "Last print" is recorded in the frontend.
- **Both themes are first-class.** Every new text/background pair passes `contrast.mjs` in dark and light: 4.5:1 for text, 3:1 for disabled text.
- **Tokens only.** New CSS uses the `--nd-*` tokens from `style/tokens.css`. No new colours are introduced.
- **Font budget per screen.** At most two families (Space Grotesk and Space Mono), plus Doto for the single hero. At most three sizes and two weights.
- **Existing behaviour is kept.** Every feature on today's 11 pages is still reachable after the redesign. Nothing is removed.

## Design

### 1. Navigation: seven hubs

The rail (`src/components/sidebar.rs`, `NAV_ITEMS`):

| Hub | Route | Tabs (route) | Replaces |
|---|---|---|---|
| Home | `/` | none | Home |
| Printer | `/printer` | none | Printer |
| Filament | `/filament` | Search (`/filament`), Batch (`/filament/batch`) | Filament Search, Batch Generate |
| Profiles | `/profiles` | Library (`/profiles`), Compare (`/profiles/compare`) | Profile Management, Profile Comparison |
| Slice | `/slice` | none | Slice |
| Analyze | `/analysis` | none | Print Analysis |
| Settings | `/settings` | General (`/settings`), Health (`/settings/health`), About (`/settings/about`) | Settings, Health Check, About |

- Settings is pinned to the bottom of the rail, below a divider. This is new; today the rail is one flat list.
- The Printer connection dot and the STL indicator stay where they are.
- **Redirects.** `/batch` goes to `/filament/batch`, `/compare` to `/profiles/compare`, `/health` to `/settings/health`, and `/about` to `/settings/about`. Each uses `leptos_router`'s `Redirect` with `replace: true`, so the old route never stays in history. Deep links and the agent's old routes keep working.
- **Feature flags.** When `analysis_enabled` is off, the Analyze hub shows locked with "Requires AI", as Print Analysis does today.
- **Agent.** `bm_navigate` in `src-tauri/src/agent/tools/app.rs` adds the new tab routes to its allowlist and keeps the old ones, so saved prompts don't break. Its description lists the new routes. Tool counts stay at 22 agent, 22 Codex and 23 MCP.

### 2. Shared patterns

These are new Leptos components in `src/components/`, styled once in `style/main.css`. Pages build from them and do not restyle them locally.

- **`PageHeader`** takes `title`, an optional `tabs: &[(label, route)]` and an optional trailing slot for actions.
  - It renders the page's single `<h1>`, which fixes the deferred "no h1" item.
  - Tabs are pill links (`aria-current="page"` on the active one) in a row under the title.
- **`Hero`** takes `label` (mono caps), `value` (a Doto number or word, or any child view such as the search box), an optional `unit` and an optional `detail` line.
  - It adopts the existing `.nd-display` and `.nd-stat`, which fixes the deferred Doto item.
  - Each page has at most one `Hero`.
  - It accepts a `tone`: neutral, signal, warning or accent. The tone colours only the value.
- **`SegmentedBar`** takes `filled: u32` and `total: u32` and renders discrete segments.
  - A pure helper, `segments_filled(percent, total)`, rounds to the nearest segment. It shows 0 only at 0% and `total` only at 100%.
  - This fixes the deferred segmented-progress item.
- **`StatRow`** takes `label` and `value`. The label is secondary text on the left; the value is mono and right-aligned.
- **Tables.** A `.nd-table` style: mono caps header in `--nd-text-secondary`, a single bottom border on the header, `font-variant-numeric: tabular-nums`, numeric columns right-aligned, no zebra striping. This fixes the deferred table-header item.
- **`StateText`** takes `kind` (Loading, Empty or Error) and an optional message. It renders `[LOADING…]`, `[EMPTY]` or `[ERROR: …]` inline in mono. It replaces ad-hoc spinners and multi-paragraph empty states.
- **Buttons.** Every page button is a pill (`--nd-radius-pill`), including the Profiles header `.btn-icon` buttons. This fixes the deferred pills item.

### 3. Home cockpit

Home (`src/pages/home.rs`) is rebuilt around a pure `HomeState` derived from contexts the app already provides (`PrinterShared`, `SlicerShared`) and two data calls (`list_profiles`, `list_unsynced_presets`).

**States, in order of precedence:**

1. **Attention.** The printer is connected and paused or failed, reports a print error, or reports an HMS code while not printing. An HMS code during a print doesn't hide the print: the Printing hero stays and its label names the code. BambuMate can't tell an HMS code's severity.
   - Hero: the state word (e.g. `PAUSED`) in accent, with the HMS code in the label and the cached HMS text as the detail line.
   - The slot in use (the printer's `tray_now`) is outlined in accent. An HMS code does not name a slot, so the slot feeding the print stands in for the affected one; in a runout pause they are the same slot.
   - Actions: "Open printer" and "What does this mean?". The second opens the agent drawer with a question about the HMS code and text already typed in the composer. The user still presses send.
2. **Printing.**
   - Hero: the percentage in Doto.
   - Detail: time left, layer x/y, and nozzle and bed temperatures.
   - A `SegmentedBar` sits under the hero.
3. **Idle.**
   - Hero: `READY`.
   - Detail: the last print's name, how long ago it finished and its duration, shown only when known.
   - Actions: "Slice a model", "New filament profile" and "Analyze last print". The last one appears only when analysis is enabled.
4. **No printer.** No printer is configured, or the configured printer is disconnected.
   - Hero: the count of user filament presets, with the detail "N presets not syncing" or "No sync problems found".
   - Actions: "New filament profile" and "Slice a model", plus a small "Connect →" link to Settings → Printer.
   - When a printer is configured but disconnected, the label reads `[DISCONNECTED]`. A stale percentage is never shown.

**Below the hero:**
- The AMS slot row. It appears whenever a printer is connected and reuses the printer page's slot data and its "Assign" affordance.
- A footer strip: the most recent generated profile and how long ago it was made, on the left; on the right, "No sync problems found" or "N presets not syncing", the latter linking to Settings → Health. BambuMate can only see presets stuck out of sync, not confirm that the others reached the cloud.

**Data rules:**
- `list_profiles`, `list_unsynced_presets` and `latest_generated_preset` run when Home mounts and again when the agent changes profiles (`AgentRefresh`). Repair happens on Settings → Health, so returning to Home re-reads. None of them runs on a timer.
- While printing, the slot in use is outlined in signal.
- "Last print" is recorded in `PrinterShared` when the printer's job state changes from running to finished. It holds the job name, finish time and duration, and lives for the session only.
- A failed `list_profiles` or `list_unsynced_presets` call shows `StateText::Error` in its own strip. The hero and the rest of the page still render.

### 4. Other hubs

| Hub | Hero | Changes |
|---|---|---|
| **Printer** | The print percentage (or the state word when idle) at full size, with nozzle, bed and chamber temperatures as stat rows. | Touch-ups: adopt `PageHeader`, `Hero` and `SegmentedBar`, and add an HMS history list. Behaviour is unchanged. |
| **Filament · Search** | The search box until a filament is chosen, then that filament's nozzle temperature, bed temperature and flow ratio as three Doto stats. | The long editor (base profiles, compare and merge, nozzle-specific adjustments) moves into collapsible sections below the hero, closed by default except the one the user last opened. Generate stays one click from the hero. |
| **Filament · Batch** | The queue's progress: done out of total, in Doto, with a `SegmentedBar`. | The queue is a `.nd-table` with a status column. |
| **Profiles · Library** | The selected profile's nozzle temperature, with flow, pressure advance and max volumetric speed as stat rows. | List on the left with a sync dot per row (signal = synced, warning = unsynced); detail on the right. Header actions become pills. |
| **Profiles · Compare** | The count of differing settings, e.g. `14 DIFFERENCES`. | Two-column `.nd-table`. Only changed values are lit (`--nd-text-display`); unchanged rows are `--nd-text-disabled` and can be hidden with a "Changed only" toggle, which is on by default. |
| **Slice** | Before slicing: the drop zone. After: print time and filament grams as two Doto stats. | Touch-ups: adopt `PageHeader`, `Hero`, `StatRow` and `.nd-table`. Behaviour is unchanged. |
| **Analyze** | The quality score in Doto, next to the photo with numbered defect pins. | The defect list is ordered by severity. Pins and list numbers match. The 3D viewport (piece 3) later replaces the photo. |
| **Settings · General** | The Health summary: `ALL GOOD` in signal, or `N ISSUES` in warning or accent, with "N checks · last run …". | Sections become label/value rows (`StatRow` and inputs) under mono section labels instead of boxed cards. Printer and slicer settings stay as they are, restyled. |
| **Settings · Health** | The same summary as the hero. | Today's Health page content, including the preset-sync repair panel, under the shared header. |
| **Settings · About** | The app name in Doto. | Version, links and licences as stat rows. |

**Severity colours** (Analyze, and anywhere `.severity-*` is used). The four tiers become distinct in both colour and form. This fixes the deferred "top two tiers share one colour" item.

| Tier | Treatment |
|---|---|
| Severe | Filled: `--nd-accent` background, `--nd-black` text. |
| Significant | `--nd-accent` text, no fill. |
| Noticeable | `--nd-warning` text. |
| Minor | `--nd-text-secondary` text. Blue is reserved for links. |

A pure `severity_tone(Severity) -> Tone` helper holds this mapping, so tests can assert that no two tiers render the same.

**STL indicator** (rail). The text badge becomes an icon with a count. It is a `<button>` with an `aria-label` ("3 STL files waiting"), and its dropdown opens and closes from the keyboard (Enter/Space opens it; Escape closes it and returns focus). This fixes the deferred STL badge item.

### 5. Setup wizard

The first-run wizard (`src/pages/setup_wizard.rs`) becomes a full-screen stepped flow:
- a step counter in mono (`02 / 05`) and a `SegmentedBar` for progress;
- one question per screen, the question itself as the hero;
- Back and Continue as pills.

Its steps, logic and stored values are unchanged.

### 6. Delivery in three waves

Each wave is its own PR off `main`, merged before the next one starts. Every wave must pass the full test suite (section 8).

1. **Wave 1: foundation, navigation and Home.**
   - The section 2 patterns.
   - The seven-hub rail with tab routes and redirects. Existing pages are mounted under their new routes unchanged.
   - The `bm_navigate` routes.
   - The STL indicator.
   - The Home cockpit with "last print" recording.
2. **Wave 2: Filament and Profiles hubs.** Search, Batch, Library and Compare.
3. **Wave 3: everything else.** Analyze with the severity colours, the Settings hub (General, Health and About), the setup wizard, and the Printer and Slice touch-ups.

## Error handling

- A failed data call shows `StateText::Error` with the message, inline, in the section that needed it. Pages never blank out, and there are no toasts.
- A printer disconnect while printing switches Home to the no-printer state with `[DISCONNECTED]`, and Printer shows the same label. Neither keeps showing the last percentage.
- An unknown route still shows the router fallback, restyled with `StateText::Empty` and a link to Home.

## Testing

**Rust unit tests** (`cargo test --bin bambumate`, already in CI):
- `HomeState` precedence: attention beats printing, printing beats idle, idle beats no printer, and a disconnect mid-print gives no-printer with the disconnected flag.
- "Last print" is recorded on the running-to-finished transition and not on other transitions.
- Route table: every old route maps to its hub tab, and every hub and tab route resolves.
- `segments_filled`: 0% gives 0, 100% gives the total, 1% gives at least 1, 99% gives at most total − 1, and rounding is correct in between.
- `severity_tone`: all four tiers are distinct.

**Backend** (`cargo test --manifest-path src-tauri/Cargo.toml`): `bm_navigate` accepts every new route and every old route, and rejects an unknown one.

**WebKit and Chromium** (`tests/webkit/`):
- `screens.mjs` (run by hand, not in CI): `ROUTES` lists the hub and tab routes, and every route is captured in both themes.
- `layout.mjs`: the rail has 7 items, and the rail-overlap check runs on every hub tab.
- `app-flows.mjs`:
  - Home with printer fixtures for all four states shows the right hero.
  - Each old route lands on its tab, and Back skips the old route.
  - Switching tabs updates the URL, and Back returns to the previous tab.
  - A failing data call renders `[ERROR: …]` while the rest of the page renders.
  - The STL indicator opens and closes from the keyboard.
- `contrast.mjs`: adds the new pairs. These are Doto display text on page and surface, the four severity treatments, and signal, warning and accent used as text on surface, all in both themes.

**Manual:** Home against the user's real H2 while printing, idle and paused. The fixtures are still community MQTT samples, not real H2 captures.

## Out of scope

- The 3D viewport (piece 3).
- New features: the filament advisor, calibration mode, camera, print journal.
- Printer control of any kind.
- Persisting "last print" across app restarts.
- Restructuring backend commands or data models.

## Risks

- **Filament Search is the largest page** (1,167 lines of Rust and 1,079 of CSS). Moving its editor into collapsible sections touches a lot of markup.
  - *Mitigation:* it is the bulk of wave 2. The existing flows in `app-flows.mjs` guard search, generate and merge.
- **Redirects and the agent.** If any code path writes an old route into history, Back could bounce between the old route and its redirect.
  - *Mitigation:* redirects replace the history entry, and a test checks that Back works after following one.
- **Fixture realism.** The Home states are tested against community MQTT samples.
  - *Mitigation:* manual acceptance on the real H2, listed above.
