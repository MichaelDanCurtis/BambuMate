# Print Journal: Every Print, Recorded

**Status:** Designed 2026-10-24, section by section, with the user's approval at each step.
**Sub-project:** Phase 3 (print intelligence), second feature. The first was the filament advisor (PR #34).
**Branch:** `claude/print-journal`. It branches from `claude/filament-advisor`, and is rebased onto `main` once PR #34 merges.

## Goal

Keep a journal of every print without asking the user to do anything:
- what printed, and when;
- how long it took;
- which filaments it used;
- how it ended.

The user can then record how it turned out: a verdict, issue tags, notes and photos. A photo can go straight into Analyze. The journal is the record that later features, such as the advisor and calibration, will learn from.

## Decisions

| Topic | Decision |
|---|---|
| Capture | **Automatic, then annotated.** The backend watches the printer and records each print, and the user adds a verdict, tags, notes and photos afterwards. |
| Placement | **A Printer hub tab.** Printer gets Live · Journal. Home's last-print line comes from the journal. |
| Analyze link | **Analyze from the entry.** "Analyze a photo" opens Analyze with the photo, model and filament preset filled in. The analysis links back to the entry. Photos are files in the app data folder, not base64 in the database. |
| Rating | **Verdict and issue tags.** The verdict is Good, Usable or Failed. Issue tags come from Analyze's defect names. Free-text notes are also available. |

## What the printer tells us

These facts come from reading Bambu Studio's open-source `DeviceManager` (AGPL; it was read to learn the protocol, and none of its code is copied).

- A report is `{"print": {...}}`. LAN reports are full state. Diffs (`msg: 1`) replace arrays and scalars whole and merge objects. The app already keeps merged state, so the watcher reads that.
- **Job identity:**
  - `task_id` and `subtask_id` (strings, `"0"` for local and SD jobs), plus `job_id`;
  - `subtask_name` and `gcode_file`;
  - `plate_idx`. For local jobs, Studio reads the plate number from the digits between the last `_` and the last `.` of the gcode file name.
- **State:** `gcode_state` takes the values IDLE, PREPARE, SLICING, RUNNING, PAUSE, FINISH and FAILED. "Printing" means PREPARE, SLICING, RUNNING or PAUSE.
- **Errors:** `print_error` is an int, shown as 8 hex digits with a dash after the first four. `hms` is an array of codes.
- **Not available over LAN:**
  - **Start time.** `gcode_start_time` exists, but Studio never parses it, so its format is unconfirmed.
  - **Cancellation.** There is no distinct cancelled signal: a user stop can't be told apart from a failure.
  - **Filament used.** Grams and metres per job come only from the Bambu cloud.
  - **Cover image.** No LAN thumbnail route is confirmed.
- **Trays:**
  - `ams.tray_now` is a numeric string. 255 means none, 254 means the external spool, and 0x80–0x87 mean standalone units. Otherwise `ams = v >> 2` and `slot = v & 3`.
  - Each tray has `tray_type`, `tray_sub_brands`, `tray_color` (hex without `#`), `remain` (%) and `remain_g` (-1 means unknown).
  - On the H2, `device.extruder.info[]` carries `(ams << 8) | slot`, where 0xFFFF means none.

## Design

### 1. Recording prints (`src-tauri/src/journal/`)

**Watcher (`watch.rs`)**
- The watcher is a pure state machine: `observe(prev: &WatchState, report: &PrinterState, now) -> (WatchState, Vec<JournalEvent>)`.
- It emits these events: `Started`, `TrayUsed`, `Ended` and `Lost`.
- `PrinterService::handle_event` feeds it after merging each report. It runs whenever the app is running, including with the window closed. The resulting writes happen off the async thread.

**When an entry starts**
- An entry opens when a job identity not already open enters PREPARE, SLICING, RUNNING or PAUSE.
- The job key is `task_id` when it isn't `"0"` or empty. Otherwise it is `subtask_name` plus `gcode_file`.
- The start time is the local clock when the job is first seen.
- If the app connects mid-print (progress above 0 at first sight), the start is back-dated from the progress and the remaining time, and marked **approximate**.

**When an entry ends**
- **FINISH:** the outcome is `finished`.
- **FAILED:** the outcome is `failed`. The `print_error` code and any HMS codes are kept. Cancellations also land here. The user's verdict and notes can record "cancelled".
- **Unknown:** if the job disappears while the app isn't watching (app closed, printer unreachable through the end, or a different job or IDLE seen on reconnect), the outcome is `unknown`, shown as "Ended while BambuMate wasn't watching". It is never guessed. On app start, an entry still open is closed as `unknown` unless the same job is still printing.

**Filament**
- Every tray seen as the current tray during the print is recorded once, with its type, colour, brand and slot label (for example A2).
- Grams and time come only from a linked slice, labelled "slice estimate". Without a link, none are shown.

**Slice link**
- At `Started`, the newest slice job from this session is matched when its model name or output file matches `subtask_name` or `gcode_file`.
- Matching ignores case, the extension and `_plate_N`.
- A match snapshots the printer, process and filament presets, the plate's time, weight and filament estimates, and the plate thumbnail into the entry. The slice cache can be cleared, so the snapshot keeps that data.

**Storage (`store.rs`)**
- Entries live in `app_data/print_journal.db`, versioned with `PRAGMA user_version`.
- Tables:
  - `prints`: id, job_key, name, gcode_file, plate, printer_model, started_at, start_approx, ended_at, outcome, error_code, hms_json, verdict, tags_json, notes, slice_json, updated_at;
  - `print_trays`;
  - `print_photos`;
  - `print_analyses`, which links to `refinement_sessions`.
- Photos are stored in `app_data/print-journal/photos/<entry-id>/`.
- Nothing is pruned automatically. Deleting an entry also removes its photo files.
- A write that fails is logged and swallowed, so it never breaks the printer connection.

### 2. The Journal tab (Printer → Journal)

**Hero**
- A large Doto line, "This month", for example "14 prints".
- The detail line, for example "11 good · 2 usable · 1 failed · 38 h printing". It counts entries that have a verdict, or else a known outcome.

**Filters**
- Pills: All · Needs a verdict · Failed.
- A search box that matches name, filament and notes.

**List**
- Newest first, one row per print.
- Each row shows:
  - the plate thumbnail, when the entry is linked to a slice;
  - the name;
  - when and how long;
  - filament chips (swatch, type, slot);
  - the outcome tag;
  - the verdict.
- A running print shows at the top as "Printing now", linking to Live.

**Entry detail** (expands in place; no modal and no new page)
- **Facts:**
  - start and end, marked approximate where they are;
  - duration;
  - printer;
  - plate;
  - error and HMS codes on failure;
  - slice estimates, when linked.
- **Verdict:** Good / Usable / Failed pills with `aria-pressed`. Issue tags are toggle chips using Analyze's defect names.
- **Notes:** a text area that saves on blur.
- **Photos:** add with the file picker or drag-and-drop. Shown as a thumbnail strip, each one removable.
- **Actions:**
  - "Analyze a photo" opens Analyze with `?journal=<id>`, with the photo, model and filament preset filled in.
  - "Delete entry" asks for confirmation inline.

**Home**
- The idle hero's last print comes from `journal_last`, so it survives a restart.
- A finished print with no verdict adds a quiet "Rate your last print" link.

**States**

| Situation | What the page shows |
|---|---|
| No entries | `[EMPTY: Prints appear here as your printer runs them]` |
| No printer configured | A note that the journal fills in by itself once a printer is set up |

### 3. Commands and data flow

**Commands**
- `journal_list(filter)`, where the filter takes All, NeedsVerdict or Failed, plus a query.
- `journal_get(id)`.
- `journal_update(id, verdict?, tags?, notes?)`.
- `journal_add_photo(id, path)`.
- `journal_remove_photo(id, photo)`.
- `journal_delete(id)`.
- `journal_month_summary()`.
- `journal_last()`.
- After each write, the backend emits `journal://changed`.

**Photos**
- A photo is copied in and re-encoded to JPEG with a 2048 px long edge.
- Images over 25 MB and non-images are refused with a plain sentence.

**Analyze**
- `analyze_print` takes an optional `journal_id`. When one is given, the new session id is recorded in `print_analyses`.

**Frontend**
- `src/journal/` holds the types, the bridge and the formatting code.
- The page is `src/pages/journal.rs`, and the Printer hub tabs become Live · Journal.
- Analyze accepts `?journal=<id>`.
- Every write after an await uses `try_*`, and a generation counter guards against stale list loads.

## Error handling

- Failed database or photo writes show an inline sentence. Raw OS or SQLite text never reaches the page.
- The watcher never panics. Unknown states are ignored, missing fields keep their previous values, and malformed numbers are skipped.
- A photo file that is missing or corrupt shows a placeholder.

## Testing

**Watcher unit tests** run report sequences built from the facts above:
- a normal finish;
- FAILED with an error;
- joining mid-print, which gives an approximate start;
- a restart while printing;
- a job switch with no FINISH in between;
- tray changes, including the external spool and H2 extruders;
- diff reports with no `gcode_state`;
- `task_id` of "0" for local jobs.

**Store tests** on a temp DB:
- create, read, update and delete;
- deleting an entry removes its photo files;
- the version check;
- the month summary.

**Other unit tests:**
- slice matching, including case, the extension and `_plate_N`;
- frontend formatting.

**Browser tests** (app-flows, with journal fixtures):
- the list renders;
- the filters work;
- the verdict and tags save;
- the notes save on blur;
- a photo can be added;
- "Analyze a photo" reaches Analyze with the journal id;
- delete asks for confirmation inline;
- Home's "Rate your last print" link appears.

## Out of scope

- Cloud data: grams used, cover images and Bambu account history.
- Reading the printer's SD card or FTP.
- Feeding the journal into the advisor. That comes later, once data exists.
- Export and import.
- Several printers. The journal records the configured printer.
