# AMS Filament Detection: What's Loaded, and Is It the Right One?

**Status:** Designed 2026-10-25. The user approved each section as it was presented.
**Sub-project:** Phase 3 (print intelligence), third feature. It follows the filament advisor (PR #34) and the print journal (PR #35).
**Branch:** `claude/ams-detection`, off `main` at f7f225e.

## Goal

Answer two questions without the user having to look anything up:
1. **What's in each slot?** Each AMS slot names the profile its filament is, from the filament ID the printer reports.
2. **Is this job's filament loaded?** Before printing, the Slice result (and Home) says whether the filament the job was sliced with is in the AMS.

## Decisions

| Topic | Decision |
|---|---|
| Scope | **Identify and check.** Slot cards show the detected profile, and the Slice result checks the job's filament against the loaded slots. |
| Placement | **The Slice result and Home.** The Slice result shows a status line under each plate's filament. Home's idle hero adds a quiet line for the newest sliced job when its filament isn't loaded. It is information only and never blocks anything. |
| A detected profile | **Shown, with a one-click "Use this".** Nothing is assigned until the user clicks. When the user's own assignment differs, both are shown and the user's assignment is kept. |

## Constraints

- **Read-only.** MQTT stays read-only (`pushall` and `get_version` only). BambuMate never sets a filament on the printer.
- **Never guess.** If a slot reports no filament ID, it shows its material only. If an ID matches no installed preset, the slot shows "not installed". If the printer isn't connected, the check reads "Can't check the AMS: printer not connected" and never uses stale slot data.
- **Not an error.** A filament that isn't loaded is not an error. Signal colour marks only a loaded filament, and red is kept for real errors.
- **Design.** Use Nothing tokens and the existing patterns, in both themes. Stay within one hero per page.
- **AGPL.** Bambu Studio's source may be read to learn formats, but never copied.

## Design

### 1. Detection and matching

**The filament ID index** (`src-tauri/src/printer/filament_index.rs`, pure and tested)
- **What it holds.** It is built from Bambu Studio's system filament presets and the user's own. Each preset is resolved through its `inherits` chain to its real `filament_id`, reusing the slicer's `PresetIndex::flatten`. For example, a system variant gets GFA00 from `Bambu PLA Basic @base`. The result maps each `filament_id` to its presets.
- **Choosing among variants.** One ID has many variants, one per printer (`@BBL H2C`, `@BBL X1C` and so on). The index picks the one compatible with the configured printer. If several fit, it prefers the one for the configured nozzle, then the first by name. User presets, whose IDs begin with `P`, resolve to that exact preset.
- **Lookup.** `lookup(id) -> Option<Detected { preset_name, path, is_user, variants }>`.
- **Caching.** The index is cached in the printer service. It is rebuilt when any of these changes:
  - the printer model;
  - the nozzle;
  - a preset is installed or synced;
  - a preset folder's modification time, checked at most once a minute.

**The detected profile for each slot.** `SlotView` gains `detected: Option<Detected>`, filled in `compute_slots`.

| Slot | What it shows |
|---|---|
| Empty | Nothing |
| A filament ID that matches an installed preset | "Detected: <preset name>" |
| A material but no ID (a third-party spool without RFID) | "PLA, no profile ID" |
| An ID that matches no installed preset | "GFL99 — not installed" |

**The job's filament.**
- The slice parser reads `tray_info_idx` from `slice_info.config` into `FilamentInfo.filament_id`.
- If that is missing, the ID is resolved from the job's filament preset name through the index.

**Matching** (`ams_match(job, slots) -> AmsMatch`, pure). It checks the AMS trays and the external spools, and ignores empty slots.
1. **Exact.** A slot's filament ID equals the job's, or the slot's assigned preset is the job's preset. Shown as "✓ Loaded in A2".
2. **Same material.** The material type matches but the profile doesn't. Material types are compared with the advisor's token classifier. Shown as "~ PETG in B1, different profile: check temperatures".
3. **Not loaded.** Shown as "✗ Not loaded: load PETG-CF before printing".
4. **No printer.** The printer isn't connected. Shown as "Can't check the AMS: printer not connected".

A colour difference only adds the note "different colour". It is never counted as a mismatch.

### 2. The interface

**Slot cards** (Printer page `SlotCard` and Home's AMS strip `HomeSlot`)
- **The detected line.** A mono "Detected: Bambu PETG HF" sits under the slot's material.
  - The `@BBL …` suffix is hidden on the card. The full name is in the tooltip and the accessible label.
- **With no assignment.** A **Use this** text button saves the detected preset as the slot's assignment, through the existing `printer_assign_slot`.
  - The "set it on the printer" guide does not open, because the printer already reports this filament.
- **With a different assignment.** The card shows "Yours: <assigned> · Detected: <detected>", keeps the user's assignment, and the existing "Different" status applies.
- **Not installed.** The card shows "GFL99 — not installed", with no button.

**The Slice result** (`ResultView`)
- **The status line.** A line under each plate's filament row shows one of the match lines from §1.
- **Colour.** "Loaded" uses the signal colour, "Same material" uses the neutral warning tone, and "Not loaded" and "Can't check" use plain text.
- **Live updates.** The line updates as the trays change, through `printer://state`.

**Home**
- **The line.** When the newest finished slice job's filament isn't loaded exactly, the idle hero adds one quiet line linking to Slice, for example "Gear needs PETG-CF · not loaded".
- **When it's hidden.** It is hidden when the filament is loaded, when there is no recent job, and while printing.
- **Why only the newest job.** Slice jobs last only for the session.

### 3. Data flow, errors, testing

**Data flow**
- `printer_view` carries `detected` per slot, so the frontend needs no new command for it.
- The match is worked out from the job's filament (carried on the slice job view) and the live slots. The plan decides which side computes it, but the rules above are the same either way.

**Errors**
- **The index can't be built:** for example, folders unreadable. No slot shows a detected line, a warning goes to the log, and no raw text reaches the page.
- **An unknown ID:** the slot shows "not installed".
- **The printer disconnects:** the check shows "Can't check the AMS". Slot data is used only while connected, the same rule the journal uses.

**Testing**
- **Index:**
  - IDs resolve through `inherits`;
  - the right printer variant is chosen;
  - a user `P` ID resolves to its own preset;
  - a missing ID gives `None`.
- **`ams_match`:**
  - each of the four levels;
  - the colour note;
  - empty slots are ignored;
  - the external spool is included.
- **Parser:** `tray_info_idx` is read from the H2C slice fixture.
- **Browser** (app-flows):
  - the slot card shows "Detected", and "Use this" assigns it;
  - the Slice result shows each line from fixture trays;
  - Home's line appears and disappears as the trays change;
  - "Can't check" shows when disconnected.

## Out of scope

- Setting a filament on the printer. MQTT stays read-only.
- Generating a profile for an uninstalled detected ID. That can be added later, linked from the "not installed" state.
- Multi-material jobs. BambuMate slices one filament per job today.
- Bambu Studio's own slot mapping when it sends a job.
- Calibration values (`k`, `cali_idx`).
