# Printer Live Connection

**Status:** Approved in brainstorming (2026-09-28).
**Sub-project:** Bambu integration item 3 (item 1: preset cloud sync, PR #26; item 2: Bambu Studio CLI slicing).
**Branch:** `claude/printer-live`, off `main` at 73c303c.

## Goal

BambuMate connects to the user's printer over the local network. It shows live print status, the contents of every AMS slot, and printer errors. It also guides the user to set each AMS slot to the right filament preset, then confirms the slot is set.

## Constraints

- **Read-only.** The user runs the printer in cloud mode. Bambu's Authorization Control firmware (2025+) rejects third-party control commands unless the printer is in LAN-only + Developer Mode. BambuMate publishes only the read requests the firmware allows (`pushall`, `get_version`) and never a control command.
- **No Bambu Cloud.** BambuMate makes no cloud API calls, does not log in, and never impersonates Bambu Studio or Bambu Connect.
- **One printer.** The primary target is the H2 series (H2D, H2C, H2S): dual nozzle, multiple AMS units, two external spool holders. Other Bambu models should parse without crashing, but they are not tuned.
- **Secure transport.** TLS is verified. There is no "disable verification" option.

## Design

### 1. Connection

- **Transport.** MQTT over TLS to `<printer-ip>:8883`, user `bblp`, password = the LAN access code. Uses `rumqttc` with `rustls`.
- **Certificate verification.** This uses a custom `rustls` verifier:
  1. Check the chain against Bambu's printer CA certificates, bundled in `src-tauri/resources/bambu-ca/`. These are public CA certs as published in OpenBambuAPI `tls.md`; the plan records each source.
  2. Require the leaf certificate's CN to equal the printer serial. Printers present certificates for their serial, not their IP, so hostname checks are replaced by this CN check.
- **Trust on first use.** If the chain doesn't verify against the bundled CAs (for example, a newer model with an unknown CA), the connection fails with a clear error. The setup screen then shows the certificate's SHA-256 fingerprint. The user can choose **Trust this printer**, which pins that fingerprint in settings. Later connections accept exactly that certificate, still with the CN check. A changed certificate fails and asks again.
- **Topics.** Subscribe to `device/<SERIAL>/report`. Publish to `device/<SERIAL>/request` only:
  - `{"pushing":{"sequence_id":"<n>","command":"pushall","version":1,"push_target":1}}`, on connect and at most once every 5 minutes;
  - `{"info":{"sequence_id":"<n>","command":"get_version"}}`, once on connect.
- **Reconnect.** Exponential backoff from 2 s up to 60 s.

**Connection states:** `Disconnected`, `Connecting`, `Connected`, `AuthFailed`, `CertUntrusted { fingerprint }`, `Unreachable`.

### 2. Setup (Settings → Printer)

- **Discovery.** A short SSDP listen on UDP 2021 (and 1900) lists nearby Bambu printers with their name, model, serial and IP. The user picks one. Manual entry of IP and serial is always available.
- **Access code.** The user enters the code shown on the printer screen. It is stored in the system keychain under service `bambumate-printer-access-code`, account = serial. It is never logged, never sent to the frontend after entry, and never exposed to the agent.
- **Settings.** IP, serial, name, model and the optional pinned fingerprint go in the app settings store.
- **Test connection.** A button connects, waits for the first full report, and shows the result.

### 3. Live state service (`src-tauri/src/printer/`)

- `client.rs`: connection, TLS, subscribe/publish and reconnect. Its only output is raw report JSON values and connection-state changes.
- `state.rs`: `PrinterState`, merged from reports. Full pushes replace the state; delta pushes (sent after the first on many models) merge field by field. It holds:
  - print state (`gcode_state`), file (`subtask_name`), `mc_percent`, `mc_remaining_time`, `layer_num`/`total_layer_num`;
  - bed temperature and target;
  - each nozzle's current and target temperature, diameter and type, plus which nozzle is active (H2 dual-nozzle fields);
  - AMS units, with humidity and temperature per unit;
  - trays per unit: `tray_type`, `tray_color`, `tray_info_idx`, `tray_sub_brands`, `nozzle_temp_min/max`, `remain`, `tag_uid`/`tray_uuid` (RFID), and `tray_now`;
  - external spools (`vt_tray` / `vir_slot`, up to two on H2);
  - `hms[]` and `print_error`.
- `hms.rs`: turns HMS codes into text.
  - Uses Bambu's public HMS code list: a static JSON over plain HTTPS, not an account API.
  - Fetched on first need, then cached in app data for 7 days.
  - Offline with no cache, it shows the code with a link to Bambu's wiki page for that code.
- `service.rs`: owns the client task. It exposes the latest `PrinterState` and connection state, and emits Tauri events `printer://state` (throttled to 2/s) and `printer://connection`.
- The service starts when a printer is configured, stops when it is removed, and restarts when settings change.

**Parsing.** Fields are optional, and unknown fields are ignored. A malformed message is logged at debug level and skipped; it never stops the connection.

### 4. UI

- **New Printer page** (`/printer`) on the rail. Its icon shows a connection dot: `--nd-signal` when connected, `--nd-warning` while connecting, `--nd-accent` on error.
- The page root carries the `.nd` scope, so it uses the Nothing tokens on `main` today and on the app-wide design system after PR #25. From top to bottom it shows:
  - **Current print** as the hero: state, progress %, layer x/y, remaining time and file name, with nozzle and bed temperatures below.
  - **AMS**: one row per unit, one card per slot. Each card has the colour swatch, material, the preset name (assigned or reported), remaining %, an RFID badge, and a slot status. External spools come after the units.
  - **Errors**: active HMS or print errors, shown inline with their text or the wiki link.
- **Not configured.** The page points to Settings → Printer.
- **Disconnected.** The page shows the last known state greyed out, with the connection status.

### 5. Guided slot assignment

- The user taps a slot card and picks which preset is loaded there from their filament presets: user presets first, then system presets, with search.
- The assignment is stored in a local SQLite table: `slot_assignments(serial, ams_id, tray_id, preset_name, filament_id, assigned_at)`. It lives in the existing history DB. External spools use `ams_id = 255` with their own tray ids.
- **Slot status** is computed from the assignment and the printer report:
  - **Matches:** the reported `tray_info_idx` equals the assigned preset's `filament_id` → **✓ Set**.
  - **RFID:** the tray has a Bambu RFID tag (a non-empty `tag_uid`/`tray_uuid`) → **✓ Bambu spool** (reported by the printer). No assignment is needed.
  - **Different:** the printer reports a different filament → **Set on printer**, with steps:
    - "On the printer: Filament → {slot label} → choose *{preset name}*."
    - "Or in Bambu Studio: Device → AMS → {slot label} → *{preset name}*."
    - If the preset is a user preset that isn't cloud-synced yet (its `.info` has no `setting_id`), the steps add: "It must sync to Bambu Cloud first — open Bambu Studio while signed in." This links to the preset sync Health check once PR #26 is merged; until then it only shows the text.
  - **Empty slot:** the report shows no filament → **Empty**. The assignment is kept but greyed out.
  - **Unassigned:** no assignment, no RFID → **Not set** (a prompt to pick).
- **Slot labels:** AMS units are lettered A, B, C, D and slots numbered 1–4, so slots read A1–D4. External spools are `Ext-L` / `Ext-R` on H2 (`Ext` on single-external models).
- When the printer's report changes to match, the card flips to ✓ with no user action.

### 6. Agent tools (read-only)

- `bm_printer_status`: connection state, the current print, temperatures, active errors with text, and model/serial (no IP, no access code).
- `bm_ams_slots`: every slot with its label, reported filament, RFID flag, remaining %, assigned preset and status.
- There are no write or control tools. Both tools return "No printer configured" or "Printer not connected" when that applies.

### 7. Errors and security

- **Wrong access code:** the MQTT CONNACK is refused as not authorised → `AuthFailed` → "The access code was rejected. Check it on the printer screen (Settings → LAN)."
- **Unreachable:** backoff, and the page shows "Can't reach the printer at {ip}."
- **Untrusted certificate:** the trust-on-first-use flow in §1.
- **Logs:** they never contain the access code. Logs of MQTT payloads are capped at debug level and keep the serial.
- **Discovery** only listens; it sends nothing.

## Testing

- **Parser and state:** fixtures of recorded report JSON cover an H2 full push, H2 deltas, and a P1-style delta. The fixtures come from community documentation (OpenBambuAPI, ha-bambulab) until real captures from the user's H2 replace them. Tests cover:
  - a delta merge that keeps unmentioned fields;
  - dual nozzles;
  - multiple AMS units;
  - external spools;
  - RFID trays;
  - malformed messages being skipped.
- **Slot status:** unit tests cover each status and the label mapping.
- **TLS verifier:** unit tests cover:
  - accepting a chain from a test CA with the right CN;
  - rejecting the wrong CN;
  - rejecting an unknown CA;
  - accepting a pinned fingerprint;
  - rejecting a changed pinned certificate.
- **Client loop:** an in-process test MQTT broker (a dev-dependency, e.g. `rumqttd`) runs over TLS with a test CA. The test covers connect, subscribe, pushall published, a report received, state updated, and reconnect after the broker drops. An auth failure maps to `AuthFailed`.
- **HMS:** cache hit and miss using a local HTTP stub; offline falls back to the wiki link.
- **WebKit `app-flows.mjs`:** mocked `printer://` events and commands exercise the Printer page:
  - the hero, the AMS cards and the error text;
  - assigning a preset to a slot;
  - the mismatch steps;
  - the card flipping to ✓ when a mocked report changes;
  - Settings → Printer setup with discovery results, and the trust-this-printer flow.
- **Manual acceptance on the user's H2:**
  1. Discover and connect.
  2. Check the live status updates during a print.
  3. Check that the AMS cards match the printer screen.
  4. Assign a preset, set it on the printer, and see ✓.

## Out of scope

- Printer control of any kind (start/stop/pause, AMS writes, calibration, temperatures, LEDs).
- The camera, multiple printers, print history or journal, and file upload.
- Developer Mode support, and Bambu Cloud.

## Risks

| Risk | Mitigation |
|---|---|
| The H2 series uses a CA that isn't bundled. | Trust-on-first-use pinning with an explicit user choice; still CN-checked. |
| Report field names differ on H2 firmware from community docs. | Parse leniently (every field optional); replace fixtures with real captures during manual acceptance. |
| Bambu changes the firmware to restrict local reads. | Bambu currently documents status pushes as unaffected. The page degrades to "not connected" with no crash. |
| The HMS JSON endpoint moves. | The cache survives, and the fallback is the code plus the wiki link. |
