# Filament Advisor: What to Print It With

**Status:** Designed 2026-10-23, section by section, with the user's approval at each step.
**Sub-project:** Phase 3 (print intelligence), first feature. The redesign (phase 2) was finished with PR #33.
**Branch:** `claude/filament-advisor`, off `main` at b45dafc.

## Goal

Answer one question before printing: "what should I print this with?" You describe what the part is for, optionally load the model, and BambuMate recommends:
- a material;
- the specific products for it, with the ones already in your AMS first.

Every pick comes with reasons, and from there it's one click to a generated profile.

## Decisions

| Topic | Decision |
|---|---|
| Engine | **Rules plus optional AI.** A curated material table scores materials against what the part needs. It works offline, and every score shows its reasons. When AI is configured, it turns a free-text description into those needs. Without AI, you pick the needs yourself from chips. |
| Buy links | **Plain links first.** Each product links to its manufacturer or SpoolScout page, with no affiliate tags. Every link goes through one function, `buy_link`, so affiliate tagging through a hosted redirect can be added later without touching the UI. |
| Placement | **A Filament hub tab plus a link from Slice.** A new "Advise" tab is added (Advise · Search · Batch). Slice gets a "Suggest filament" link that opens Advise with the model already loaded. |
| Product order | **AMS-loaded, then preferred brands, then the rest A–Z.** The catalog has no prices or ratings, so products are never ranked on quality. The page states the ordering rule. |

## Constraints

- **Honest ranking.** Products are never ranked by commission or by any claim the data can't support. A material is never claimed "food safe": the food-contact chip only adds a caution.
- **Works offline.** Without AI or a printer, scoring still runs. AI only fills in the chips.
- **No new runtime dependencies.** The AI call reuses the existing provider dispatch in `scraper/extraction.rs`.
- **Nothing is installed or written** until you click "Generate profile". That then runs the existing generate and install flow.
- **Affiliate rule, for later.** Once affiliate tagging is added, links must go through a redirect the user controls, and the page must show a visible disclosure. `buy_link`'s doc comment says so.
- **Design.** Nothing tokens and the shared patterns (`PageHeader`, `Hero`, `HeroStats`, `StatRow`, `StateText`) apply, in both themes.

## Design

### 1. Recommendation (`src-tauri/src/advisor/`)

**Needs.** Each need is a chip:
- strong;
- stiff;
- flexible;
- impact-tough;
- heat ≥60 °C, heat ≥80 °C or heat ≥100 °C, as one choice;
- outdoor/UV;
- chemical/fuel;
- food-contact-adjacent, which adds a caution and never a claim;
- easy print;
- fine detail;
- low cost.

**Material table** (`materials.rs`, pure data):
- **Materials covered:** PLA, PLA+, PLA-CF, PETG, PET-CF, ABS, ASA, TPU 95A, PA, PA-CF and PC.
- **Per material:**
  - a score of 0–3 for each property;
  - a heat deflection temperature;
  - whether it needs an enclosure;
  - whether it is abrasive, meaning it needs a hardened nozzle;
  - whether it needs drying;
  - how prone it is to warping, from 0 to 3.
- **The base `MaterialType`** each material maps to, for profile generation.

**Hard constraints from the printer:**
- **Enclosure.** Whether the printer is enclosed comes from its model. H2C, H2D, H2S, X1, X1C, X1E, P1S and P2S are enclosed; A1, A1 mini and P1P are open. An unknown model is assumed enclosed, and the page says so.
- **Hardened nozzle.** When connected, this comes from the nozzle type. When unknown, abrasive materials get a "needs a hardened nozzle" note instead of being filtered out.
- **Failing a constraint.** A material that fails a hard constraint goes into "Not on this printer" with its reason. It is never dropped silently.

**Geometry**, when a model is loaded, from `model_view`:

| Geometry | Effect |
|---|---|
| Footprint over 150 mm on either side, or a small-bed-contact risk | Lowers the score of materials prone to warping |
| Tall-and-thin risk | Favours stiff materials |
| Overhang risk | Adds the note "Supports likely, whatever the material" |

**Scoring** (`score.rs`): `score(needs, printer, geometry) -> Vec<MaterialFit>`.
- It is deterministic, and ties break in a fixed material order.
- Each fit carries the total, a per-need reason (met, partly or not met), the printer notes and the geometry notes.
- Results are the top 3 materials plus the "Not on this printer" list.

**Interpretation** (`interpret.rs`):
- `interpret(text) -> Option<Needs>`, via a generic `complete_json(prompt, schema)` helper pulled out of `scraper/extraction.rs`. Only the existing call path is refactored.
- The output is validated against the fixed list of needs, and unknown entries are dropped.
- It returns `None` when AI is off.

