# Calibration Mode: Guided Filament Calibration

**Status:** Designed 2026-10-25. The user approved each section as it was presented.
**Sub-project:** Phase 3 (print intelligence), fourth feature. It follows AMS filament detection (PR #36).
**Branch:** `claude/calibration-mode`, off `main` at 15dc43f.

## Goal

Calibrate a filament without a notebook. BambuMate:
- says which test to run next;
- gives the exact values to enter in Bambu Studio;
- turns what the user reads off the print into the preset value, using Bambu Studio's own formulas;
- saves that value into the right preset;
- keeps a history per filament.

## Decisions

| Topic | Decision |
|---|---|
| Approach | **Guide and record.** Bambu Studio prints the tests. BambuMate guides, reads, saves and records. It never generates test prints of its own. |
| Tests | Temperature, flow rate (coarse and fine), retraction, and max volumetric speed. Pressure advance is an optional step that is recorded only. |
| Saving | **Into the user's preset, or into a copy.** A user preset is edited in place, with a backup and an undo. A Bambu system preset is first copied to "<name> (calibrated)", and the copy is assigned to the AMS slot when a slot was chosen. |
| Reading | **Pick, with an optional photo for the AI.** The user taps a band or patch, or types a value or height. An optional photo gets an AI suggestion, which the user confirms. |
| Placement | **A Filament → Calibrate tab**, plus a "Calibrate" link on the AMS slot cards and on profiles. |

## Constraints

- **Read-only printer.** MQTT stays read-only: `pushall` and `get_version` only. BambuMate never writes K to the printer and never starts a print.
- **Never guess.**
  - A value is computed only from the setup values the user ran with and the reading the user gave.
  - If the extruder variant to write can't be determined, the user is asked; BambuMate never writes every variant.
  - An AI suggestion is never saved without the user's tap.
- **AGPL.** The facts below were learned from Bambu Studio's source and Bambu Lab's wiki. The formulas are reimplemented, and no code is copied.
- **Design.**
  - Use Nothing tokens and the existing patterns (step strip, hub tabs), in both themes.
  - Keep one hero per page, and no second PageHeader inside the hub.
  - Red is kept for real errors.
- **Async.** Use `try_*` signal access after `.await`.
- **Release builds.** The `claude-subscription` feature stays test-only.

## Facts from Bambu Studio

These come from `bambulab/BambuStudio` master as of 2026-10-13 (release v02.08.02.61) and from wiki.bambulab.com. The research notes are kept in the session scratchpad.

**Where the tests live.**
- **The top Calibration tab** holds two wizards, Flow Dynamics and Flow Rate. It needs a connected Bambu printer. On the H2 family, flow rate is manual only.
- **The Calibration menu** is on the menu bar on macOS, and on the top bar elsewhere.
  - It holds Temperature, Flow rate (Coarse / Fine), Pressure advance, Retraction test, and More (Max flowrate, VFA).
  - For a Bambu printer preset it shows only when **Developer Mode** is on (Preferences).
  - Each item starts a new project, which the user slices and prints normally.
  - The menu saves nothing. The user types the result into the preset, and the edited presets are left dirty with the test overrides.
- **Launching.** Bambu Studio can't be launched into a calibration: there is no CLI flag and no URL action for it.

**The tests.**

| Test | Inputs (defaults) | Layout | Value to save | Preset key |
|---|---|---|---|---|
| Temperature | Start and end by material, step fixed at 5 °C. Clamped to 180–350, start ≥ end + 5. | 10 mm blocks, hottest at the bottom | `start − 5 × k`, where k is the block index from the bed (0 = bottom) | `nozzle_temperature` |
| Flow rate, coarse (pass 1) | Base = the preset's `filament_flow_ratio` | 9 blocks, modifiers −20 to +20 % in steps of 5 | `base × (100 + M) / 100` | `filament_flow_ratio` |
| Flow rate, fine (pass 2) | Base = the pass 1 result, which must already be saved | 10 blocks, modifiers −9 to 0 | `base × (100 + M) / 100` | `filament_flow_ratio` |
| Retraction | Start 0, end 2, step 0.1 mm per mm of height | Tower of height `1.4 + (end − start) / step` mm | `start + floor(max(0, h − 0.4)) × step` | `filament_retraction_length` |
| Max volumetric speed | Start 5, end 20, step 0.5 mm³/s per mm of height | Spiral wall of height `(end − start + 1) / step` mm | `start + h × step` (wiki: 10 + 14 × 1 = 24) | `filament_max_volumetric_speed` |
| Pressure advance | Studio's Flow Dynamics wizard (auto, or manual line or pattern) | n/a | K, which Studio saves **on the printer** (`extrusion_cali` table) | none: preset `pressure_advance` is ignored on Bambu printers |

The default temperature ranges (start / end) are:

| Material | Start / end (°C) |
|---|---|
| PLA | 230 / 190 |
| ABS and ASA | 270 / 230 |
| PETG | 250 / 230 |
| PCTG | 280 / 240 |
| TPU | 240 / 210 |
| PA-CF and PET-CF | 320 / 280 |
| Custom | 230 / 190 |

Results must fall in these ranges:
- temperature: 180–350 °C;
- flow ratio: greater than 0 and less than 2;
- retraction: 0–10 mm;
- max volumetric speed: greater than 0 and at most 200;
- K: greater than 0 and less than 2.

**H2 variants.**
- `filament_flow_ratio`, `filament_max_volumetric_speed` and `filament_retraction_length` are per-variant arrays: one element per extruder and nozzle-volume variant.
- Studio's flow-rate wizard writes only the selected variant's element, and BambuMate does the same.
- `filament_retraction_length` may hold `"nil"`, meaning "use the printer's value".

**Order.** Bambu publishes no formal order. BambuMate recommends temperature → flow rate (coarse, then fine) → retraction → max volumetric speed, with pressure advance optional, because the H2 runs flow dynamics calibration before each print by default.

## Design

### 1. A guided run

**A run is for one filament.** That filament is either an AMS slot, using its assigned or detected preset, or any filament preset. The run is tied to the configured printer and nozzle.

**The step strip** reads Temperature · Flow · Retraction · Max speed · Pressure advance (optional).
- Each step reads **Not run**, **Saved** (with its value and date), **Recorded** (pressure advance), or **Skipped**.
- Any step can be opened; the order is a recommendation.

**Each step card has three parts.**
1. **Set up in Bambu Studio.**
   - The card gives the exact steps and values, for example: "Calibration → Temperature · PLA · start 230 · end 190", or "Calibration → Flow rate → Coarse".
   - BambuMate picks the defaults from the filament's material and its current preset. For max speed, the end is the larger of 20 and the preset's current value plus 5.
   - The user can change the values, and they are stored with the step so the reading maths matches.
   - An **Open Bambu Studio** button uses the existing launcher.
   - The first menu-based step shows, once, how to turn on Developer Mode.
   - The card also notes that Studio leaves its presets with unsaved test changes, and that those changes should be discarded.
2. **Read the print.**
   - A diagram is drawn from the stored setup values, and the user taps a band or patch:
     - temperature: one band per 5 °C, labelled;
     - flow rate: patches labelled by their modifier;
     - retraction and max speed: a ruler of heights.
   - Or the user types the value or the height.
   - **Add a photo** is optional. With an AI backend configured, BambuMate suggests a pick ("AI suggests the 225 °C band"), and the user taps to accept it.
   - BambuMate applies the formula from the table above and shows the value with its diff against the preset, for example `nozzle_temperature 220 → 225`.
3. **Save.**
   - **Save to preset** writes the value. **Undo** then appears inline.
   - The coarse flow result must be saved before the fine pass is offered, so that Studio slices the fine pass from it.
   - **Pressure advance** points the user to Studio's Flow Dynamics wizard, which saves K on the printer. The user can record the K they used. BambuMate never writes it anywhere.

**Linking a print.** A step card offers **Link the print**: the newest finished journal print after the step was opened is suggested first, followed by the four before it. Linked prints show a "Calibration: Temperature" chip in the journal, which links back to the step.

### 2. The interface

**Filament → Calibrate** is a new tab after Advise, Search and Batch, at the route `/filament/calibrate`. A query parameter (`?slot=A2`, or `?preset=<path>`) preselects the filament.

**The picker** sits at the top. It lists the AMS slots and filament presets, and shows the configured printer and nozzle as context.

**The active step's card is the hero.** The step strip sits above it, using the step-strip pattern from Search.
- **The value** is shown large in Doto, for example "225 °C" or "0.97", with the diff beneath it.
- **Save to preset** is the primary action.
- **The diagram** is mono and flat: bands as a column, patches as a row, and heights as a ruler. The selected item is marked with the signal colour.

**History** is a quiet table under the card, with these columns: date, test, value, preset, printer and nozzle, and a photo thumbnail where there is one.

**Entry points.**
- A **Calibrate** text link on each AMS slot card (Printer and Home) goes to `?slot=…`.
- The same link on a profile in Profiles → Library goes to `?preset=…`.
- The journal chip links back to the step.

**States.**
- **No preset.** When a slot has no assigned or detected preset, the card says so and links to Search to create one.
- **A system preset.** The card shows "Will save as Bambu PLA Basic (calibrated)" before saving.
- **Unknown variant.** The card asks the user to pick the variant, listing the preset's own variant names.
- **No AI backend.** The photo is kept, and there is no suggestion.

### 3. Data, errors, testing

**Storage** is `src-tauri/src/calibration/store.rs`. It is a SQLite database in the app data folder, using the journal's migration pattern.
- **`runs`** holds: id, `preset_name`, `preset_path`, `filament_id`, `printer_model`, `nozzle`, `slot_label` (optional), `created_at` and `updated_at`.
- **`results`** holds:
  - id, `run_id` and `test`, where test is one of `temperature`, `flow_coarse`, `flow_fine`, `retraction`, `max_volumetric_speed` or `pressure_advance`;
  - `setup` (JSON), `reading` (JSON) and `value`;
  - `preset_key`, `old_value`, `new_value`, `target_path`, `variant`, `backup_path`;
  - `photo_path` and `journal_entry_id` (both optional);
  - `saved` (bool; false for recorded-only), `undone` (bool) and `created_at`.
- **Photos** are copied under the app data folder in `calibration/photos/`, as journal photos are.

**Modules.**
- **`calibration/tests.rs`** is pure. It holds:
  - the per-material defaults;
  - the setup validation;
  - the formulas;
  - the band and patch labels;
  - the result ranges.
- **`calibration/apply.rs`** applies a result:
  - It resolves the target. A user preset is edited in place. A system preset is copied to "<name> (calibrated)", keeping the system preset's `filament_id`, using the existing duplicate path, and assigned to the slot if one was chosen. The copy keeps the parent's id as Bambu Studio's own user presets do, so a Bambu RFID spool (which reports that id) still reads as matched in its slot.
  - It resolves the variant index from the slot's extruder and nozzle volume type in the printer state; otherwise it uses the variant the user picked.
  - It writes one key, plus `nozzle_temperature_initial_layer` for temperature, which keeps its offset from `nozzle_temperature`.
  - It backs up the file first, writes through the guarded profile writer, validates the result, and bumps `note_presets_changed()`.
  - Max volumetric speed is written to one decimal place.
  - Undo restores the backup only if the file still holds the value that was written. Otherwise it reports that the preset has changed since.
- **`calibration/suggest.rs`** sends the photo and the test's labels to the configured AI backend, using the analyzer's vision path, with a fixed prompt. The reply is JSON (`{"pick": <label>, "confidence": 0..1, "reason": "…"}`) and is validated against the labels.
- **`commands/calibration.rs`** provides these commands:
  - `calibration_runs`
  - `calibration_start(slot | preset)`
  - `calibration_defaults(run, test)`
  - `calibration_preview(run, test, setup, reading)`, which returns the value, the diff and the target
  - `calibration_save`
  - `calibration_record` (pressure advance)
  - `calibration_undo`
  - `calibration_suggest`
  - `calibration_link_print`
- **The journal.** A new `purpose` field on the entry holds `{calibration_result_id, test}`. It is kept apart from the defect tags.

**Errors.**
- **Write failure or invalid result.** The card shows `[ERROR: …]` inline, and the preset is unchanged.
- **Preset changed on disk.** Save re-reads the current value. If it no longer matches the preview's old value, the card shows the new diff and asks again.
- **Value out of range.** The value is refused and the range is shown.
- **AI failure.** The card shows "Couldn't read the photo", and the user picks by hand.
- **No printer configured.** Runs still work on a preset. The variant is then always picked by the user.

**Testing.**
- **Unit tests:**
  - every formula against the wiki's worked examples (flow 0.98 with +5 gives 1.029; 1.029 with −6 gives 0.96726; max speed 10 + 14 × 1 = 24; pressure advance tower 15 mm gives 0.03 for reference);
  - band and patch labels and setup validation;
  - result ranges;
  - variant-element writes on an H2D dual-variant preset, and `"nil"` retraction handling;
  - the temperature initial-layer offset.
- **Store:** migrations, CRUD, and undo state.
- **Apply:**
  - a user preset is edited in place;
  - a system preset is copied and assigned;
  - undo works, and undo after an outside edit is refused;
  - the generation counter is bumped.
- **Suggest:** reply parsing and label validation, using a stubbed backend.
- **Browser (app-flows):**
  - the tab is routed;
  - pick a slot, then read a temperature band, save, see the diff, and undo;
  - the fine flow pass is offered only after coarse is saved;
  - pressure advance is record-only;
  - the Calibrate links from the slot cards and profiles work;
  - an AI suggestion is accepted;
  - linking a journal print shows the chip.

## Out of scope

- Generating BambuMate's own test prints.
- Writing K to the printer, or reading the printer's K table. Reading could be added later from the pushall tray data.
- The VFA and tolerance tests.
- Non-Bambu printers.
- Bambu Studio's printer-side wizards (auto flow dynamics). The user runs these in Studio.