**Commands:**
- `advise_interpret(text) -> Needs`.
- `advise(needs, model_path: Option<String>) -> Advice`. It gathers:
  - the printer model from settings;
  - the nozzle and the AMS trays from the live printer state, when connected;
  - geometry from `model_view`, which is cached;
  - products from the catalog, filtered by material.
- Getting and setting `preferred_brands`, stored in `preferences.json`.

**Products for a material**, in this order:
1. AMS trays whose material matches, marked "Loaded in A2" and so on.
2. Catalog entries from preferred brands.
3. The rest of the catalog's matches, A–Z.

Each product has a `buy_link` (the plain URL for now) and enough to start profile generation.

### 2. The Advise page (Filament → Advise)

**Header.** The Filament hub's own header, with Advise as the current tab. A second `PageHeader` would add a second `<h1>`.

**Input band:**
- **Description field.** "What is the part for?" is shown only when AI is configured. Otherwise a one-line note says AI isn't set up.
- **Needs chips.** A pill row with `aria-pressed`. The heat options are mutually exclusive.
- **Model row (optional).** It has three controls:
  - "Use last sliced model";
  - "Choose model…";
  - Clear.

  It shows the file name, plus one line with the footprint size and the risks, for example "Overhang 64 mm² · Tall and thin 6.0:1". There is no 3D view on this page.
- **"Advise" button.** With a description, it interprets the text first and fills the chips, then scores. Otherwise it scores straight from the chips.

**Hero.**
- The top material is shown as a large Doto word.
- The detail line gives the fit and the main ticks.
- A row of rank pills ("1 · PETG", "2 · ASA", "3 · PLA") sits under the hero, with `aria-pressed`. Selecting a rank switches the panel below. The page keeps one hero, so there is no `HeroStats`.

**Reasons.** A `StatRow` list for the selected material, with one row per need marked ✓, ~ or ✗. The printer and geometry notes follow.

**Products.**
- A table of product, brand and a status tag ("Loaded in A2" or "Preferred"), with two actions: "Generate profile" and "Where to buy".
- One line above the table states the ordering rule.
- "Generate profile" reuses the Search page's existing flow: catalog entry, then specs, then generate, then install.

**Not on this printer.** A collapsed list of materials that fail a hard constraint, each with its reason.

**Settings → General** gets a "Preferred brands" picker, filled from the catalog's brands.

**Slice.** Next to the 3D view, a "Suggest filament" text button opens `/filament/advise?model=<path>`.

**States:**

| Situation | What the page shows |
|---|---|
| Before advising | `[EMPTY: Describe the part or pick what it needs]` |
| Interpreting the description | `[LOADING: Reading your description]` |
| AI fails | "Couldn't read the description; pick the needs below." Scoring still works. |
| Catalog empty | The products table asks you to refresh the catalog. |
| No printer configured | One line: "Assuming an enclosed printer; nozzle unknown." |
| Model fails to load | An inline error `StateText`. The advisor still runs without geometry. |

**Disclosure.** v1 has no affiliate links, so no disclosure is shown.

## Error handling

- Every frontend write after an `.await` uses `try_set`, and every read uses a `try_` read. Pages can unmount while a request is still in flight.
- AI output is untrusted. It is checked against the list of needs and never shown raw.
- Every advisor input is optional except the needs. An empty set of needs scores on easy print, the default.

## Testing

**Backend unit tests:**
- **Table invariants:**
  - every score is 0–3;
  - the heat deflection temperatures are plausible;
  - CF materials are abrasive;
  - TPU is flexible.
- **Scoring:**
  - "outdoor + heat ≥80" ranks ASA first on an enclosed printer, and lists it under "Not on this printer" on an A1;
  - "flexible" gives TPU;
  - a large footprint lowers the score of ABS;
  - an unknown nozzle adds a note to CF materials and does not filter them out;
  - ties are stable.
- **Interpretation:** unknown needs are dropped, and malformed JSON returns `None`.
- **Product order:** AMS first, then preferred brands, then A–Z.
- **Preferred brands:** they are stored and read back correctly.

**Frontend unit tests:** formatting of fit and reasons.

**Browser tests** (`app-flows`, with fixtures for `advise`, `advise_interpret`, the catalog and AMS):
- the chips toggle;
- a description fills the chips;
- the hero shows the top material;
- choosing rank 2 changes the reasons;
- AMS-loaded products come first;
- "Suggest filament" on Slice opens Advise with the model;
- with no AI, the description field is hidden.

## Out of scope

- Affiliate tagging and the hosted redirect.
- Prices and ratings.
- Per-product properties beyond the material, since the catalog doesn't have them.
- Multi-material and colour advice.
- Rotating or reorienting the model.
- Advice from the agent drawer: its tools can call `advise` later.
