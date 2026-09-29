# Printer Live Connection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect BambuMate to the user's Bambu printer on the local network, read-only, and show live print status, every AMS slot with guidance to set it to the right preset, and printer errors.

**Architecture:**
- A new backend module, `src-tauri/src/printer/`, owns everything printer-side:
  - `state.rs` parses reports and merges full pushes and deltas into `PrinterState`.
  - `tls.rs` verifies the printer against Bambu's CAs with a CN = serial check, or a user-pinned fingerprint.
  - `client.rs` runs the `rumqttc` connection and reconnect. Its only output is raw reports and connection states.
  - `hms.rs` turns error codes into text, `slots.rs` computes slot labels and status, `discovery.rs` listens for SSDP, and `settings.rs` stores settings and the keychain entry.
  - `service.rs` ties these together and emits `printer://state` (at most twice a second) and `printer://connection`.
- Slot assignments live in a new `slot_assignments` table in the existing history database.
- Eight `printer_*` Tauri commands serve Settings → Printer and the Printer page. Two read-only agent tools, `bm_printer_status` and `bm_ams_slots`, read the same view with the IP removed.
- The frontend adds a `PrinterShared` context fed by the two events, a Settings → Printer section, a `/printer` page under the `.nd` scope, and a connection dot on the sidebar entry.

**Tech Stack:** Rust (Tauri 2, rumqttc 0.25 with rustls 0.23 on the ring provider, x509-parser, sha2, socket2, rusqlite, reqwest, keyring), Leptos 0.8 CSR/WASM with Trunk, Playwright WebKit/Chromium flows in `tests/webkit`. Tests use rcgen, tokio-rustls and an in-process MQTT broker.

**Spec:** `docs/superpowers/specs/2026-09-28-printer-live-design.md`

## Global Constraints

- **Read-only:** BambuMate publishes only `{"pushing":{"sequence_id":"<n>","command":"pushall","version":1,"push_target":1}}` (on connect, at most once every 5 minutes) and `{"info":{"sequence_id":"<n>","command":"get_version"}}` (once per connect) to `device/<SERIAL>/request`. It never publishes a control command. `client::READ_ONLY_COMMANDS` lists the two, and a client test asserts every published message uses one of them.
- **No Bambu Cloud:** no cloud API calls, no login, and no impersonation of Bambu Studio or Bambu Connect. The only outbound HTTP is the public HMS error list (`https://e.bambulab.com/query.php`), a static JSON document that needs no account.
- **TLS is always verified:** the chain must reach a bundled Bambu CA, or the certificate's SHA-256 must equal the fingerprint the user pinned. The leaf CN must equal the serial either way. There is no "disable verification" option, and no code path builds a verifier that accepts anything else.
- **Access code:**
  - It is stored only in the system keychain: service `bambumate-printer-access-code`, account = serial.
  - It is never logged. `ClientParams` has a hand-written `Debug` that redacts it, and `rumqttc::MqttOptions` is never formatted.
  - It is never returned to the frontend (`PrinterConfigView` has only `has_access_code: bool`) and never reaches the agent.
- **One printer:** one `PrinterConfig` in the settings store under key `printer`, and one `PrinterService` managed by Tauri.
- **IP address:** the agent tools never include the printer's IP.
- **UI copy:** these strings are verbatim from the spec. On the Printer page the preset name in both "Set on printer" steps is rendered in `<em>`; the spec's `*…*` marks that italics.
  - `The access code was rejected. Check it on the printer screen (Settings → LAN).`
  - `Can't reach the printer at {ip}.`
  - Badges: `✓ Set`, `✓ Bambu spool`, `Set on printer`, `Empty`, `Not set`.
  - Steps: `On the printer: Filament → {slot label} → choose *{preset name}*.`, `Or in Bambu Studio: Device → AMS → {slot label} → *{preset name}*.`, `It must sync to Bambu Cloud first — open Bambu Studio while signed in.`
  - Buttons: `Test connection`, `Trust this printer`.
  - Agent tools: `No printer configured`, `Printer not connected`.
  - Sections: `Current print`, `AMS`, `Errors`.
- **Slot labels:** `A1`–`D4`, `Ext-L`/`Ext-R` on H2 (vir_slot ids 254/255), and `Ext` with one external spool. External spools use `ams_id = 255`.
- **Nothing scope:** the Printer page root element carries the `.nd` class and styles only with `--nd-*` tokens.
- **Product name:** the Claude product label is "Claude Agent", never "Claude Code", in any new UI copy, doc or comment.
- **Commits:**
  - Never commit `.omc/`, `.claude/` or `.superpowers/`. Stage explicit paths only (never `git add -A` or `git add .`).
  - Commit messages carry no attribution lines.
- **Logging:** MQTT payload logs are `debug` level at most and keep the serial. A malformed report is logged at `debug` and skipped; it never stops the connection.
- **Verification commands** (run all of them before the final commit of any task that touches that side):
  - `cargo fmt --check` and `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
  - `cd src-tauri && cargo test`
  - `cargo test --bin bambumate`
  - `cargo check --target wasm32-unknown-unknown`
  - `trunk build`
  - `cd tests/webkit && node app-flows.mjs ../..` (needs `trunk build` first; run `npm install --no-save playwright@1.49.1 && npx playwright install webkit chromium` in `tests/webkit` once if `node_modules` is missing)

## Spec deviations

Each item resolves a place where the spec is ambiguous or doesn't match the real code or protocol.

1. **"Rail" is the sidebar on `main`.** `main` has a text sidebar (`src/components/sidebar.rs`), not the icon rail PR #25 brings. The Printer entry is a sidebar link, "Printer", with the connection dot after the label. The dot element carries `.nd` so the `--nd-*` tokens resolve outside the page scope.
2. **`--nd-signal` doesn't exist on `main`.** It arrives with PR #25 (`claude/nothing-design-system`). The dot uses `var(--nd-signal, var(--nd-success))`, so it picks up the signal colour once PR #25 lands.
3. **The dot's states:**
   - No printer configured: no dot.
   - `Connected`: signal colour.
   - `Connecting` and `Disconnected` (a retry is pending): warning.
   - `AuthFailed`, `CertUntrusted`, `WrongSerial` and `Unreachable`: accent.
4. **New connection state `WrongSerial { presented }`.** The spec's states have no case for "the certificate is valid but for another printer". Trusting can't fix that, because the CN check always applies. Its copy is `The printer at {ip} reports serial {presented}. Check the serial in Settings → Printer.`
5. **`slot_assignments` gains `preset_path`.** The spec's columns can't tell later whether an assigned user preset has synced to Bambu Cloud: that needs the preset's `.info`. The table is `slot_assignments(serial, ams_id, tray_id, preset_name, filament_id, preset_path, assigned_at)` with primary key `(serial, ams_id, tray_id)`.
6. **The assigned preset's `filament_id` is resolved through `inherits`.** Leaf system presets such as `Bambu PLA Basic @BBL H2D` don't set `filament_id`; their `@base` parent does. The existing `inheritance::resolve_inheritance` deliberately skips `filament_id`, so `slots::resolve_filament_id` walks the chain itself. A preset whose id can't be resolved can never read **✓ Set**; it shows **Set on printer**.
7. **The RFID test is "not all zeros".** Printers report `tag_uid: "0000000000000000"` and `tray_uuid: "000…0"` for non-RFID spools (OpenBambuAPI `mqtt.md`), so "non-empty" means "contains a non-zero digit".
8. **Status precedence:** Empty → Matches → RFID → Different → Unassigned.
   - An empty slot is **Empty** even when assigned; the assignment is kept and greyed.
   - A matching assignment wins over the RFID badge.
   - An RFID spool with a stale assignment still reads **✓ Bambu spool**, because the spec says RFID spools need no assignment.
9. **Discovery listens on UDP 2021, 1990 and 1900.** Community sources disagree on 1900 and 1990 (see Protocol facts), so all three are joined. Discovery binds with `SO_REUSEADDR`, plus `SO_REUSEPORT` on macOS and Linux, because Bambu Studio also listens on 2021. It still only listens and sends nothing.
10. **TLS 1.2 only.** ha-bambulab caps printer TLS at 1.2 because some firmware (P2S 01.02.00.00) never answers a TLS 1.3 ClientHello. BambuMate does the same.
11. **Device intermediate CAs are bundled too.** Some printers don't send their `BBL Device CA …` intermediate. `resources/bambu-ca/` holds the three ha-bambulab ships (N6-V2, N7-V2, O1C2-V2), each signed by `BBL CA2 RSA`. They are used only as intermediates, never as trust anchors.
12. **`pushall` is not repeated on a quick reconnect.** It is sent on the first connect, then at most once every 5 minutes across reconnects (`PushallGate`). After a quick reconnect the merged state is kept and deltas keep it current. Test connection uses its own client, so it always sends one.
13. **Full push or delta:**
    - A push is full when `msg == 0`.
    - Without `msg` (the H2D mock pushes have none), a push is full when it has at least 20 top-level fields. Real full pushes have 60–100; deltas have a handful.
    - Merging is field by field. Arrays of objects with an `id` merge by id, and an object with only `id` (and `state`) replaces its old element. That is how an emptied tray arrives.
14. **HMS list per model.** `query.php?lang=en&d=<first 3 characters of the serial>` returns the model's list (the H2D list, `d=094`, has 580 more codes than the generic one). Only that 3-character model prefix is sent. The cache file is `hms_en_<prefix>.json` in app data.
15. **Print errors use the same list's `device_error` table.** `print_error` is shown as `XXXX_XXXX` and links to `https://wiki.bambulab.com/en/hms/home` when there is no text.
16. **Humidity:** `humidity` is a 1–5 level and `humidity_raw` a percentage. The card shows the percentage when present and the level otherwise.
17. **Test connection takes the typed access code.** It uses the code from the form when one is typed, else the keychain code for that serial. `printer_save` stores a typed code and deletes the old serial's keychain entry when the serial changes.
18. **Trust this printer saves and re-tests.** It saves the form with the pinned fingerprint, including the typed access code, then runs Test connection again.
19. **Reset for clean install also removes the printer.** `reset_to_clean_install` deletes the printer's keychain entry (keyed by serial, not `bambumate`) before the store is cleared, and stops the service.
20. **Copy the spec didn't give:**
    - Not configured: `No printer is set up yet.` + link `Set one up in Settings → Printer`
    - Connecting: `Connecting to the printer…`
    - Disconnected: `Not connected.`
    - Untrusted on the page: `The printer's certificate isn't from a Bambu CA that BambuMate knows. Trust it in Settings → Printer.`
    - Settings intro: `Connect to your Bambu printer on the local network to see live status and what is loaded in each AMS slot. BambuMate only reads from the printer.`
    - Discovery: `Find printers` / `Searching…` / `No printers found. Enter the IP address and serial number below.`
    - Fields: `IP address`, `Serial number`, `Access code` (placeholders `Saved in the system keychain` / `Shown on the printer screen (Settings → LAN)`)
    - Buttons and notes: `Save`, `Remove printer`, `Saved.`, `Printer removed.`, `Enter the access code shown on the printer screen.`
    - Test results: `Connected to {model}. Live status is on the Printer page.` / `Connected, but the printer hasn't sent its status yet.`
    - Trust: `This printer's certificate isn't signed by a Bambu CA that BambuMate knows. If this fingerprint matches your printer, trust it. BambuMate will accept only this certificate from now on.`
    - Slot picker: `Which preset is loaded here?`, `Search presets`, `Your presets`, `Bambu presets`, `Clear assignment`, `Cancel`
    - Errors: `No active errors.`, `Look up this code on the Bambu wiki`
    - AMS rows: `AMS A`…, `AMS HT1`, `External`; unit meta `Humidity {n}%` / `Humidity level {n}`
    - Validation: `Enter the printer's IP address, like 192.168.1.20.` / `Enter the printer's serial number (letters and digits).`
    - Preset guard: `That preset isn't in Bambu Studio's filament folders.`
21. **AMS HT units** (ids 128+) are labelled `HT1`, `HT2`, …; the spec only names A–D.
22. **The access code field is cleared after a successful save** and is never pre-filled.
23. **`printer_assign_slot` takes only the preset path.** The backend reads the name and resolves the filament id. It refuses paths outside Bambu Studio's system and user filament folders.
24. **Remaining % is shown only for RFID spools.** Printers report `remain: 0` (or −1) for spools they can't measure, so a number there would mislead.
25. **Sidebar position.** The Printer entry sits after Profiles.

## Protocol facts

Verified against primary community sources on 2026-09-28:

| Fact | Value | Source |
|---|---|---|
| Broker | MQTT 3.1.1 over TLS, `<ip>:8883`, user `bblp`, password = LAN access code | OpenBambuAPI `mqtt.md` |
| Topics | subscribe `device/<SERIAL>/report`, publish `device/<SERIAL>/request` | OpenBambuAPI `mqtt.md` |
| pushall | `{"pushing":{"sequence_id":"0","command":"pushall","version":1,"push_target":1}}`; "refrain from executing this command at intervals less than 5 minutes on the P1P" | OpenBambuAPI `mqtt.md` |
| get_version | `{"info":{"sequence_id":"0","command":"get_version"}}`; reply `info.module[]`, printer module has `product_name: "Bambu Lab H2D"` | OpenBambuAPI `mqtt.md`, ha-bambulab `mock_data/MOCK-H2D.json` |
| Full push marker | full pushes carry `msg: 0` on most models; the H2D mocks carry no `msg` | ha-bambulab `mock_data/*.json` |
| Printer cert | issued by Bambu; `CN=<printer serial>`; no IP SAN | OpenBambuAPI `tls.md` |
| CA bundle | BBL CA (RSA 2048, 2022–2032), BBL CA2 RSA and BBL CA2 ECC (self-signed 2025–2050, and cross-signed by BBL CA). Identical in all three sources | bambulab/BambuStudio `resources/cert/printer.cer` @ `f75448910c279287556ff8347606c87138c22347` (sha256 `36f2bcee…52a0`); OpenBambuAPI `examples/ca_cert.pem`; ha-bambulab `pybambu/certs/bambu.cert` |
| Device intermediates | `BBL Device CA N6-V2`, `N7-V2`, `O1C2-V2`, all signed by BBL CA2 RSA (`openssl verify` OK) | ha-bambulab `pybambu/certs/bambu_{x2c_260425,p2s_250626,h2c_251122}.cert` @ `0e027ff135a6d9265cb756d3e246747954c76722` |
| TLS version | ha-bambulab caps at TLS 1.2 ("P2S firmware 01.02.00.00 never responds to a TLS 1.3 ClientHello") | ha-bambulab `pybambu/bambu_client.py` `create_local_ssl_context` |
| Dual nozzle | `print.device.extruder.info[]` with `id` 0/1 and `temp` packed as `target << 16 \| current`; `print.device.extruder.state >> 4 & 0xF` = active extruder; id 0 = right, 1 = left | ha-bambulab `models.py` `Temperature.print_update`, `Extruder.print_update`, `right_nozzle_temperature` |
| Nozzle info | `print.device.nozzle.info[]` with `id`, `diameter` (number), `type` (`HS01`); H2C also lists rack nozzles with ids ≥ 16 | ha-bambulab `MOCK-H2D.json`, `MOCK-H2C.json` |
| Bed | `print.device.bed.info.temp` packed like the nozzles; legacy `bed_temper` / `bed_target_temper` | ha-bambulab `Temperature.print_update` |
| AMS | `print.ams.ams[]` with `id` (string), `humidity`, `humidity_raw`, `temp`, `tray[]`; AMS HT ids are 128+ | ha-bambulab `tests/pybambu/2AMS1-1AMS2-1AMSHT.json` |
| Tray fields | `id`, `tray_type`, `tray_color` (`RRGGBBAA`), `tray_info_idx`, `tray_sub_brands`, `nozzle_temp_min/max` (strings), `remain` (number), `tag_uid`, `tray_uuid`, `state`; an empty slot is `{"id":"3","state":0}` or `{"id":"0"}` | OpenBambuAPI `mqtt.md`; ha-bambulab mocks |
| External spools | H2: `vir_slot[]` with ids `"254"` and `"255"`, ha-bambulab maps 254 → left, 255 → right; other models: single `vt_tray` (id `"254"`) | ha-bambulab `models.py` `ExternalSpool.print_update`, `sensor.py` |
| tray_now | `ams.tray_now`: 255 = none, 254 = external, else `ams_id * 4 + tray_id` | OpenBambuAPI `mqtt.md` |
| HMS entry | `{"attr": 50331904, "code": 65543}` → `0300_0100_0001_0007` | ha-bambulab `models.py` `HMSList.print_update` |
| HMS text | `GET https://e.bambulab.com/query.php?lang=en[&d=<serial prefix>]` → `{"result":0,"data":{"device_hms":{"en":[{"ecode":"0300010000010007","intro":"…"}]},"device_error":{"en":[{"ecode":"0300400C","intro":"…"}]}}}` (fetched live; generic list 2045 HMS / 490 error codes, `d=094` 2592 / 869) | ha-bambulab `scripts/update_error_text.py`; live request 2026-09-28 |
| HMS wiki | `https://wiki.bambulab.com/en/{family}/troubleshooting/hmscode/{XXXX_XXXX_XXXX_XXXX}`, family `h2` (H2D, H2D Pro), `h2c`, `h2s`, `p2s`, `x2d`, `a1`, `x1`; hub `https://wiki.bambulab.com/en/hms/home` | ha-bambulab `hms_error_text/wiki_links.json` |
| SSDP | printer multicasts `NOTIFY` with `NT: urn:bambulab-com:device:3dprinter:1`, `Location: <ip>`, `USN: <serial>`, `DevModel.bambu.com`, `DevName.bambu.com`, `DevSignal.bambu.com`, `DevConnect.bambu.com`, `DevBind.bambu.com` | gashton/bambustudio_tools `bambudiscovery.sh`; contentnation "Bambu Lab Printer Reverse-Engineering Knowledge Base"; nuxx.net "Bambu Lab P1S on IoT VLAN" |
| Model codes | `O1D` H2D, `O1E` H2D Pro, `O1C2` H2C, `O1S` H2S, `BL-P001` X1 Carbon, `C13` X1E, `C11` P1P, `C12` P1S, `N7` P2S, `N1` A1 mini, `N2S` A1 | bambulab/BambuStudio `resources/profiles/BBL/machine/*.json` `model_id` |

**Not verified** (no real H2 capture available; replace fixtures during manual acceptance):
- Whether the H2D/H2C/H2S leaf certificate chains to the bundled CAs, and whether the printer sends its intermediate. There is no H2D device CA in any source. The trust-on-first-use pin covers a miss.
- Whether rustls (ECDHE-only suites) handshakes with every printer firmware. ha-bambulab uses OpenSSL.
- The `msg` marker on H2 pushes, and whether H2 sends deltas at all. The mocks show full pushes without `msg`.
- That `vir_slot` 254 is the left holder and 255 the right. This is ha-bambulab's mapping, not a capture.
- That `humidity_raw` is a percentage.
- The SSDP ports. Sources say 2021 plus 1990 or 1900, so BambuMate listens on all three.
- That `DevModel.bambu.com` carries the same codes as Bambu Studio's `model_id`. Some older docs show `3DPrinter-X1-Carbon`, which is mapped too.
- That the wiki pages exist for every code under the family path.
- That the printer answers a wrong access code with CONNACK 4 or 5. Both map to `AuthFailed`; a printer that just closes the socket would show `Unreachable`.
- The HMS endpoint's stability. The spec's risk table covers it: the cache survives, and the fallback is the code plus the wiki link.

## Crates

Checked on crates.io on 2026-09-28. All crypto uses the **ring** provider, passed explicitly with `ClientConfig::builder_with_provider`, so no process-wide `CryptoProvider` is installed or needed. aws-lc-rs needs cmake and NASM on Windows CI.

| Crate | Version | Where | Why |
|---|---|---|---|
| `rumqttc` | `0.25.1`, `default-features = false`, `features = ["use-rustls-no-provider"]` | deps | MQTT client. `use-rustls` would enable `tokio-rustls/default` → aws-lc-rs |
| `rustls` | `0.23.45`, `default-features = false`, `features = ["ring", "std", "tls12", "logging"]` | deps | custom `ServerCertVerifier`; `rustls::client::verify_server_cert_signed_by_trust_anchor`, `rustls::server::ParsedCertificate`, `rustls::crypto::verify_tls12_signature` |
| `x509-parser` | `0.18.1` | deps | read the leaf CN |
| `sha2` | `0.10` (0.10.9 already in `Cargo.lock`) | deps | certificate fingerprint |
| `socket2` | `0.6` with `all` | deps | `SO_REUSEADDR` / `SO_REUSEPORT` and multicast join for discovery |
| `rcgen` | `0.14.10` (default features: ring) | dev | test CA and printer certs |
| `tokio-rustls` | `0.26.4`, `default-features = false`, `features = ["ring", "tls12", "logging"]` | dev | TLS for the in-process test broker |
| `bytes` | `1` | dev | packet buffers in the test broker |

`rumqttd` was evaluated and rejected as the test broker:
- It pulls rustls 0.22.
- It drops a bad login without sending a CONNACK, so `AuthFailed` couldn't be tested.
- It can't close connections on demand, so reconnect couldn't be tested.

`src-tauri/src/printer/testbroker.rs` instead uses `rumqttc`'s own public `Packet::read`/`Packet::write` over a `tokio-rustls` server. It is about 230 lines.

## File Map

| File | Responsibility |
|---|---|
| `src-tauri/src/printer/mod.rs` (new) | Module list |
| `src-tauri/src/printer/state.rs` (new) | `parse_report`, `ReportMerger`, `merge_into`, `PrinterState` and friends; report fixtures (`fixtures` test module) |
| `src-tauri/src/printer/testdata/*.json` (new) | H2D full push, H2D deltas, P1P full + delta, H2D `get_version` |
| `src-tauri/resources/bambu-ca/*.pem` (new) | Bambu printer CA bundle and three device intermediates |
| `src-tauri/src/printer/tls.rs` (new) | `PrinterCertVerifier`, `RejectionSlot`, `fingerprint`, `client_config`; `testpki` test helpers |
| `src-tauri/src/printer/client.rs` (new) | `run` (connect, subscribe, requests, backoff), `test_connection`, `ConnectionState`, `PushallGate`, `Backoff` |
| `src-tauri/src/printer/testbroker.rs` (new, test only) | In-process TLS MQTT broker |
| `src-tauri/src/printer/hms.rs` (new) | `HmsCatalog` (fetch, 7-day cache, lookup), `ErrorView`, `hms_wiki_url` |
| `src-tauri/src/printer/slots.rs` (new) | `slot_label`, `slot_status`, `compute_slots`, `SlotView`, `resolve_filament_id`, `preset_needs_cloud_sync`, `set_on_printer_steps` |
| `src-tauri/src/printer/discovery.rs` (new) | SSDP `parse_notify`, `discover`, `model_name` |
| `src-tauri/src/printer/settings.rs` (new) | `PrinterConfig`, `PrinterConfigView`, store and keychain helpers |
| `src-tauri/src/printer/service.rs` (new) | `PrinterService`, `PrinterView`, `PrinterEvents`, `TauriEvents`, throttled emit |
| `src-tauri/src/history/{store,types}.rs` | `slot_assignments` table and methods; `SlotAssignment` |
| `src-tauri/src/commands/printer.rs` (new), `commands/mod.rs`, `lib.rs` | Eight commands; service start at launch |
| `src-tauri/src/commands/config.rs` | Reset also removes the printer's keychain entry |
| `src-tauri/src/agent/tools/printer.rs` (new), `tools/{mod,app,fake_host}.rs`, `agent/{host,mod}.rs`, `agent/codex/mod.rs`, `agent/claude/mcp_server.rs` | Two read-only tools; `/printer` route; tool counts 18 → 20 |
| `src/printer/{mod,types,bridge}.rs` (new), `src/main.rs`, `src/app.rs` | Shared printer state, event listeners, invoke wrappers |
| `src/components/printer_settings.rs` (new), `components/mod.rs`, `src/pages/settings.rs` | Settings → Printer |
| `src/pages/printer.rs` (new), `pages/mod.rs`, `src/components/sidebar.rs` | Printer page, slot picker, rail dot |
| `style/printer.css` (new), `index.html` | Styles |
| `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs` | Printer fixtures, `window.__fixtures`, flow steps |

---

### Task 1: Report parser and merged printer state

**Files:**
- Create: `src-tauri/src/printer/mod.rs`
- Create: `src-tauri/src/printer/state.rs`
- Create: `src-tauri/src/printer/testdata/h2d_full.json`, `h2d_delta_progress.json`, `h2d_delta_ams.json`, `p1p_full.json`, `p1p_delta.json`, `get_version_h2d.json`
- Modify: `src-tauri/src/lib.rs:9-10` (module list)

**Interfaces:**
- Consumes: nothing new (`serde`, `serde_json`).
- Produces (in `crate::printer::state`):
  - `pub enum Report { Status { print: Map<String, Value>, full: bool }, Version { model: Option<String>, firmware: Option<String> }, Other }` (`Debug, Clone, PartialEq`)
  - `pub fn parse_report(payload: &[u8]) -> Result<Report, String>`
  - `pub struct ReportMerger` (`Default`) with `pub fn apply(&mut self, print: &Map<String, Value>, full: bool) -> PrinterState` and `pub fn has_full(&self) -> bool`
  - `pub fn merge_into(base: &mut Map<String, Value>, delta: &Map<String, Value>)`
  - `pub struct PrinterState { gcode_state: Option<String>, subtask_name: Option<String>, mc_percent: Option<u32>, mc_remaining_time: Option<u32>, layer_num: Option<u32>, total_layer_num: Option<u32>, bed_temp: Option<f64>, bed_target_temp: Option<f64>, nozzles: Vec<Nozzle>, active_nozzle: Option<u32>, ams_units: Vec<AmsUnit>, external_spools: Vec<Tray>, tray_now: Option<u32>, hms: Vec<HmsCode>, print_error: Option<u32> }` with `pub fn from_print(p: &Map<String, Value>) -> Self`. All fields are `pub`, and it derives `Debug, Clone, Default, PartialEq, Serialize, Deserialize`.
  - `pub struct Nozzle { id: u32, temp, target_temp, diameter: Option<f64>, nozzle_type: Option<String> }`
  - `pub struct AmsUnit { id: u32, humidity_level: Option<u32>, humidity_pct: Option<u32>, temp: Option<f64>, trays: Vec<Tray> }`
  - `pub struct Tray { id: u32, empty: bool, tray_type, tray_color, tray_info_idx, tray_sub_brands: String, nozzle_temp_min, nozzle_temp_max: Option<u32>, remain: Option<u32>, tag_uid, tray_uuid: String }` with `pub fn has_rfid(&self) -> bool`
  - `pub struct HmsCode { attr: u32, code: u32 }` (`Copy`) with `pub fn display(&self) -> String` → `"0300_0100_0001_0007"`
  - Test-only `pub(crate) mod fixtures`, reused by Tasks 3, 5, 7 and 8:
    - `H2D_FULL`, `H2D_DELTA_PROGRESS`, `H2D_DELTA_AMS`, `P1P_FULL`, `P1P_DELTA`, `GET_VERSION_H2D: &str`
    - `pub fn state_after(fixtures: &[&str]) -> PrinterState`

- [ ] **Step 1: Add the fixtures**

These are trimmed from ha-bambulab `custom_components/bambu_lab/pybambu/mock_data/MOCK-H2D.json` at commit `0e027ff135a6d9265cb756d3e246747954c76722` (2026-09-21). The trim adds one emptied slot (B4), a non-RFID PETG in A3, a non-RFID user preset in B2 (`P4d6ae04`), a loaded Ext-L and one HMS code. The P1P fixtures follow the OpenBambuAPI `mqtt.md` pushall example.

Create `src-tauri/src/printer/testdata/h2d_full.json`:

```json
{
  "print": {
    "command": "push_status",
    "sequence_id": "2021",
    "gcode_state": "RUNNING",
    "subtask_name": "T-pose - slim H2D dual AMS riser",
    "mc_percent": 6,
    "mc_remaining_time": 549,
    "layer_num": 1,
    "total_layer_num": 200,
    "bed_temper": 70.0,
    "bed_target_temper": 70.0,
    "nozzle_temper": 245.0,
    "nozzle_target_temper": 245.0,
    "nozzle_diameter": "0.4",
    "nozzle_type": "stainless_steel",
    "print_error": 0,
    "hms": [{ "attr": 50331904, "code": 65543 }],
    "wifi_signal": "-45dBm",
    "ipcam": { "ipcam_dev": "1", "resolution": "1080p" },
    "device": {
      "bed": { "info": { "temp": 4587590 }, "state": 2 },
      "extruder": {
        "info": [
          { "id": 0, "snow": 259, "temp": 16056565 },
          { "id": 1, "snow": 65279, "temp": 47 }
        ],
        "state": 2
      },
      "nozzle": {
        "exist": 3,
        "info": [
          { "diameter": 0.4, "id": 0, "type": "HS01", "wear": 0 },
          { "diameter": 0.4, "id": 1, "type": "HS01", "wear": 0 }
        ],
        "state": 0
      },
      "type": 1
    },
    "ams": {
      "ams_exist_bits": "3",
      "tray_now": "3",
      "version": 275305,
      "ams": [
        {
          "id": "0",
          "info": "2103",
          "humidity": "5",
          "humidity_raw": "21",
          "temp": "27.0",
          "tray": [
            { "id": "0", "state": 11, "tray_type": "PLA-S", "tray_sub_brands": "Support for PLA", "tray_info_idx": "GFS02", "tray_color": "FFFFFFFF", "nozzle_temp_min": "190", "nozzle_temp_max": "230", "remain": 1, "tag_uid": "0C047A9600000100", "tray_uuid": "95558FE23FB14F3A9E4985B8B37839B1", "cols": ["FFFFFFFF"] },
            { "id": "1", "state": 11, "tray_type": "PLA", "tray_sub_brands": "PLA Basic", "tray_info_idx": "GFA00", "tray_color": "FFFFFFFF", "nozzle_temp_min": "190", "nozzle_temp_max": "230", "remain": 31, "tag_uid": "9AD3FBAC00000100", "tray_uuid": "5998E373899342BE9FC6BFA51103AB78", "cols": ["FFFFFFFF"] },
            { "id": "2", "state": 11, "tray_type": "PETG", "tray_sub_brands": "", "tray_info_idx": "GFG99", "tray_color": "1F7A3DFF", "nozzle_temp_min": "230", "nozzle_temp_max": "260", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000", "cols": ["1F7A3DFF"] },
            { "id": "3", "state": 11, "tray_type": "PLA", "tray_sub_brands": "PLA Basic", "tray_info_idx": "GFA00", "tray_color": "FFFFFFFF", "nozzle_temp_min": "190", "nozzle_temp_max": "230", "remain": 100, "tag_uid": "FABCE4E000000100", "tray_uuid": "8CCF700805CD483086E38C8AC5BB6ED9", "cols": ["FFFFFFFF"] }
          ]
        },
        {
          "id": "1",
          "info": "2003",
          "humidity": "5",
          "humidity_raw": "18",
          "temp": "29.7",
          "tray": [
            { "id": "0", "state": 11, "tray_type": "PLA", "tray_sub_brands": "PLA Basic", "tray_info_idx": "GFA00", "tray_color": "000000FF", "nozzle_temp_min": "190", "nozzle_temp_max": "230", "remain": 55, "tag_uid": "64AFAB0000000100", "tray_uuid": "CA9B8419B0284ACA84044AA73FF17557", "cols": ["000000FF"] },
            { "id": "1", "state": 11, "tray_type": "PLA", "tray_sub_brands": "", "tray_info_idx": "P4d6ae04", "tray_color": "F95959FF", "nozzle_temp_min": "190", "nozzle_temp_max": "240", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000", "cols": ["F95959FF"] },
            { "id": "2", "state": 11, "tray_type": "PETG-CF", "tray_sub_brands": "PETG-CF", "tray_info_idx": "GFG50", "tray_color": "000000FF", "nozzle_temp_min": "240", "nozzle_temp_max": "270", "remain": 100, "tag_uid": "B2A80B7000000100", "tray_uuid": "DE69ECEBB924468D99CA2A9DFCB5FD3B", "cols": ["000000FF"] },
            { "id": "3", "state": 0 }
          ]
        }
      ]
    },
    "vir_slot": [
      { "id": "254", "tray_type": "PLA", "tray_sub_brands": "", "tray_info_idx": "GFA01", "tray_color": "76D9F4FF", "nozzle_temp_min": "190", "nozzle_temp_max": "240", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000", "cols": ["76D9F4FF"] },
      { "id": "255", "tray_type": "", "tray_sub_brands": "", "tray_info_idx": "", "tray_color": "00000000", "nozzle_temp_min": "0", "nozzle_temp_max": "0", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000", "cols": ["00000000"] }
    ],
    "vt_tray": { "id": "255", "tray_type": "", "tray_info_idx": "", "tray_color": "00000000", "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000" }
  }
}
```

Create `src-tauri/src/printer/testdata/h2d_delta_progress.json`:

```json
{
  "print": {
    "command": "push_status",
    "msg": 1,
    "sequence_id": "2022",
    "mc_percent": 7,
    "mc_remaining_time": 540,
    "layer_num": 2,
    "device": {
      "extruder": {
        "info": [{ "id": 0, "temp": 16056564 }],
        "state": 2
      }
    }
  }
}
```

Create `src-tauri/src/printer/testdata/h2d_delta_ams.json`. The printer now reports `P1234567` in B2, as if the user set the slot on the printer:

```json
{
  "print": {
    "command": "push_status",
    "msg": 1,
    "sequence_id": "2023",
    "ams": {
      "ams": [
        {
          "id": "1",
          "tray": [
            { "id": "1", "state": 11, "tray_type": "PLA", "tray_sub_brands": "", "tray_info_idx": "P1234567", "tray_color": "F95959FF", "nozzle_temp_min": "190", "nozzle_temp_max": "240", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000" }
          ]
        }
      ]
    }
  }
}
```

Create `src-tauri/src/printer/testdata/p1p_full.json`:

```json
{
  "print": {
    "command": "push_status",
    "msg": 0,
    "sequence_id": "2021",
    "gcode_state": "IDLE",
    "subtask_name": "",
    "mc_percent": 0,
    "mc_remaining_time": 0,
    "layer_num": 0,
    "total_layer_num": 0,
    "bed_temper": 25.0,
    "bed_target_temper": 0.0,
    "nozzle_temper": 25.0,
    "nozzle_target_temper": 0.0,
    "nozzle_diameter": "0.4",
    "nozzle_type": "stainless_steel",
    "print_error": 0,
    "hms": [],
    "ams": {
      "ams_exist_bits": "1",
      "tray_now": "255",
      "version": 4,
      "ams": [
        {
          "id": "0",
          "humidity": "4",
          "temp": "22.7",
          "tray": [
            { "id": "0" },
            { "id": "1", "tray_type": "PLA", "tray_sub_brands": "", "tray_info_idx": "GFA00", "tray_color": "000000FF", "nozzle_temp_min": "190", "nozzle_temp_max": "240", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000" },
            { "id": "2", "tray_type": "PLA", "tray_sub_brands": "", "tray_info_idx": "GFA05", "tray_color": "DFE2E3FF", "nozzle_temp_min": "190", "nozzle_temp_max": "240", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000" },
            { "id": "3", "tray_type": "PLA", "tray_sub_brands": "", "tray_info_idx": "GFL00", "tray_color": "F95959FF", "nozzle_temp_min": "190", "nozzle_temp_max": "240", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000" }
          ]
        }
      ]
    },
    "vt_tray": { "id": "254", "tray_type": "ABS", "tray_sub_brands": "", "tray_info_idx": "GFB99", "tray_color": "000000FF", "nozzle_temp_min": "240", "nozzle_temp_max": "280", "remain": 0, "tag_uid": "0000000000000000", "tray_uuid": "00000000000000000000000000000000" }
  }
}
```

Create `src-tauri/src/printer/testdata/p1p_delta.json`:

```json
{
  "print": {
    "command": "push_status",
    "msg": 1,
    "sequence_id": "2030",
    "gcode_state": "RUNNING",
    "mc_percent": 42,
    "nozzle_temper": 219.5,
    "nozzle_target_temper": 220.0
  }
}
```

Create `src-tauri/src/printer/testdata/get_version_h2d.json`:

```json
{
  "info": {
    "command": "get_version",
    "sequence_id": "1",
    "module": [
      { "hw_ver": "N/A", "name": "ota", "product_name": "Bambu Lab H2D", "sn": "0948AB000000001", "sw_ver": "01.01.01.00" },
      { "hw_ver": "N3F05", "name": "n3f/0", "product_name": "AMS 2 Pro (1)", "sn": "REDACTED", "sw_ver": "02.00.19.47" }
    ]
  }
}
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/printer/mod.rs`:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod state;
```

In `src-tauri/src/lib.rs`, add `pub mod printer;` after `pub mod model_catalog;`:

```rust
pub mod model_catalog;
pub mod printer;
mod process_command;
```

Create `src-tauri/src/printer/state.rs` with only the test code for now. The implementation goes above it in Step 4.

```rust
#[cfg(test)]
pub(crate) mod fixtures {
    //! Report fixtures. `h2d_full` is trimmed from ha-bambulab
    //! `pybambu/mock_data/MOCK-H2D.json` (commit 0e027ff, 2026-09-21), with
    //! one slot emptied, one non-RFID spool and one loaded external spool
    //! added. `p1p_full` follows the OpenBambuAPI `mqtt.md` pushall example.
    //! Replace with real captures from the user's H2 during acceptance.
    pub const H2D_FULL: &str = include_str!("testdata/h2d_full.json");
    pub const H2D_DELTA_PROGRESS: &str = include_str!("testdata/h2d_delta_progress.json");
    pub const H2D_DELTA_AMS: &str = include_str!("testdata/h2d_delta_ams.json");
    pub const P1P_FULL: &str = include_str!("testdata/p1p_full.json");
    pub const P1P_DELTA: &str = include_str!("testdata/p1p_delta.json");
    pub const GET_VERSION_H2D: &str = include_str!("testdata/get_version_h2d.json");

    use super::{parse_report, PrinterState, Report, ReportMerger};

    /// Applies each fixture in order and returns the final state.
    pub fn state_after(fixtures: &[&str]) -> PrinterState {
        let mut merger = ReportMerger::default();
        let mut state = PrinterState::default();
        for f in fixtures {
            if let Report::Status { print, full } = parse_report(f.as_bytes()).unwrap() {
                state = merger.apply(&print, full);
            }
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn full_h2d_push_parses_print_progress_and_temperatures() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.gcode_state.as_deref(), Some("RUNNING"));
        assert_eq!(
            s.subtask_name.as_deref(),
            Some("T-pose - slim H2D dual AMS riser")
        );
        assert_eq!(s.mc_percent, Some(6));
        assert_eq!(s.mc_remaining_time, Some(549));
        assert_eq!((s.layer_num, s.total_layer_num), (Some(1), Some(200)));
        assert_eq!((s.bed_temp, s.bed_target_temp), (Some(70.0), Some(70.0)));
        assert!(s.print_error.is_none());
    }

    #[test]
    fn dual_nozzles_come_from_the_device_block() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.nozzles.len(), 2);
        assert_eq!(s.nozzles[0].id, 0);
        assert_eq!(s.nozzles[0].temp, Some(245.0));
        assert_eq!(s.nozzles[0].target_temp, Some(245.0));
        assert_eq!(s.nozzles[1].temp, Some(47.0));
        assert_eq!(s.nozzles[1].target_temp, Some(0.0));
        assert_eq!(s.nozzles[1].diameter, Some(0.4));
        assert_eq!(s.nozzles[1].nozzle_type.as_deref(), Some("HS01"));
        assert_eq!(s.active_nozzle, Some(0));
    }

    #[test]
    fn multiple_ams_units_parse_with_every_tray() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.ams_units.len(), 2);
        let b = &s.ams_units[1];
        assert_eq!(b.id, 1);
        assert_eq!(b.humidity_level, Some(5));
        assert_eq!(b.humidity_pct, Some(18));
        assert_eq!(b.temp, Some(29.7));
        assert_eq!(b.trays.len(), 4);
        assert_eq!(b.trays[2].tray_type, "PETG-CF");
        assert_eq!(b.trays[2].nozzle_temp_min, Some(240));
        assert!(b.trays[3].empty, "an id-only tray is an empty slot");
        assert_eq!(s.tray_now, Some(3));
    }

    #[test]
    fn rfid_trays_are_told_apart_from_third_party_spools() {
        let s = state_after(&[H2D_FULL]);
        let a = &s.ams_units[0];
        assert!(a.trays[1].has_rfid());
        assert_eq!(a.trays[1].remain, Some(31));
        assert!(!a.trays[2].has_rfid(), "all-zero tag ids are not RFID");
        assert_eq!(a.trays[2].tray_info_idx, "GFG99");
    }

    #[test]
    fn h2_external_spools_come_from_vir_slot() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.external_spools.len(), 2);
        assert_eq!(s.external_spools[0].id, 254);
        assert_eq!(s.external_spools[0].tray_info_idx, "GFA01");
        assert!(!s.external_spools[0].empty);
        assert_eq!(s.external_spools[1].id, 255);
        assert!(s.external_spools[1].empty);
    }

    #[test]
    fn hms_codes_decode_to_the_wiki_form() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.hms.len(), 1);
        assert_eq!(s.hms[0].display(), "0300_0100_0001_0007");
    }

    #[test]
    fn delta_merge_keeps_unmentioned_fields() {
        let s = state_after(&[H2D_FULL, H2D_DELTA_PROGRESS]);
        assert_eq!(s.mc_percent, Some(7));
        assert_eq!(s.layer_num, Some(2));
        assert_eq!(s.nozzles[0].temp, Some(244.0));
        // Not in the delta: unchanged.
        assert_eq!(s.nozzles[1].temp, Some(47.0));
        assert_eq!(s.nozzles[1].nozzle_type.as_deref(), Some("HS01"));
        assert_eq!(
            s.subtask_name.as_deref(),
            Some("T-pose - slim H2D dual AMS riser")
        );
        assert_eq!(s.ams_units.len(), 2);
        assert_eq!(s.hms.len(), 1);
    }

    #[test]
    fn delta_for_one_tray_changes_only_that_tray() {
        let s = state_after(&[H2D_FULL, H2D_DELTA_AMS]);
        let b = &s.ams_units[1];
        assert_eq!(b.trays[1].tray_info_idx, "P1234567");
        assert_eq!(b.trays[0].tray_info_idx, "GFA00");
        assert_eq!(b.humidity_pct, Some(18));
        assert_eq!(s.ams_units[0].trays.len(), 4);
    }

    #[test]
    fn an_id_only_tray_in_a_delta_empties_the_slot() {
        let emptied = r#"{"print":{"command":"push_status","msg":1,
            "ams":{"ams":[{"id":"0","tray":[{"id":"1","state":0}]}]}}}"#;
        let s = state_after(&[H2D_FULL, emptied]);
        let tray = &s.ams_units[0].trays[1];
        assert!(tray.empty);
        assert_eq!(tray.tray_info_idx, "");
        assert!(!tray.has_rfid());
    }

    #[test]
    fn a_full_push_replaces_the_state() {
        let one_unit = r#"{"print":{"command":"push_status","msg":0,
            "ams":{"ams":[{"id":"0","tray":[{"id":"0"}]}]}}}"#;
        let s = state_after(&[H2D_FULL, one_unit]);
        assert_eq!(s.ams_units.len(), 1);
        assert!(s.gcode_state.is_none());
    }

    #[test]
    fn p1_style_delta_merges_into_a_single_nozzle_state() {
        let s = state_after(&[P1P_FULL, P1P_DELTA]);
        assert_eq!(s.gcode_state.as_deref(), Some("RUNNING"));
        assert_eq!(s.mc_percent, Some(42));
        assert_eq!(s.nozzles.len(), 1);
        assert_eq!(s.nozzles[0].temp, Some(219.5));
        assert_eq!(s.nozzles[0].diameter, Some(0.4));
        assert_eq!(s.nozzles[0].nozzle_type.as_deref(), Some("stainless_steel"));
        assert_eq!(s.active_nozzle, None);
        assert_eq!(s.bed_temp, Some(25.0));
        assert_eq!(s.tray_now, None, "255 means nothing loaded");
        assert!(s.ams_units[0].trays[0].empty);
        assert_eq!(s.external_spools.len(), 1);
        assert_eq!(s.external_spools[0].id, 254);
        assert_eq!(s.external_spools[0].tray_type, "ABS");
    }

    #[test]
    fn get_version_reports_the_model_name() {
        assert_eq!(
            parse_report(GET_VERSION_H2D.as_bytes()).unwrap(),
            Report::Version {
                model: Some("H2D".into()),
                firmware: Some("01.01.01.00".into())
            }
        );
    }

    #[test]
    fn malformed_messages_are_errors_not_panics() {
        assert!(parse_report(b"not json").is_err());
        assert!(parse_report(b"[1,2,3]").is_err());
        assert!(parse_report(br#"{"print":"nope"}"#).is_err());
        assert_eq!(
            parse_report(br#"{"print":{"command":"gcode_line","result":"success"}}"#).unwrap(),
            Report::Other
        );
    }

    #[test]
    fn wrong_typed_fields_are_ignored() {
        let odd = r#"{"print":{"command":"push_status","msg":0,
            "mc_percent":"abc","layer_num":-3,"hms":[{"attr":"x"}],"ams":{"ams":"?"}}}"#;
        let s = state_after(&[odd]);
        assert_eq!(s.mc_percent, None);
        assert_eq!(s.layer_num, None);
        assert!(s.hms.is_empty());
        assert!(s.ams_units.is_empty());
    }

    #[test]
    fn a_push_without_msg_is_full_only_when_it_is_large() {
        let small = br#"{"print":{"command":"push_status","mc_percent":1}}"#;
        assert!(matches!(
            parse_report(small).unwrap(),
            Report::Status { full: false, .. }
        ));
        assert!(matches!(
            parse_report(H2D_FULL.as_bytes()).unwrap(),
            Report::Status { full: true, .. }
        ));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib printer::state`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `parse_report` and the other items not written yet.

- [ ] **Step 4: Write the implementation**

Insert this at the top of `src-tauri/src/printer/state.rs`, above the `#[cfg(test)]` line:

```rust
//! Printer state built from `device/<serial>/report` messages.
//!
//! Pushes are merged as raw JSON first (`ReportMerger`), then read into the
//! typed `PrinterState`. Every field is optional and unknown fields are
//! ignored, so models other than the H2 series parse without tuning.
//!
//! Field names follow OpenBambuAPI `mqtt.md` and ha-bambulab `pybambu/models.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A push without a `msg` marker counts as full when it has at least this
/// many top-level fields. Real full pushes have 60-100; deltas a handful.
const FULL_PUSH_MIN_FIELDS: usize = 20;

/// One message from the report topic.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    /// `print.push_status`: a full push replaces the state, a delta merges.
    Status {
        print: Map<String, Value>,
        full: bool,
    },
    /// `info.get_version`: the model name and firmware version.
    Version {
        model: Option<String>,
        firmware: Option<String>,
    },
    /// Any other message (command acknowledgements and the like).
    Other,
}

/// Parses one report payload. An error means the message is malformed; the
/// caller logs it at debug level and skips it.
pub fn parse_report(payload: &[u8]) -> Result<Report, String> {
    let value: Value = serde_json::from_slice(payload).map_err(|e| format!("not JSON: {e}"))?;
    let Value::Object(top) = value else {
        return Err("report is not a JSON object".into());
    };
    if let Some(print) = top.get("print") {
        let Value::Object(print) = print else {
            return Err("`print` is not an object".into());
        };
        let command = print.get("command").and_then(Value::as_str);
        if command.is_some() && command != Some("push_status") {
            return Ok(Report::Other);
        }
        let full = match print.get("msg").and_then(num_u64) {
            Some(0) => true,
            Some(_) => false,
            None => print.len() >= FULL_PUSH_MIN_FIELDS,
        };
        return Ok(Report::Status {
            print: print.clone(),
            full,
        });
    }
    if let Some(Value::Object(info)) = top.get("info") {
        if info.get("command").and_then(Value::as_str) == Some("get_version") {
            let modules = info
                .get("module")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let printer = modules.iter().find(|m| {
                m.get("product_name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.starts_with("Bambu Lab "))
            });
            let ota = modules
                .iter()
                .find(|m| m.get("name").and_then(Value::as_str) == Some("ota"));
            let model = printer
                .and_then(|m| m.get("product_name"))
                .and_then(Value::as_str)
                .map(|n| n.trim_start_matches("Bambu Lab ").to_string());
            let firmware = ota
                .and_then(|m| m.get("sw_ver"))
                .and_then(Value::as_str)
                .map(str::to_string);
            return Ok(Report::Version { model, firmware });
        }
    }
    Ok(Report::Other)
}

/// Keeps the merged raw `print` object and derives `PrinterState` from it.
#[derive(Debug, Default, Clone)]
pub struct ReportMerger {
    raw: Map<String, Value>,
    has_full: bool,
}

impl ReportMerger {
    /// Applies one push and returns the resulting state.
    pub fn apply(&mut self, print: &Map<String, Value>, full: bool) -> PrinterState {
        if full {
            self.raw = print.clone();
            self.has_full = true;
        } else {
            merge_into(&mut self.raw, print);
        }
        PrinterState::from_print(&self.raw)
    }

    /// True once a full push has arrived.
    pub fn has_full(&self) -> bool {
        self.has_full
    }
}

/// Merges `delta` into `base` field by field. Objects merge recursively.
/// Arrays of objects that all carry an `id` merge element by element on
/// that id; any other array replaces. An element with only `id` (and
/// `state`) replaces its old element: that is how an emptied tray is sent.
pub fn merge_into(base: &mut Map<String, Value>, delta: &Map<String, Value>) {
    for (key, value) in delta {
        merge_value(base.entry(key.clone()).or_insert(Value::Null), value);
    }
}

fn merge_value(base: &mut Value, delta: &Value) {
    match (base, delta) {
        (Value::Object(b), Value::Object(d)) => merge_into(b, d),
        (Value::Array(b), Value::Array(d)) if is_id_list(b) && is_id_list(d) => {
            for item in d {
                let id = id_key(item);
                match b.iter_mut().find(|old| id_key(old) == id) {
                    Some(old) if is_id_only(item) => *old = item.clone(),
                    Some(old) => merge_value(old, item),
                    None => b.push(item.clone()),
                }
            }
        }
        (b, d) => *b = d.clone(),
    }
}

fn is_id_list(items: &[Value]) -> bool {
    !items.is_empty() && items.iter().all(|v| v.get("id").is_some())
}

fn id_key(item: &Value) -> Option<String> {
    match item.get("id")? {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn is_id_only(item: &Value) -> bool {
    item.as_object()
        .is_some_and(|o| o.keys().all(|k| k == "id" || k == "state"))
}

/// The live printer state shown on the Printer page and to the agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrinterState {
    /// `gcode_state`: IDLE, PREPARE, RUNNING, PAUSE, FINISH, FAILED, ...
    pub gcode_state: Option<String>,
    /// `subtask_name`: the file being printed.
    pub subtask_name: Option<String>,
    pub mc_percent: Option<u32>,
    /// `mc_remaining_time`, in minutes.
    pub mc_remaining_time: Option<u32>,
    pub layer_num: Option<u32>,
    pub total_layer_num: Option<u32>,
    pub bed_temp: Option<f64>,
    pub bed_target_temp: Option<f64>,
    /// One entry per extruder: two on the H2D/H2C, one elsewhere.
    pub nozzles: Vec<Nozzle>,
    /// Index of the active extruder (H2 dual-nozzle only).
    pub active_nozzle: Option<u32>,
    pub ams_units: Vec<AmsUnit>,
    /// External spool holders: `vir_slot` (ids 254/255) on the H2 series,
    /// else the single `vt_tray`.
    pub external_spools: Vec<Tray>,
    /// `ams.tray_now` (255 = nothing loaded is `None`).
    pub tray_now: Option<u32>,
    pub hms: Vec<HmsCode>,
    /// `print_error`, when non-zero.
    pub print_error: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Nozzle {
    /// Extruder id. On the H2 series 0 is the right nozzle, 1 the left.
    pub id: u32,
    pub temp: Option<f64>,
    pub target_temp: Option<f64>,
    pub diameter: Option<f64>,
    /// `HS01` style on the H2 series, `stainless_steel` style elsewhere.
    pub nozzle_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AmsUnit {
    /// 0-3 for AMS / AMS 2 Pro, 128+ for AMS HT.
    pub id: u32,
    /// `humidity`: Bambu's 1-5 level.
    pub humidity_level: Option<u32>,
    /// `humidity_raw`: percent, on newer firmware.
    pub humidity_pct: Option<u32>,
    pub temp: Option<f64>,
    pub trays: Vec<Tray>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tray {
    pub id: u32,
    /// No filament reported in this slot.
    pub empty: bool,
    pub tray_type: String,
    /// `RRGGBBAA`.
    pub tray_color: String,
    /// The filament id of the preset set on this slot (`GFA00`, `P1234567`).
    pub tray_info_idx: String,
    pub tray_sub_brands: String,
    pub nozzle_temp_min: Option<u32>,
    pub nozzle_temp_max: Option<u32>,
    /// Remaining filament in percent. Only meaningful for RFID spools.
    pub remain: Option<u32>,
    pub tag_uid: String,
    pub tray_uuid: String,
}

impl Tray {
    /// A Bambu RFID spool: a tag uid or tray uuid that is not all zeros.
    pub fn has_rfid(&self) -> bool {
        is_set_id(&self.tag_uid) || is_set_id(&self.tray_uuid)
    }
}

fn is_set_id(s: &str) -> bool {
    s.chars().any(|c| c != '0')
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HmsCode {
    pub attr: u32,
    pub code: u32,
}

impl HmsCode {
    /// `0300_0100_0001_0007`, the form Bambu's wiki and apps show.
    pub fn display(&self) -> String {
        format!(
            "{:04X}_{:04X}_{:04X}_{:04X}",
            self.attr >> 16,
            self.attr & 0xFFFF,
            self.code >> 16,
            self.code & 0xFFFF
        )
    }
}

impl PrinterState {
    /// Reads the typed state out of a merged `print` object.
    pub fn from_print(p: &Map<String, Value>) -> Self {
        let device = p.get("device");
        let (bed_temp, bed_target_temp) = match device
            .and_then(|d| d.pointer("/bed/info/temp"))
            .and_then(num_u64)
        {
            Some(packed) => unpack_temp(packed),
            None => (
                p.get("bed_temper").and_then(num_f64),
                p.get("bed_target_temper").and_then(num_f64),
            ),
        };
        let extruder = device.and_then(|d| d.get("extruder"));
        let active_nozzle = extruder
            .and_then(|e| e.get("state"))
            .and_then(num_u64)
            .map(|s| ((s >> 4) & 0xF) as u32);
        let ams = p.get("ams");
        PrinterState {
            gcode_state: str_field(p, "gcode_state"),
            subtask_name: str_field(p, "subtask_name"),
            mc_percent: p.get("mc_percent").and_then(num_u32),
            mc_remaining_time: p.get("mc_remaining_time").and_then(num_u32),
            layer_num: p.get("layer_num").and_then(num_u32),
            total_layer_num: p.get("total_layer_num").and_then(num_u32),
            bed_temp,
            bed_target_temp,
            nozzles: nozzles(p, device),
            active_nozzle,
            ams_units: ams
                .and_then(|a| a.get("ams"))
                .and_then(Value::as_array)
                .map(|units| units.iter().filter_map(ams_unit).collect::<Vec<_>>())
                .map(sorted_units)
                .unwrap_or_default(),
            external_spools: external_spools(p),
            tray_now: ams
                .and_then(|a| a.get("tray_now"))
                .and_then(num_u32)
                .filter(|n| *n != 255),
            hms: p
                .get("hms")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|h| {
                            Some(HmsCode {
                                attr: h.get("attr").and_then(num_u32)?,
                                code: h.get("code").and_then(num_u32)?,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            print_error: p.get("print_error").and_then(num_u32).filter(|c| *c != 0),
        }
    }
}

fn nozzles(p: &Map<String, Value>, device: Option<&Value>) -> Vec<Nozzle> {
    let nozzle_info = device
        .and_then(|d| d.pointer("/nozzle/info"))
        .and_then(Value::as_array);
    let extruders = device
        .and_then(|d| d.pointer("/extruder/info"))
        .and_then(Value::as_array);
    if let Some(extruders) = extruders {
        let mut out: Vec<Nozzle> = extruders
            .iter()
            .filter_map(|e| {
                let id = e.get("id").and_then(num_u32)?;
                let (temp, target_temp) = e
                    .get("temp")
                    .and_then(num_u64)
                    .map(unpack_temp)
                    .unwrap_or((None, None));
                let info = nozzle_info.and_then(|list| {
                    list.iter()
                        .find(|n| n.get("id").and_then(num_u32) == Some(id))
                });
                Some(Nozzle {
                    id,
                    temp,
                    target_temp,
                    diameter: info.and_then(|n| n.get("diameter")).and_then(num_f64),
                    nozzle_type: info
                        .and_then(|n| n.get("type"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect();
        out.sort_by_key(|n| n.id);
        return out;
    }
    let any = ["nozzle_temper", "nozzle_target_temper", "nozzle_diameter"]
        .iter()
        .any(|k| p.contains_key(*k));
    if !any {
        return Vec::new();
    }
    vec![Nozzle {
        id: 0,
        temp: p.get("nozzle_temper").and_then(num_f64),
        target_temp: p.get("nozzle_target_temper").and_then(num_f64),
        diameter: p.get("nozzle_diameter").and_then(num_f64),
        nozzle_type: str_field(p, "nozzle_type"),
    }]
}

fn ams_unit(v: &Value) -> Option<AmsUnit> {
    let id = v.get("id").and_then(num_u32)?;
    let mut trays: Vec<Tray> = v
        .get("tray")
        .and_then(Value::as_array)
        .map(|t| t.iter().filter_map(tray).collect())
        .unwrap_or_default();
    trays.sort_by_key(|t| t.id);
    Some(AmsUnit {
        id,
        humidity_level: v.get("humidity").and_then(num_u32),
        humidity_pct: v.get("humidity_raw").and_then(num_u32),
        temp: v.get("temp").and_then(num_f64),
        trays,
    })
}

fn sorted_units(mut units: Vec<AmsUnit>) -> Vec<AmsUnit> {
    units.sort_by_key(|u| u.id);
    units
}

fn tray(v: &Value) -> Option<Tray> {
    let id = v.get("id").and_then(num_u32)?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let tray_type = s("tray_type");
    let tray_info_idx = s("tray_info_idx");
    Some(Tray {
        id,
        empty: tray_type.is_empty() && tray_info_idx.is_empty(),
        tray_color: s("tray_color"),
        tray_sub_brands: s("tray_sub_brands"),
        nozzle_temp_min: v
            .get("nozzle_temp_min")
            .and_then(num_u32)
            .filter(|t| *t > 0),
        nozzle_temp_max: v
            .get("nozzle_temp_max")
            .and_then(num_u32)
            .filter(|t| *t > 0),
        remain: v
            .get("remain")
            .and_then(num_f64)
            .filter(|r| *r >= 0.0)
            .map(|r| r as u32),
        tag_uid: s("tag_uid"),
        tray_uuid: s("tray_uuid"),
        tray_type,
        tray_info_idx,
    })
}

fn external_spools(p: &Map<String, Value>) -> Vec<Tray> {
    if let Some(slots) = p.get("vir_slot").and_then(Value::as_array) {
        let mut out: Vec<Tray> = slots.iter().filter_map(tray).collect();
        out.sort_by_key(|t| t.id);
        return out;
    }
    p.get("vt_tray").and_then(tray).into_iter().collect()
}

/// H2 packs temperatures as `target << 16 | current`.
fn unpack_temp(packed: u64) -> (Option<f64>, Option<f64>) {
    (
        Some((packed & 0xFFFF) as f64),
        Some(((packed >> 16) & 0xFFFF) as f64),
    )
}

fn str_field(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Numbers arrive as JSON numbers or as strings ("27.0", "190").
fn num_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite())
}

fn num_u64(v: &Value) -> Option<u64> {
    num_f64(v).filter(|f| *f >= 0.0).map(|f| f as u64)
}

fn num_u32(v: &Value) -> Option<u32> {
    num_u64(v).and_then(|n| u32::try_from(n).ok())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib printer::state`
Expected: `test result: ok. 15 passed; 0 failed`

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/printer/mod.rs src-tauri/src/printer/state.rs src-tauri/src/printer/testdata
git commit -m "Parse printer reports and merge pushes into a printer state"
```

---

### Task 2: Printer TLS verification

**Files:**
- Create: `src-tauri/resources/bambu-ca/bambu-printer-ca.pem`, `bbl-device-ca-n6-v2.pem`, `bbl-device-ca-n7-v2.pem`, `bbl-device-ca-o1c2-v2.pem`
- Create: `src-tauri/src/printer/tls.rs`
- Modify: `src-tauri/src/printer/mod.rs`
- Modify: `src-tauri/Cargo.toml` (dependencies and dev-dependencies)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (in `crate::printer::tls`):
  - `pub fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider>` (ring)
  - `pub enum Rejection { Untrusted { fingerprint: String }, WrongSerial { presented: String } }` (`Debug, Clone, PartialEq, Eq`)
  - `pub struct RejectionSlot` (`Clone, Default`) with `pub fn take(&self) -> Option<Rejection>`
  - `pub fn fingerprint(der: &[u8]) -> String` (`AB:CD:…`), `pub fn normalize_fingerprint(s: &str) -> String`, `pub fn leaf_common_name(der: &[u8]) -> Option<String>`
  - `pub struct PrinterCertVerifier`, which implements `rustls::client::danger::ServerCertVerifier`:
    - `pub fn bambu(serial: &str, pinned_fingerprint: Option<&str>, rejection: RejectionSlot) -> Result<Self, String>`
    - `pub fn with_trust(roots_pem: &str, intermediates_pem: &[&str], serial: &str, pinned_fingerprint: Option<&str>, rejection: RejectionSlot) -> Result<Self, String>`
  - `pub fn client_config(verifier: Arc<PrinterCertVerifier>) -> Result<Arc<rustls::ClientConfig>, String>` (TLS 1.2 only)
  - Test-only `pub(crate) mod testpki`, reused by Tasks 3 and 7:
    - `pub struct TestCa { pub pem: String, … }` with `TestCa::new(name: &str) -> TestCa` and `TestCa::leaf(&self, serial: &str) -> TestLeaf`
    - `pub struct TestLeaf { pub cert_der: Vec<u8>, pub key_pem: String }`

- [ ] **Step 1: Add the dependencies**

In `src-tauri/Cargo.toml`, after `uuid = { version = "1", features = ["v4"] }` in `[dependencies]`, add:

```toml
# Printer live connection (read-only MQTT over TLS to the printer on the LAN).
# rustls uses the ring provider explicitly: aws-lc-rs needs cmake/NASM on Windows.
rustls = { version = "0.23.45", default-features = false, features = ["ring", "std", "tls12", "logging"] }
x509-parser = "0.18.1"
sha2 = "0.10"
```

In `[dev-dependencies]`, after the `tokio` line, add:

```toml
# In-process TLS MQTT broker and test certificates for printer::client/tls tests.
rcgen = "0.14.10"
```

Run: `cd src-tauri && cargo tree -i aws-lc-rs`
Expected: `` error: package ID specification `aws-lc-rs` did not match any packages `` (nothing pulls aws-lc-rs).

- [ ] **Step 2: Add the Bambu CA certificates**

Fetch the files byte for byte from pinned commits:

```bash
mkdir -p src-tauri/resources/bambu-ca
curl -fsSL -o src-tauri/resources/bambu-ca/bambu-printer-ca.pem \
  https://raw.githubusercontent.com/bambulab/BambuStudio/f75448910c279287556ff8347606c87138c22347/resources/cert/printer.cer
HAB=https://raw.githubusercontent.com/greghesp/ha-bambulab/0e027ff135a6d9265cb756d3e246747954c76722/custom_components/bambu_lab/pybambu/certs
curl -fsSL -o src-tauri/resources/bambu-ca/bbl-device-ca-n6-v2.pem   $HAB/bambu_x2c_260425.cert
curl -fsSL -o src-tauri/resources/bambu-ca/bbl-device-ca-n7-v2.pem   $HAB/bambu_p2s_250626.cert
curl -fsSL -o src-tauri/resources/bambu-ca/bbl-device-ca-o1c2-v2.pem $HAB/bambu_h2c_251122.cert
shasum -a 256 src-tauri/resources/bambu-ca/*.pem
```

Expected:

```
36f2bcee347ec7adce719b5fd350099591a4d3d0ec4e039c7019890d78e152a0  src-tauri/resources/bambu-ca/bambu-printer-ca.pem
9f0fa015ab75ad406ff22ed770f9168c40ba92889fc6ac7f388eb79bd7bf003c  src-tauri/resources/bambu-ca/bbl-device-ca-n6-v2.pem
d08f42719d1ec04c0f8e7d32a208f96fedbdfdc12ae9ca6e63bb13000397d60d  src-tauri/resources/bambu-ca/bbl-device-ca-n7-v2.pem
60a45afe6e1ce9bc031e97422da409d9e2e2fcd6f58650a776933cd8bcf8de90  src-tauri/resources/bambu-ca/bbl-device-ca-o1c2-v2.pem
```

OpenBambuAPI publishes the same bundle as `examples/ca_cert.pem`; it differs only by a trailing newline. If the network is unavailable, write the PEM text below instead. The file hash then depends on the trailing newline, so check the certificates themselves:

```bash
for f in src-tauri/resources/bambu-ca/*.pem; do
  openssl crl2pkcs7 -nocrl -certfile "$f" | openssl pkcs7 -print_certs | \
  awk '/BEGIN/{c=""} {c=c $0 "\n"} /END/{print c > "/tmp/bmca.pem"; close("/tmp/bmca.pem"); system("openssl x509 -in /tmp/bmca.pem -noout -fingerprint -sha256 -subject")}'
done
```

Expected SHA-256 fingerprints (colons omitted):
- `bambu-printer-ca.pem`:
  - `E98F19578B3F124ACE6B8A247FFEDA52DC99C89FD4E7D20C82829977B7F33502`: BBL CA2 RSA (self-signed)
  - `1F99D846718C8D333AC319CB8E70919AB63236090FBADDE909BA4A6F0867A683`: BBL CA2 ECC (self-signed)
  - `3D13019B45A4FA8A6CB8DC0FD7BECBB19BB2BF0C2FCB9434A9345FA58680DA0B`: BBL CA2 RSA (by BBL CA)
  - `E898483224F9092F575D627433BB21ED9D125A77E3635B0DA6FC18FE850A02D7`: BBL CA2 ECC (by BBL CA)
  - `030BCA81CECE18B7EFF3CFD2B75D09D3EFCA893BC069609E37FA04257FE4D840`: BBL CA
- `bbl-device-ca-n6-v2.pem`: `68BF3182BC32D5A5454F864928ABAA291941F6D5AC6A86A0E5AD6ECFE2D5477B`
- `bbl-device-ca-n7-v2.pem`: `6A854D546ED6E3CE7A2112657E53F4555374413E6F5CB55F995D1DDD7C06DC69`
- `bbl-device-ca-o1c2-v2.pem`: `CE667B6C001D26C34ED97333FB6099699409FC3D4E12429C37BAEBB05C46C60A`

`src-tauri/resources/bambu-ca/bambu-printer-ca.pem`:

```
-----BEGIN CERTIFICATE-----
MIIFfzCCA2egAwIBAgIUXtzR6tRiL/RHBRXOoyFU0+XrliowDQYJKoZIhvcNAQEL
BQAwRjELMAkGA1UEBhMCQ04xITAfBgNVBAoMGEJCTCBUZWNobm9sb2dpZXMgQ28u
IEx0ZDEUMBIGA1UEAwwLQkJMIENBMiBSU0EwIBcNMjUwNjE3MDEzODA4WhgPMjA1
MDA2MTcwMTM4MDhaMEYxCzAJBgNVBAYTAkNOMSEwHwYDVQQKDBhCQkwgVGVjaG5v
bG9naWVzIENvLiBMdGQxFDASBgNVBAMMC0JCTCBDQTIgUlNBMIICIjANBgkqhkiG
9w0BAQEFAAOCAg8AMIICCgKCAgEAo4550G4c42gTKzQqixwKT089RizIdZpyOcGA
679rPaOdWsMqVwnYPP2FpMqXKkjFbedE+SpGloi2NKCuiPNVRbq9PHOOZwTs7YLo
bOwf53FJuO6vRFpzFfX1tlc9zlFqJvZnYO9NgHpMysidocWcgrDN/SIDywgPB5CV
bYg3Vvzua9fwZx9e5KT9xd5IpTqdTrWS47jQOVKLhdQCbJFIlMrblOwLBAx+fHok
wqh6tkI6Ktuyyjw8Dysebi1ndWjKtZ2mW47r8xZ/J+z3EZqcyJMY6MRtx/zb1jBF
uHtkjrb5Kv1DMzSKlkaNJIbvC+Mk+hI97W+SjLSRuIdC7+oJUzWaSzgu9cjXCVfm
q8t4IL/35hP69PK95LgLectIrP96CYAT/aVMG19FrFW0QWEyfT+kzG4jkumfPbHq
Y2nNkEN0+tjj3h4WdzrWgQEojK/lhfcRFVkts74+aZoMpQP+vmL17CKmSzXk5o/e
K21xgxJdzMbdztfTpibiXk0abfOpN+1VR+3NYa+bROAKNyGaReEGsyW2bjcjNx51
5Vqzj3SVxhMSp5vfF9E4A1jE99M/l9jQDM6RzkT0lMccGAd5tUSdNvDlrqtQaQiK
v/ZsXPgXLTWfOpvaLNEgwdMgZMuhjpkwvAZyoYfeF9kyydjDh7bvrX//cz/VopAU
lxUtQtMCAwEAAaNjMGEwHQYDVR0OBBYEFNVJgQad1sNTN0jxVkwbJ/XM1an1MB8G
A1UdIwQYMBaAFNVJgQad1sNTN0jxVkwbJ/XM1an1MA8GA1UdEwEB/wQFMAMBAf8w
DgYDVR0PAQH/BAQDAgEGMA0GCSqGSIb3DQEBCwUAA4ICAQBFZDKMJfp/N4gBeFHh
MiFehaUyMS6e9mzrTfMLJLJoj6Jopa9V9jIfcCEBGZuRThqFcATV+UdFHSINpUcH
upcCYnazTRC4dn1hnxnQ1ojQcHxdGp9xGw/YclAKD97d8bPShfBMT1to9zbMK7T5
L8zgqg01YIOKjQk0Hcd0+0iUr6m8zQ5P8Rl3QXqAyeWgqmYQrrjTWwPsgdfHNXKX
vDrx7/cqry5lKU802hUplKMBxelv4W8407Ytj1lfJOwvxqxxsFU5jSwcUG3zo2vk
QtjRs8m5BKup5K1OPYkkPu7Ld89X0XpU073/dNDG11uxb1eDKrtNP6vZuZjNE2Pq
8HCoI1EtP+ItyqtUMvHi6Z2zsmlA25broVioeUKxjlIecpQ9JR/FhDu9CWNF/nDW
LSORNaMMzgsMSzI+HCiUhqN+qMIvVP6rzGTJzwqz/lc5Lf+ZPCnGA9WJTT4uPIhf
ufbZmnUJ35WuWKHxovDsqBh88zQ9sZ+ei4Hi4vVzOhUgfG3aLoSQEYqRoqaboANh
wCwzyuW2Rv54u5QSBbd6Gx1OpvsWmLPWd2/iL2kISl5wfmLGVydvSJa+rbOfuAy7
ycVQacVDQCAnbhoVrQy7+454QsKSW3ZV6BcyRrorewCyCYgd7nyxflxHZTBEykXX
haGNe/KFNvJBMOIuIUzknRRmiQ==
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIIB8zCCAZmgAwIBAgIUe61jGQ4RzIC8k+sNuqbI/CaNqPIwCgYIKoZIzj0EAwIw
RjELMAkGA1UEBhMCQ04xITAfBgNVBAoMGEJCTCBUZWNobm9sb2dpZXMgQ28uIEx0
ZDEUMBIGA1UEAwwLQkJMIENBMiBFQ0MwIBcNMjUwNjE3MDEzODM1WhgPMjA1MDA2
MTcwMTM4MzVaMEYxCzAJBgNVBAYTAkNOMSEwHwYDVQQKDBhCQkwgVGVjaG5vbG9n
aWVzIENvLiBMdGQxFDASBgNVBAMMC0JCTCBDQTIgRUNDMFkwEwYHKoZIzj0CAQYI
KoZIzj0DAQcDQgAEpKTF7wRSty4DXpGJzgCPwRh8ghLlxUC3qJbyEgLqTvJgbiwY
APPHK7kVbVmerkqhHOT4QeWRlTG3dOQGLA2VpaNjMGEwHQYDVR0OBBYEFKuRpsjY
REOyIKH7HwOE6jhGBd6NMB8GA1UdIwQYMBaAFKuRpsjYREOyIKH7HwOE6jhGBd6N
MA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0PAQH/BAQDAgEGMAoGCCqGSM49BAMCA0gA
MEUCIErBiUm3VdtP3rz4kb8aLpI5p+BzL7M9vElBGWWJxpHMAiEA3r5tJWVGwuxi
YCrB1c40KYFRFyahGrhOJZAj/YhRdnU=
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIIEeTCCA2GgAwIBAgIUOq+lNIaC2xsswkFqj5JPyVBl45cwDQYJKoZIhvcNAQEL
BQAwQjELMAkGA1UEBhMCQ04xIjAgBgNVBAoMGUJCTCBUZWNobm9sb2dpZXMgQ28u
LCBMdGQxDzANBgNVBAMMBkJCTCBDQTAeFw0yNTA2MTcwMjAxMjdaFw0zNTA2MTUw
MjAxMjdaMEYxCzAJBgNVBAYTAkNOMSEwHwYDVQQKDBhCQkwgVGVjaG5vbG9naWVz
IENvLiBMdGQxFDASBgNVBAMMC0JCTCBDQTIgUlNBMIICIjANBgkqhkiG9w0BAQEF
AAOCAg8AMIICCgKCAgEAo4550G4c42gTKzQqixwKT089RizIdZpyOcGA679rPaOd
WsMqVwnYPP2FpMqXKkjFbedE+SpGloi2NKCuiPNVRbq9PHOOZwTs7YLobOwf53FJ
uO6vRFpzFfX1tlc9zlFqJvZnYO9NgHpMysidocWcgrDN/SIDywgPB5CVbYg3Vvzu
a9fwZx9e5KT9xd5IpTqdTrWS47jQOVKLhdQCbJFIlMrblOwLBAx+fHokwqh6tkI6
Ktuyyjw8Dysebi1ndWjKtZ2mW47r8xZ/J+z3EZqcyJMY6MRtx/zb1jBFuHtkjrb5
Kv1DMzSKlkaNJIbvC+Mk+hI97W+SjLSRuIdC7+oJUzWaSzgu9cjXCVfmq8t4IL/3
5hP69PK95LgLectIrP96CYAT/aVMG19FrFW0QWEyfT+kzG4jkumfPbHqY2nNkEN0
+tjj3h4WdzrWgQEojK/lhfcRFVkts74+aZoMpQP+vmL17CKmSzXk5o/eK21xgxJd
zMbdztfTpibiXk0abfOpN+1VR+3NYa+bROAKNyGaReEGsyW2bjcjNx515Vqzj3SV
xhMSp5vfF9E4A1jE99M/l9jQDM6RzkT0lMccGAd5tUSdNvDlrqtQaQiKv/ZsXPgX
LTWfOpvaLNEgwdMgZMuhjpkwvAZyoYfeF9kyydjDh7bvrX//cz/VopAUlxUtQtMC
AwEAAaNjMGEwDwYDVR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8EBAMCAQYwHQYDVR0O
BBYEFNVJgQad1sNTN0jxVkwbJ/XM1an1MB8GA1UdIwQYMBaAFI80QmjcZ06PxCKe
xXxJ5avdRL4eMA0GCSqGSIb3DQEBCwUAA4IBAQAvS8tyfagaGsFf9YncA2ko/Na5
9BVF+8TlUo+32oznwIVpS1AhSgLP6rNVekXNFKbuP5htudLQ17ZRBJI/UMVyYEDq
IN7xv7Zj+zJwF6W6haYrjb2Vk8igw1XvNULZfvVNNKIkvJUiVqEslWrC+k74crk/
Wv8ChVf+zqvfIN6LV3esaGRL02J3AprQGb7DDhR1EefQMScDkNpGJMUmvCmfknrl
iK8qgvQN1SWO7JRf6fNKHsN1ZQvyP0pgLWxpT3V0/0/WttqX3cMGuJF+jVUzm/Nh
xYhFewG8vc3KzTjnwQApMA6CW554FOJWFyOD2jn5yJLT3Vue+aYDQRp4bKMx
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIICrjCCAZagAwIBAgIUOq+lNIaC2xsswkFqj5JPyVBl45gwDQYJKoZIhvcNAQEL
BQAwQjELMAkGA1UEBhMCQ04xIjAgBgNVBAoMGUJCTCBUZWNobm9sb2dpZXMgQ28u
LCBMdGQxDzANBgNVBAMMBkJCTCBDQTAeFw0yNTA2MTcwMjAxNDdaFw0zNTA2MTUw
MjAxNDdaMEYxCzAJBgNVBAYTAkNOMSEwHwYDVQQKDBhCQkwgVGVjaG5vbG9naWVz
IENvLiBMdGQxFDASBgNVBAMMC0JCTCBDQTIgRUNDMFkwEwYHKoZIzj0CAQYIKoZI
zj0DAQcDQgAEpKTF7wRSty4DXpGJzgCPwRh8ghLlxUC3qJbyEgLqTvJgbiwYAPPH
K7kVbVmerkqhHOT4QeWRlTG3dOQGLA2VpaNjMGEwDwYDVR0TAQH/BAUwAwEB/zAO
BgNVHQ8BAf8EBAMCAQYwHQYDVR0OBBYEFKuRpsjYREOyIKH7HwOE6jhGBd6NMB8G
A1UdIwQYMBaAFI80QmjcZ06PxCKexXxJ5avdRL4eMA0GCSqGSIb3DQEBCwUAA4IB
AQCg6PjUSSZV+4bvejcVMvgXmKzfD95osWn0ctnoMBxPDa+m+Gg+BcLT2IlFAe3E
KYMvu4T295WQc92rjKYqW6cirFppng9uEFW2mZLimxaSmutsTftE3sbMVMJ/SLYN
PV7TFv6mcBSIFWXwmBOIpbh4BUcVfONTvdSfIqfyAVxsq4xzc2nc6hPBpAm21Ayj
ToC1ev/TbDJ8VllFZiEVmWWlIP3aNzAm8S2mOpxPB2WnanaZHSrvXLFhstyzwrjD
yO1/isOZ7wtr7rcuTJdEvvvCimOZlkfRhaDoTew9tQ0E2FVpzzSinw02qmQ1xIE9
5/H5ZzJSPkpeAHWEPnKkxg0v
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIIDZTCCAk2gAwIBAgIUV1FckwXElyek1onFnQ9kL7Bk4N8wDQYJKoZIhvcNAQEL
BQAwQjELMAkGA1UEBhMCQ04xIjAgBgNVBAoMGUJCTCBUZWNobm9sb2dpZXMgQ28u
LCBMdGQxDzANBgNVBAMMBkJCTCBDQTAeFw0yMjA0MDQwMzQyMTFaFw0zMjA0MDEw
MzQyMTFaMEIxCzAJBgNVBAYTAkNOMSIwIAYDVQQKDBlCQkwgVGVjaG5vbG9naWVz
IENvLiwgTHRkMQ8wDQYDVQQDDAZCQkwgQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IB
DwAwggEKAoIBAQDL3pnDdxGOk5Z6vugiT4dpM0ju+3Xatxz09UY7mbj4tkIdby4H
oeEdiYSZjc5LJngJuCHwtEbBJt1BriRdSVrF6M9D2UaBDyamEo0dxwSaVxZiDVWC
eeCPdELpFZdEhSNTaT4O7zgvcnFsfHMa/0vMAkvE7i0qp3mjEzYLfz60axcDoJLk
p7n6xKXI+cJbA4IlToFjpSldPmC+ynOo7YAOsXt7AYKY6Glz0BwUVzSJxU+/+VFy
/QrmYGNwlrQtdREHeRi0SNK32x1+bOndfJP0sojuIrDjKsdCLye5CSZIvqnbowwW
1jRwZgTBR29Zp2nzCoxJYcU9TSQp/4KZuWNVAgMBAAGjUzBRMB0GA1UdDgQWBBSP
NEJo3GdOj8QinsV8SeWr3US+HjAfBgNVHSMEGDAWgBSPNEJo3GdOj8QinsV8SeWr
3US+HjAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQABlBIT5ZeG
fgcK1LOh1CN9sTzxMCLbtTPFF1NGGA13mApu6j1h5YELbSKcUqfXzMnVeAb06Htu
3CoCoe+wj7LONTFO++vBm2/if6Jt/DUw1CAEcNyqeh6ES0NX8LJRVSe0qdTxPJuA
BdOoo96iX89rRPoxeed1cpq5hZwbeka3+CJGV76itWp35Up5rmmUqrlyQOr/Wax6
itosIzG0MfhgUzU51A2P/hSnD3NDMXv+wUY/AvqgIL7u7fbDKnku1GzEKIkfH8hm
Rs6d8SCU89xyrwzQ0PR853irHas3WrHVqab3P+qNwR0YirL0Qk7Xt/q3O1griNg2
Blbjg3obpHo9
-----END CERTIFICATE-----
```

`src-tauri/resources/bambu-ca/bbl-device-ca-n6-v2.pem`:

```
-----BEGIN CERTIFICATE-----
MIIFiDCCA3CgAwIBAgIUaOxODRrxHIIZlHAod5eOwyG3BJgwDQYJKoZIhvcNAQEL
BQAwRjELMAkGA1UEBhMCQ04xITAfBgNVBAoMGEJCTCBUZWNobm9sb2dpZXMgQ28u
IEx0ZDEUMBIGA1UEAwwLQkJMIENBMiBSU0EwHhcNMjUwNjI2MDgyOTA5WhcNNDAw
NjI2MDgyOTA5WjBOMQswCQYDVQQGEwJDTjEhMB8GA1UECgwYQkJMIFRlY2hub2xv
Z2llcyBDby4gTHRkMRwwGgYDVQQDDBNCQkwgRGV2aWNlIENBIE42LVYyMIICIjAN
BgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEA1NthATzLt/4+0iPVymkP9vK5zHWI
qBCG928Wn5tmp+aTf5KohduZeQ9n4z/fmo89SuJ0fEWQE1zCl9Bxl2WoxsykKO0y
/kpSwqosZW9GT49sBNAyQ4RRz/GgJMDkfIYEIyQmuMMu73JU1rtodL0VESmiNd7G
HHIQouZOk5rvsiS9kla/A2zbb6jCPgUmyd+yuWUkDBU93LHAg1vNuiesi5WY0vqp
f2f0cOM5LGDna7vtyfQd0T6kcydSha4iM3rA+gqoXEXChgDmjD1DBDgrAD6B4aZ4
/WohGAvfP/Yi+342gXRTmyzS2Z0yUuxgiEwhXgaiDF1W1gfaURikgG3apYTZ0fIB
bGYsTtXixgve57ZCC+YT7quoYJ8q42TjDiWjK34DRg31EXE7HNim2Z2wXxUM8cWL
EDhPzkALJwNMu3YNDQSWelw4bY3aBLT3V2aycKNelped+KQ1PFo1KS5hNDRHt36Z
m85X8B9TqTNRFQ9I6UBmJ2iJJV2wNGD6ESng8nyWYJqW+3+uq9hEJR5n7RS8mFXR
CUP5iiY0G9RSjE4b7NlTa+8ecP5EbgYkQCcqh35wxIOCh3xZ/GDkX0NMpLDZNbHT
30uDFkX3fAtKcxQeYO+72U0MP6oOkVzmfSRRDKEXoStALOSoHkQ2wE92D3GMfBe8
WRRhHH0DBIQgJEsCAwEAAaNmMGQwEgYDVR0TAQH/BAgwBgEB/wIBADAOBgNVHQ8B
Af8EBAMCAQYwHQYDVR0OBBYEFJdo0JbktDSn/d8vSxGDWWRnYrQTMB8GA1UdIwQY
MBaAFNVJgQad1sNTN0jxVkwbJ/XM1an1MA0GCSqGSIb3DQEBCwUAA4ICAQCEBArn
sJQEaDwW4CQQmCFURzSvuP1xTpnzrIaR34EEaQJGkKMP6r4+ejEzhnE/CGeSouS3
C2/AR05jjRcG/UijvyvPYdKNBaGdsuQjUZ2+mo6MMZ6AMsRcsqfTObv6p1/Punrr
zxtzCfd9r+RogKR6XbAGPqKzh+y+Y1vwiJ6VlpZxWauWePSanqrT88JNd7c/+D4n
zGzX7suxPCjsVoi3NHF6lcFL2iO0fH/dmWULsLx/8AqKd8ba4QUZXQ3xJMQ7+KC0
bc1D2NJuBxjpPEGTnZ4mZ3e+eqsbbkUCOsTiWQBwMM/KN5zCiw8mSQ5fYxVBJGNr
6Avz2GrUUb/b++T1AftO+osEHR8RUHcCPzHC4z16dwnjnb2oaC7kXmz9omn/qTRI
N7xhExBp/Xt+QQEV0992i0JrVL4Ghg/9Ip6T813h++str/caUv8kUPVdlPSUG0Z0
bWJu6wisQq+Lii5MFlGicMz8Xz1lLELaqJQRq9f71/tccVEpxqhifi8TR+efW9gv
JV3bYCRf2qdxcsbFfIDygZJjIdnvWHnhq19QGCHk4cW41hXza0AlDQ5gcBbXhKnm
TgGz+rNXmeAor/NfDfL5XU20miQGjHepBCAJro64ofkntnWd1vyePJJnSjqEn4AG
kvkHW8vmTffkFHMWC/YoXRXklNlv79Z4+Rywrw==
-----END CERTIFICATE-----
```

`src-tauri/resources/bambu-ca/bbl-device-ca-n7-v2.pem`:

```
-----BEGIN CERTIFICATE-----
MIIFiDCCA3CgAwIBAgIUaOxODRrxHIIZlHAod5eOwyG3BJkwDQYJKoZIhvcNAQEL
BQAwRjELMAkGA1UEBhMCQ04xITAfBgNVBAoMGEJCTCBUZWNobm9sb2dpZXMgQ28u
IEx0ZDEUMBIGA1UEAwwLQkJMIENBMiBSU0EwHhcNMjUwNjI2MDgzMDU0WhcNNDAw
NjI2MDgzMDU0WjBOMQswCQYDVQQGEwJDTjEhMB8GA1UECgwYQkJMIFRlY2hub2xv
Z2llcyBDby4gTHRkMRwwGgYDVQQDDBNCQkwgRGV2aWNlIENBIE43LVYyMIICIjAN
BgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEA1q/fVvmynVm4D1QtzktdXv5SknS+
v/VuCM0odWHaQ8FhliXuaFUD5sHtN6gSUDU15KdcvfeS850SPd79F/N48AIWFhDq
Za0nJnk2+Gv79ITkISaa8lBbpqwvDHd/k8beDQLN72pZFMWcekhCTl12B6vPgxxw
PQtsqaFjpA0Ye+a/NkNtEPBEt0ZUlv/2H4masB/YaVf3Pfkmcsz8toGAYQCx99os
Gz9S6OJvF2WYDrhQwGXCT9Ebeqhdf2A8bTRqMXh2Eusfxs8Vwnex+HMZ0fGD24MK
wxMvRJJ0ThwPRmEYDiR5EhoPuWdyFmZ+Cazg7AeNLo7FjTetsHNBuo61FSolrP+X
rzRkdbJrxa2vbuyAApFK71WeK98MlUCO5b4mkMkenBDVLqSIyw5Q/aAB4PqDscAs
ixlOgE91fUlA3p0ZN3BK5BTi/oNue2v6/+krqYV5gBUcnx4ixYz5joaokCFrWM/O
s7Fjo8iMVv/XGwH/UDa1VGV9wN5FU6TyuUYSl00SGpEt4/1jAqriqBU3B1dC+BhZ
8huhCW19xI2kWsZDGcmEhxnoZfYmpD4AdMYoF/daUVPP3F4CSrjqLzXABUHBOMZJ
g5KAEb92j34ZycVnQfOB4SB1hKSoHcuT3UAjmR9FE13apfwwvynoCJzohW1szwKN
5YzBBv/szamiQS0CAwEAAaNmMGQwEgYDVR0TAQH/BAgwBgEB/wIBADAOBgNVHQ8B
Af8EBAMCAQYwHQYDVR0OBBYEFKz7LDkKuUzML/DZxB17wptduym6MB8GA1UdIwQY
MBaAFNVJgQad1sNTN0jxVkwbJ/XM1an1MA0GCSqGSIb3DQEBCwUAA4ICAQBqWtA9
c0ttQ1u/3DHsgOpP2eNZjMCTdK8ee/5szdV4fMR/+ZrF1ao4fP0Oc8qcRnmQsZ1S
Mx727aQm+X87Hh5GU+2aJUfROLjiWEVE3zT8F+wYmojm/rR9Ai0CY3TWDiF3hspQ
0aPeME1ZvMSAFvSeWUdj8RbRMUSTneyPOk6dosNJZd4fpGr0uV2xCsL2Ykw/e6W8
5WzQSDoLbW08wa+N4SuDzto+IxX4Fg+IgQ4fc2x1cWUEN5Q5Mch8dOlJJUFK+jHh
muhAWM3pCcJAR1Fi9cEkkpgHILLonng/xJTzpaX/bvzpyoSLqlvF0Wizr+2CxvHg
lHf80fw6w+TVmnubTPk1JAzEn/KXxQFOXz7zy6MB3luK64C8L7XwlUJk4sCmbD0N
slsUN7s5thgZM+o4zAj1G6KOMH3lcpBw0SiLVOB3qLFANMT0pnKDYMMnTVMKv3CB
GVqbFPAUn2vLScaZG5jiuBsSaIY9WAfGrae02Lp+KYbt2aRQ1SM4llkbdHYbIx53
n3+9BCScDy/3Gy7c3o8avUGI2AKGi+5teciOaLbmzkz2iFWjiUz4CB6Bm1VePRru
UZkPxKr9W3Y20kZwOalGQDrcms+WCpp54zd200CgYF6A6IkgmMtdh1K+lnRMCao5
v09+U4m1fOSvF0We4MPCYt+z/E/br+uZ1pIBcg==
-----END CERTIFICATE-----
```

`src-tauri/resources/bambu-ca/bbl-device-ca-o1c2-v2.pem`:

```
-----BEGIN CERTIFICATE-----
MIIFijCCA3KgAwIBAgIUdrsWybAyHo24kPcb0WlVA2L6L48wDQYJKoZIhvcNAQEL
BQAwRjELMAkGA1UEBhMCQ04xITAfBgNVBAoMGEJCTCBUZWNobm9sb2dpZXMgQ28u
IEx0ZDEUMBIGA1UEAwwLQkJMIENBMiBSU0EwHhcNMjUwNzE1MDYxMDA5WhcNNDAw
NzE1MDYxMDA5WjBQMQswCQYDVQQGEwJDTjEhMB8GA1UECgwYQkJMIFRlY2hub2xv
Z2llcyBDby4gTHRkMR4wHAYDVQQDDBVCQkwgRGV2aWNlIENBIE8xQzItVjIwggIi
MA0GCSqGSIb3DQEBAQUAA4ICDwAwggIKAoICAQCuMS5LueR9hQpmKrQfqXpUp6q9
Ih1MPfWTY3lg9ko5asbDyv6iVJgYGES9BnqiJUVscNXDwO3q+D9ZvnJ9rpaywfWk
O11CU9tLgil8y+7J6DwDhnNYe1lemzDb3h+vSCB0eDllHFPUZBdYWHA1E5tvxSvU
e/6ifG3EiW2AAq+un2RC/U4sJm/0OVWskCuFFZEeIy01tUX13IV/1eW2Ts5x2vn1
XYG1caY9ky3aYG5Zg+i9vJL948gWXlIFVuo24uMGRQ7IVqhXatARAK0V+vbskL8s
jreuGafjEIUjhvNDoXaKQ8FU0K7fK+ldNM8gRS9ruTRvH9xnW1pGDoupwR1oY24e
vODH8EhfYqaGNqXROPCnP/4z8QnjgyYubTJm1v1tLRPn5s/zPkl7/htsLRGW8Ua4
dDeqev4+Jgz7S0uehlm/+x5N9LqUs2TfmpJwvdJzT4+7aKvbJXC1t74vKKjUccjS
Jtg5GpvfrHaDHzMC6hPAvls0EkyRUtih9vpVjsz4DX9kc/C1o/EDI6fd2zBZ+oYA
leefTSoAWIfhbZclBpSbXYYMUED9gjaLadWs6GlKhi7fu1ePNFzuWKYfq+/f+qYB
LvIFHIjhVp1dIGZVfMS3dZMm8oUh5NEoKSFLbj8WpeIGyWt7Voubsz5ua+aVB0hV
OW6NMV8rQdIRa+QDTQIDAQABo2YwZDASBgNVHRMBAf8ECDAGAQH/AgEAMA4GA1Ud
DwEB/wQEAwIBBjAdBgNVHQ4EFgQUewDW+YeXtkKDpIk+lZXUWJHuOYowHwYDVR0j
BBgwFoAU1UmBBp3Ww1M3SPFWTBsn9czVqfUwDQYJKoZIhvcNAQELBQADggIBAEhJ
jmOusM52x3J+0y7IY8ratwxxDADZM6l0VuVzr9LJRRCI8DE3So9BtYNuTyAxMPLy
nhq1rZlw9OXt1iJll3YMicSg8TcZIGQYEPQl125j8KNRwwz8QhSO/+ZtDn7GFOEA
l7I6vTMNVCdvTa1Zr4m4SiklFrokKGWxd3vZrqoMf08kjBarvOUqvmUVBB/aNegK
tiMVe8YG7xxaZY4jcdcptizj07SuQc2Z0fiyvSXcyQDEHkaIscGJYXPegmSecWOL
mLJMoXJwmCE5p7BsipQn3O82q4tLcHSunf3gc8Ys2nJ3BOA+oz36Cgs5SwdJd1zo
08moBYOCB5pjKm2dpttN3uyM8fk30UAhYTt37HUoyClVUHhbcQd0gV2W9d0bX9tv
rYl1EY6xiadElwKTpC6vWDHkBlkc7mWpqzpVkh7NvWuEAN9qM15zzgO0nDEqwDod
2b6Ww5YJ20ugWa+88295nZ0tG/UBgFkCMyhy6KFXRo/MMENmMakZJumr+Exq6KEe
aajPOIgdgJOkZGyFnEtCuC6mvg+zED+T5R+0K1kLXpWxO2Ofwylu+s91E/bOzPdX
rLMRshz4Xw7jJ1tHt+H81BxG3ies08EtBOmcJv1bx+gfNfRChR7KckiSpX8ZxpF/
sekqtMMSz9xFFJxMKGnCgqZucrJAZYyw92e5D0/p
-----END CERTIFICATE-----
```

The PEMs are compiled in with `include_str!`, so `tauri.conf.json` needs no resource entry.

- [ ] **Step 3: Write the failing tests**

Replace `src-tauri/src/printer/mod.rs` with:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod state;
pub mod tls;
```

Create `src-tauri/src/printer/tls.rs` with only the test code for now:

```rust
#[cfg(test)]
pub(crate) mod testpki {
    //! Generated CAs and printer certificates for tests.

    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    };

    pub struct TestCa {
        pub pem: String,
        params: CertificateParams,
        key: KeyPair,
    }

    pub struct TestLeaf {
        pub cert_der: Vec<u8>,
        pub key_pem: String,
    }

    impl TestCa {
        pub fn new(name: &str) -> Self {
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.distinguished_name.push(DnType::CommonName, name);
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
            let key = KeyPair::generate().unwrap();
            let cert = params.self_signed(&key).unwrap();
            Self {
                pem: cert.pem(),
                params,
                key,
            }
        }

        /// A printer certificate: CN = serial, no DNS or IP names.
        pub fn leaf(&self, serial: &str) -> TestLeaf {
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.distinguished_name.push(DnType::CommonName, serial);
            let key = KeyPair::generate().unwrap();
            let issuer = Issuer::from_params(&self.params, &self.key);
            let cert = params.signed_by(&key, &issuer).unwrap();
            TestLeaf {
                cert_der: cert.der().to_vec(),
                key_pem: key.serialize_pem(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testpki::TestCa;
    use super::*;

    const SERIAL: &str = "0948AB000000001";

    fn verify(v: &PrinterCertVerifier, leaf_der: &[u8]) -> Result<ServerCertVerified, Error> {
        v.verify_server_cert(
            &CertificateDer::from(leaf_der.to_vec()),
            &[],
            &ServerName::try_from("192.168.1.20").unwrap(),
            &[],
            UnixTime::now(),
        )
    }

    #[test]
    fn accepts_a_chain_from_a_trusted_ca_with_the_right_cn() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_ok());
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn rejects_a_certificate_for_another_serial() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.leaf("01P00A000000002");
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::WrongSerial {
                presented: "01P00A000000002".into()
            })
        );
    }

    #[test]
    fn rejects_an_unknown_ca_and_reports_the_fingerprint() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Somebody Else");
        let leaf = other.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v =
            PrinterCertVerifier::with_trust(&trusted.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::Untrusted {
                fingerprint: fingerprint(&leaf.cert_der)
            })
        );
    }

    #[test]
    fn accepts_the_pinned_certificate_from_an_unknown_ca() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Newer Bambu CA");
        let leaf = other.leaf(SERIAL);
        let pin = fingerprint(&leaf.cert_der).to_lowercase();
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            SERIAL,
            Some(&pin),
            RejectionSlot::default(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_ok());
    }

    #[test]
    fn rejects_a_changed_certificate_despite_a_pin() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Newer Bambu CA");
        let old_leaf = other.leaf(SERIAL);
        let new_leaf = other.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            SERIAL,
            Some(&fingerprint(&old_leaf.cert_der)),
            slot.clone(),
        )
        .unwrap();
        assert!(verify(&v, &new_leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::Untrusted {
                fingerprint: fingerprint(&new_leaf.cert_der)
            })
        );
    }

    #[test]
    fn a_pin_does_not_skip_the_cn_check() {
        let trusted = TestCa::new("Test Printer CA");
        let leaf = TestCa::new("Other").leaf("SOMEONE-ELSE");
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            SERIAL,
            Some(&fingerprint(&leaf.cert_der)),
            RejectionSlot::default(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
    }

    #[test]
    fn the_bundled_bambu_cas_load() {
        let v = PrinterCertVerifier::bambu(SERIAL, None, RejectionSlot::default()).unwrap();
        assert!(v.roots.len() >= 3, "BBL CA, BBL CA2 RSA, BBL CA2 ECC");
        assert_eq!(v.extra_intermediates.len(), 3);
        for der in &v.extra_intermediates {
            let cn = leaf_common_name(der).unwrap();
            assert!(cn.starts_with("BBL Device CA"), "{cn}");
        }
    }

    #[test]
    fn fingerprints_are_colon_separated_uppercase_sha256() {
        let fp = fingerprint(b"abc");
        assert_eq!(
            fp,
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
        assert_eq!(
            normalize_fingerprint("ba:78 16"),
            normalize_fingerprint("BA7816")
        );
    }

    #[test]
    fn the_client_config_builds_with_the_ring_provider() {
        let v = PrinterCertVerifier::bambu(SERIAL, None, RejectionSlot::default()).unwrap();
        assert!(client_config(Arc::new(v)).is_ok());
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib printer::tls`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `PrinterCertVerifier` and the other items not written yet.

- [ ] **Step 5: Write the implementation**

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/tls.rs`:

```rust
//! TLS verification for the printer's MQTT broker.
//!
//! A printer presents a certificate whose CN is its serial number, not its
//! IP address, so the usual hostname check is replaced by a CN check. The
//! chain must lead to one of Bambu's printer CAs, bundled from
//! `src-tauri/resources/bambu-ca/`. When it doesn't (for example a model
//! with a CA we don't ship), the user may pin that exact certificate by its
//! SHA-256 fingerprint. The CN check applies either way. There is no option
//! that skips verification.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};

/// Bambu's printer CA bundle: BBL CA, BBL CA2 RSA and BBL CA2 ECC.
/// Source: bambulab/BambuStudio `resources/cert/printer.cer`.
const BAMBU_PRINTER_CA_PEM: &str = include_str!("../../resources/bambu-ca/bambu-printer-ca.pem");

/// Per-model intermediate CAs some printers don't send in their chain. All
/// are signed by BBL CA2 RSA, so they can't widen trust beyond Bambu's CAs.
/// Source: greghesp/ha-bambulab `pybambu/certs/`.
const BAMBU_DEVICE_CA_PEMS: &[&str] = &[
    include_str!("../../resources/bambu-ca/bbl-device-ca-n6-v2.pem"),
    include_str!("../../resources/bambu-ca/bbl-device-ca-n7-v2.pem"),
    include_str!("../../resources/bambu-ca/bbl-device-ca-o1c2-v2.pem"),
];

/// The only crypto provider BambuMate's printer code uses. Passed to every
/// config explicitly, so no process-wide default provider is needed.
pub fn crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Why the verifier refused a printer's certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// Not signed by a bundled CA and not the pinned certificate.
    Untrusted { fingerprint: String },
    /// The certificate belongs to a different printer.
    WrongSerial { presented: String },
}

/// Where the verifier records its last rejection, so the client can tell a
/// certificate problem apart from a network one after the handshake fails.
#[derive(Debug, Clone, Default)]
pub struct RejectionSlot(Arc<Mutex<Option<Rejection>>>);

impl RejectionSlot {
    fn set(&self, r: Rejection) {
        *self.0.lock().unwrap() = Some(r);
    }
    /// Returns and clears the last rejection.
    pub fn take(&self) -> Option<Rejection> {
        self.0.lock().unwrap().take()
    }
}

/// `AB:CD:…`, uppercase: the SHA-256 of the DER certificate.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Uppercase hex with separators removed, for comparing fingerprints.
pub fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The subject CN of a DER certificate.
pub fn leaf_common_name(der: &[u8]) -> Option<String> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let cn = cert.subject().iter_common_name().next()?;
    cn.as_str().ok().map(str::to_string)
}

fn certs_from_pem(pem: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("bad certificate PEM: {e}"))
}

#[derive(Debug)]
pub struct PrinterCertVerifier {
    roots: RootCertStore,
    extra_intermediates: Vec<CertificateDer<'static>>,
    serial: String,
    pinned: Option<String>,
    provider: Arc<CryptoProvider>,
    rejection: RejectionSlot,
}

impl PrinterCertVerifier {
    /// A verifier that trusts Bambu's bundled printer CAs.
    pub fn bambu(
        serial: &str,
        pinned_fingerprint: Option<&str>,
        rejection: RejectionSlot,
    ) -> Result<Self, String> {
        Self::with_trust(
            BAMBU_PRINTER_CA_PEM,
            BAMBU_DEVICE_CA_PEMS,
            serial,
            pinned_fingerprint,
            rejection,
        )
    }

    /// A verifier over explicit CAs. Tests use it with a generated CA.
    pub fn with_trust(
        roots_pem: &str,
        intermediates_pem: &[&str],
        serial: &str,
        pinned_fingerprint: Option<&str>,
        rejection: RejectionSlot,
    ) -> Result<Self, String> {
        let mut roots = RootCertStore::empty();
        let (added, _ignored) = roots.add_parsable_certificates(certs_from_pem(roots_pem)?);
        if added == 0 {
            return Err("no usable CA certificate".into());
        }
        let mut extra_intermediates = Vec::new();
        for pem in intermediates_pem {
            extra_intermediates.extend(certs_from_pem(pem)?);
        }
        Ok(Self {
            roots,
            extra_intermediates,
            serial: serial.trim().to_string(),
            pinned: pinned_fingerprint
                .map(normalize_fingerprint)
                .filter(|p| !p.is_empty()),
            provider: crypto_provider(),
            rejection,
        })
    }
}

impl ServerCertVerifier for PrinterCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let presented = leaf_common_name(end_entity).unwrap_or_default();
        if presented != self.serial {
            self.rejection.set(Rejection::WrongSerial { presented });
            return Err(Error::InvalidCertificate(CertificateError::NotValidForName));
        }
        let parsed = ParsedCertificate::try_from(end_entity)?;
        let mut chain: Vec<CertificateDer<'_>> = intermediates.to_vec();
        chain.extend(self.extra_intermediates.iter().cloned());
        let chained = rustls::client::verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &self.roots,
            &chain,
            now,
            self.provider.signature_verification_algorithms.all,
        );
        match chained {
            Ok(()) => Ok(ServerCertVerified::assertion()),
            Err(err) => {
                let fp = fingerprint(end_entity);
                if self.pinned.as_deref() == Some(normalize_fingerprint(&fp).as_str()) {
                    return Ok(ServerCertVerified::assertion());
                }
                self.rejection.set(Rejection::Untrusted { fingerprint: fp });
                Err(err)
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The client config for one printer. TLS 1.2 only: some Bambu firmware
/// never answers a TLS 1.3 ClientHello (ha-bambulab caps it the same way).
pub fn client_config(verifier: Arc<PrinterCertVerifier>) -> Result<Arc<ClientConfig>, String> {
    let config = ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib printer::tls`
Expected: `test result: ok. 9 passed; 0 failed`. A dead-code warning for `TestLeaf::key_pem` is expected until Task 3 uses it.

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/resources/bambu-ca src-tauri/src/printer/mod.rs src-tauri/src/printer/tls.rs
git commit -m "Verify the printer's TLS certificate against Bambu's CAs or a pinned fingerprint"
```

---

### Task 3: MQTT client with reconnect, tested against an in-process TLS broker

**Files:**
- Create: `src-tauri/src/printer/client.rs`
- Create: `src-tauri/src/printer/testbroker.rs` (test only)
- Modify: `src-tauri/src/printer/mod.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes:
  - Task 1: `state::{parse_report, Report}` and `state::fixtures::{H2D_FULL, GET_VERSION_H2D}`
  - Task 2: `tls::{Rejection, RejectionSlot, PrinterCertVerifier, client_config, fingerprint, crypto_provider}` and `tls::testpki::{TestCa, TestLeaf}`
- Produces (in `crate::printer::client`):
  - `pub const MQTT_PORT: u16 = 8883`, `pub const MQTT_USER: &str = "bblp"`, `pub const READ_ONLY_COMMANDS: &[&str] = &["pushall", "get_version"]`
  - `pub enum ConnectionState { Disconnected, Connecting, Connected, AuthFailed, CertUntrusted { fingerprint: String }, WrongSerial { presented: String }, Unreachable }`:
    - Derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize` with `#[serde(tag = "state", rename_all = "snake_case")]`, so it serializes as `{"state":"cert_untrusted","fingerprint":"…"}`.
    - Has `pub fn needs_user(&self) -> bool`.
  - `pub enum ClientEvent { Connection(ConnectionState), Report(Vec<u8>) }`
  - `pub struct ClientParams { pub host: String, pub port: u16, pub serial: String, pub access_code: String, pub tls: Arc<rustls::ClientConfig>, pub rejection: RejectionSlot }`: `Clone`, with a redacting `Debug`.
  - `pub struct Timing { pub backoff_min: Duration, pub backoff_max: Duration, pub pushall_interval: Duration }`: `Copy`; `Default` is 2 s / 60 s / 300 s.
  - `pub fn report_topic(serial: &str) -> String`, `pub fn request_topic(serial: &str) -> String`, `pub fn pushall_request(seq: u64) -> String`, `pub fn get_version_request(seq: u64) -> String`
  - `pub struct Backoff` with `new(min, max)`, `next_delay(&mut self) -> Duration` and `reset(&mut self)`; `pub struct PushallGate` with `new(interval)` and `try_take(&mut self, now: Instant) -> bool`
  - `pub async fn run(params: ClientParams, events: mpsc::Sender<ClientEvent>, timing: Timing)`
  - `pub struct TestOutcome { pub connection: ConnectionState, pub got_report: bool, pub model: Option<String> }` (`Serialize, Deserialize`)
  - `pub async fn test_connection(params: ClientParams, timing: Timing, wait: Duration) -> TestOutcome`
  - Test only, in `crate::printer::testbroker::FakeBroker`:
    - `start(leaf: &TestLeaf, serial: &str, password: &str, pushall_reply: &str) -> FakeBroker` (async)
    - `.with_version_reply(payload) -> Self`, `.published() -> Vec<(String, Vec<u8>)>`, `.subscriptions() -> Vec<String>`, `.connection_count() -> usize`, `.drop_connections()`
    - `pub addr: SocketAddr`

- [ ] **Step 1: Add the dependencies**

In `src-tauri/Cargo.toml` `[dependencies]`, directly after the `# rustls uses the ring provider…` comment line, add:

```toml
rumqttc = { version = "0.25.1", default-features = false, features = ["use-rustls-no-provider"] }
```

In `[dev-dependencies]`, after `rcgen = "0.14.10"`, add:

```toml
tokio-rustls = { version = "0.26.4", default-features = false, features = ["ring", "tls12", "logging"] }
bytes = "1"
```

`use-rustls-no-provider` is required: rumqttc's `use-rustls` feature turns on `tokio-rustls/default`, which is aws-lc-rs.

- [ ] **Step 2: Add the test broker**

Replace `src-tauri/src/printer/mod.rs` with:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod client;
pub mod state;
#[cfg(test)]
mod testbroker;
pub mod tls;
```

Create `src-tauri/src/printer/testbroker.rs`:

```rust
//! A minimal in-process MQTT 3.1.1 broker over TLS, standing in for a
//! printer in tests. It checks the `bblp` password, acknowledges
//! subscriptions, records every publish, answers `pushall` with a canned
//! report and `get_version` with a canned version reply, and can drop all
//! connections to exercise reconnect.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use rumqttc::{ConnAck, ConnectReturnCode, Packet, Publish, QoS, SubAck, SubscribeReasonCode};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

use super::tls::testpki::TestLeaf;

const MAX_PACKET: usize = 1 << 20;

#[derive(Default)]
struct Shared {
    published: Mutex<Vec<(String, Vec<u8>)>>,
    subscriptions: Mutex<Vec<String>>,
    version_reply: Mutex<Option<Vec<u8>>>,
    connections: AtomicUsize,
}

pub struct FakeBroker {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    kill: watch::Sender<u64>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeBroker {
    fn drop(&mut self) {
        self.accept_task.abort();
        self.kill.send_modify(|g| *g += 1);
    }
}

impl FakeBroker {
    /// Serves `leaf` on 127.0.0.1 with an ephemeral port.
    pub async fn start(leaf: &TestLeaf, serial: &str, password: &str, pushall_reply: &str) -> Self {
        let certs = vec![CertificateDer::from(leaf.cert_der.clone())];
        let key = PrivateKeyDer::from_pem_slice(leaf.key_pem.as_bytes()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(super::tls::crypto_provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared::default());
        let (kill, _) = watch::channel(0u64);

        let report_topic = format!("device/{serial}/report");
        let password = password.to_string();
        let reply = pushall_reply.as_bytes().to_vec();
        let accept_shared = shared.clone();
        let accept_kill = kill.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let shared = accept_shared.clone();
                let mut kill_rx = accept_kill.subscribe();
                let (topic, password, reply) =
                    (report_topic.clone(), password.clone(), reply.clone());
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    tokio::select! {
                        _ = serve(tls, &shared, &topic, &password, &reply) => {}
                        _ = kill_rx.changed() => {}
                    }
                });
            }
        });
        Self {
            addr,
            shared,
            kill,
            accept_task,
        }
    }

    /// Also answer `get_version` with this payload.
    pub fn with_version_reply(self, payload: &str) -> Self {
        *self.shared.version_reply.lock().unwrap() = Some(payload.as_bytes().to_vec());
        self
    }

    pub fn published(&self) -> Vec<(String, Vec<u8>)> {
        self.shared.published.lock().unwrap().clone()
    }

    pub fn subscriptions(&self) -> Vec<String> {
        self.shared.subscriptions.lock().unwrap().clone()
    }

    /// Connections that passed the password check.
    pub fn connection_count(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    /// Closes every open connection, as a printer reboot would.
    pub fn drop_connections(&self) {
        self.kill.send_modify(|g| *g += 1);
    }
}

async fn serve<S>(mut stream: S, shared: &Shared, report_topic: &str, password: &str, reply: &[u8])
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut buf = BytesMut::new();
    let Some(Packet::Connect(connect)) = read_packet(&mut stream, &mut buf).await else {
        return;
    };
    let ok = connect
        .login
        .as_ref()
        .is_some_and(|l| l.username == "bblp" && l.password == password);
    let code = if ok {
        ConnectReturnCode::Success
    } else {
        ConnectReturnCode::NotAuthorized
    };
    if write_packet(&mut stream, Packet::ConnAck(ConnAck::new(code, false)))
        .await
        .is_err()
        || !ok
    {
        return;
    }
    shared.connections.fetch_add(1, Ordering::SeqCst);
    while let Some(packet) = read_packet(&mut stream, &mut buf).await {
        let answer =
            match packet {
                Packet::Subscribe(s) => {
                    let mut subs = shared.subscriptions.lock().unwrap();
                    for f in &s.filters {
                        if !subs.contains(&f.path) {
                            subs.push(f.path.clone());
                        }
                    }
                    let codes = s
                        .filters
                        .iter()
                        .map(|_| SubscribeReasonCode::Success(QoS::AtMostOnce))
                        .collect();
                    Some(Packet::SubAck(SubAck::new(s.pkid, codes)))
                }
                Packet::Publish(p) => {
                    let body = p.payload.to_vec();
                    shared
                        .published
                        .lock()
                        .unwrap()
                        .push((p.topic.clone(), body.clone()));
                    let text = String::from_utf8_lossy(&body);
                    if text.contains("\"pushall\"") {
                        Some(Packet::Publish(Publish::new(
                            report_topic,
                            QoS::AtMostOnce,
                            reply.to_vec(),
                        )))
                    } else if text.contains("\"get_version\"") {
                        shared.version_reply.lock().unwrap().clone().map(|v| {
                            Packet::Publish(Publish::new(report_topic, QoS::AtMostOnce, v))
                        })
                    } else {
                        None
                    }
                }
                Packet::PingReq => Some(Packet::PingResp),
                Packet::Disconnect => return,
                _ => None,
            };
        if let Some(answer) = answer {
            if write_packet(&mut stream, answer).await.is_err() {
                return;
            }
        }
    }
}

async fn read_packet<S: tokio::io::AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut BytesMut,
) -> Option<Packet> {
    loop {
        match Packet::read(buf, MAX_PACKET) {
            Ok(p) => return Some(p),
            Err(rumqttc::Error::InsufficientBytes(_)) => {
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).await.ok()?;
                if n == 0 {
                    return None;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(_) => return None,
        }
    }
}

async fn write_packet<S: tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    packet: Packet,
) -> std::io::Result<()> {
    let mut out = BytesMut::new();
    packet
        .write(&mut out, MAX_PACKET)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    stream.write_all(&out).await?;
    stream.flush().await
}
```

- [ ] **Step 3: Write the failing tests**

Create `src-tauri/src/printer/client.rs` with only the test code for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::fixtures::{GET_VERSION_H2D, H2D_FULL};
    use crate::printer::testbroker::FakeBroker;
    use crate::printer::tls::testpki::TestCa;
    use crate::printer::tls::{client_config, fingerprint, PrinterCertVerifier};

    const SERIAL: &str = "0948AB000000001";
    const CODE: &str = "12345678";

    fn fast() -> Timing {
        Timing {
            backoff_min: Duration::from_millis(50),
            backoff_max: Duration::from_millis(200),
            pushall_interval: Duration::from_secs(300),
        }
    }

    fn params(ca: &TestCa, broker: &FakeBroker, code: &str, pin: Option<&str>) -> ClientParams {
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, pin, rejection.clone()).unwrap();
        ClientParams {
            host: "127.0.0.1".into(),
            port: broker.addr.port(),
            serial: SERIAL.into(),
            access_code: code.into(),
            tls: client_config(Arc::new(verifier)).unwrap(),
            rejection,
        }
    }

    async fn next_state(rx: &mut mpsc::Receiver<ClientEvent>) -> ConnectionState {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ClientEvent::Connection(ConnectionState::Connecting))) => {}
                Ok(Some(ClientEvent::Connection(s))) => return s,
                Ok(Some(ClientEvent::Report(_))) => {}
                other => panic!("no state change: {other:?}"),
            }
        }
    }

    async fn next_report(rx: &mut mpsc::Receiver<ClientEvent>) -> Vec<u8> {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ClientEvent::Report(b))) => return b,
                Ok(Some(_)) => {}
                other => panic!("no report: {other:?}"),
            }
        }
    }

    #[test]
    fn requests_are_the_documented_read_only_json() {
        let v: serde_json::Value = serde_json::from_str(&pushall_request(7)).unwrap();
        assert_eq!(
            v,
            json!({"pushing":{"sequence_id":"7","command":"pushall","version":1,"push_target":1}})
        );
        let v: serde_json::Value = serde_json::from_str(&get_version_request(8)).unwrap();
        assert_eq!(
            v,
            json!({"info":{"sequence_id":"8","command":"get_version"}})
        );
    }

    #[test]
    fn backoff_doubles_from_two_seconds_to_a_minute() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        let seen: Vec<u64> = (0..7).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(seen, vec![2, 4, 8, 16, 32, 60, 60]);
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(2));
    }

    #[test]
    fn pushall_is_allowed_once_per_interval() {
        let mut g = PushallGate::new(Duration::from_secs(300));
        let t0 = Instant::now();
        assert!(g.try_take(t0));
        assert!(!g.try_take(t0 + Duration::from_secs(299)));
        assert!(g.try_take(t0 + Duration::from_secs(300)));
    }

    #[test]
    fn debug_output_never_contains_the_access_code() {
        let ca = TestCa::new("CA");
        let v =
            PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, RejectionSlot::default())
                .unwrap();
        let p = ClientParams {
            host: "10.0.0.2".into(),
            port: MQTT_PORT,
            serial: SERIAL.into(),
            access_code: "SECRET99".into(),
            tls: client_config(Arc::new(v)).unwrap(),
            rejection: RejectionSlot::default(),
        };
        assert!(!format!("{p:?}").contains("SECRET99"));
    }

    #[tokio::test]
    async fn connects_subscribes_requests_and_forwards_reports() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, CODE, None), tx, fast()));

        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        let report = next_report(&mut rx).await;
        assert!(matches!(
            crate::printer::state::parse_report(&report).unwrap(),
            crate::printer::state::Report::Status { full: true, .. }
        ));
        assert_eq!(broker.subscriptions(), vec![report_topic(SERIAL)]);
        let published = broker.published();
        assert!(published
            .iter()
            .all(|(topic, _)| *topic == request_topic(SERIAL)));
        let commands: Vec<String> = published
            .iter()
            .map(|(_, body)| {
                let v: serde_json::Value = serde_json::from_slice(body).unwrap();
                let inner = v.as_object().unwrap().values().next().unwrap();
                inner["command"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(commands, vec!["get_version", "pushall"]);
        assert!(commands
            .iter()
            .all(|c| READ_ONLY_COMMANDS.contains(&c.as_str())));
        task.abort();
    }

    #[tokio::test]
    async fn reconnects_after_the_broker_drops_without_a_second_pushall() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, CODE, None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        next_report(&mut rx).await;

        broker.drop_connections();
        assert_eq!(next_state(&mut rx).await, ConnectionState::Disconnected);
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        assert_eq!(broker.connection_count(), 2);
        // Wait until the second connection's get_version has arrived.
        for _ in 0..50 {
            if broker.published().len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pushalls = broker
            .published()
            .iter()
            .filter(|(_, b)| String::from_utf8_lossy(b).contains("pushall"))
            .count();
        assert_eq!(pushalls, 1, "pushall is limited to once per 5 minutes");
        task.abort();
    }

    #[tokio::test]
    async fn a_wrong_access_code_is_auth_failed() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, "00000000", None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::AuthFailed);
        task.abort();
    }

    #[tokio::test]
    async fn an_unknown_ca_is_cert_untrusted_with_the_fingerprint() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Unknown CA");
        let leaf = other.leaf(SERIAL);
        let broker = FakeBroker::start(&leaf, SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&trusted, &broker, CODE, None), tx, fast()));
        assert_eq!(
            next_state(&mut rx).await,
            ConnectionState::CertUntrusted {
                fingerprint: fingerprint(&leaf.cert_der)
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let mut p = params(&ca, &broker, CODE, None);
        drop(broker);
        // A port nothing listens on.
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        p.port = spare.local_addr().unwrap().port();
        drop(spare);
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(p, tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Unreachable);
        task.abort();
    }

    #[tokio::test]
    async fn test_connection_reports_the_model_and_a_full_report() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        let out = test_connection(
            params(&ca, &broker, CODE, None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(out.connection, ConnectionState::Connected);
        assert!(out.got_report);
        assert_eq!(out.model.as_deref(), Some("H2D"));
    }

    #[tokio::test]
    async fn test_connection_stops_at_a_rejected_code() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let out = test_connection(
            params(&ca, &broker, "wrong", None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(out.connection, ConnectionState::AuthFailed);
        assert!(!out.got_report);
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib printer::client`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `pushall_request` and the other items not written yet.

- [ ] **Step 5: Write the implementation**

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/client.rs`:

```rust
//! The MQTT connection to the printer: TLS, subscribe, the two read
//! requests, and reconnect with backoff.
//!
//! Its only output is `ClientEvent`s: raw report payloads and connection
//! state changes. It publishes nothing but `pushall` and `get_version`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, Event, MqttOptions, Packet, QoS,
    TlsConfiguration, Transport,
};
use rustls::ClientConfig;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

use super::tls::{Rejection, RejectionSlot};

/// The printer's MQTT-over-TLS port.
pub const MQTT_PORT: u16 = 8883;
/// The LAN-mode MQTT user.
pub const MQTT_USER: &str = "bblp";
/// The only commands BambuMate ever publishes.
pub const READ_ONLY_COMMANDS: &[&str] = &["pushall", "get_version"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    AuthFailed,
    CertUntrusted { fingerprint: String },
    WrongSerial { presented: String },
    Unreachable,
}

impl ConnectionState {
    /// A state that retrying won't fix without the user changing something.
    pub fn needs_user(&self) -> bool {
        matches!(
            self,
            Self::AuthFailed | Self::CertUntrusted { .. } | Self::WrongSerial { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    Connection(ConnectionState),
    /// A raw payload from `device/<serial>/report`.
    Report(Vec<u8>),
}

/// What the client needs to reach one printer.
#[derive(Clone)]
pub struct ClientParams {
    pub host: String,
    pub port: u16,
    pub serial: String,
    pub access_code: String,
    pub tls: Arc<ClientConfig>,
    pub rejection: RejectionSlot,
}

// Written by hand so the access code can never reach a log line.
impl std::fmt::Debug for ClientParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("serial", &self.serial)
            .field("access_code", &"<redacted>")
            .finish()
    }
}

/// Reconnect and request pacing. `Timing::default()` is the production value.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    pub pushall_interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            backoff_min: Duration::from_secs(2),
            backoff_max: Duration::from_secs(60),
            pushall_interval: Duration::from_secs(300),
        }
    }
}

pub fn report_topic(serial: &str) -> String {
    format!("device/{serial}/report")
}

pub fn request_topic(serial: &str) -> String {
    format!("device/{serial}/request")
}

pub fn pushall_request(seq: u64) -> String {
    json!({"pushing": {"sequence_id": seq.to_string(), "command": "pushall", "version": 1, "push_target": 1}})
        .to_string()
}

pub fn get_version_request(seq: u64) -> String {
    json!({"info": {"sequence_id": seq.to_string(), "command": "get_version"}}).to_string()
}

/// Exponential backoff: `min`, `2·min`, … up to `max`.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
    min: Duration,
    max: Duration,
}

impl Backoff {
    pub fn new(min: Duration, max: Duration) -> Self {
        Self {
            next: min,
            min,
            max,
        }
    }
    pub fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = (self.next * 2).min(self.max);
        d
    }
    pub fn reset(&mut self) {
        self.next = self.min;
    }
}

/// Allows one `pushall` per interval, across reconnects.
#[derive(Debug, Clone)]
pub struct PushallGate {
    last: Option<Instant>,
    interval: Duration,
}

impl PushallGate {
    pub fn new(interval: Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }
    pub fn try_take(&mut self, now: Instant) -> bool {
        match self.last {
            Some(t) if now.duration_since(t) < self.interval => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

/// Connects, and reconnects with backoff, until `events` is closed or the
/// task is aborted.
pub async fn run(params: ClientParams, events: mpsc::Sender<ClientEvent>, timing: Timing) {
    let mut backoff = Backoff::new(timing.backoff_min, timing.backoff_max);
    let mut gate = PushallGate::new(timing.pushall_interval);
    let mut seq: u64 = 0;
    loop {
        if events
            .send(ClientEvent::Connection(ConnectionState::Connecting))
            .await
            .is_err()
        {
            return;
        }
        let ended = connect_once(&params, &events, &mut gate, &mut seq, &mut backoff).await;
        let Some(state) = ended else { return };
        tracing::debug!(serial = %params.serial, ?state, "printer connection ended");
        if events.send(ClientEvent::Connection(state)).await.is_err() {
            return;
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// One connection attempt. `None` means the receiver is gone.
async fn connect_once(
    params: &ClientParams,
    events: &mpsc::Sender<ClientEvent>,
    gate: &mut PushallGate,
    seq: &mut u64,
    backoff: &mut Backoff,
) -> Option<ConnectionState> {
    let client_id = format!(
        "bambumate-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut opts = MqttOptions::new(client_id, params.host.clone(), params.port);
    opts.set_credentials(MQTT_USER, params.access_code.clone());
    opts.set_keep_alive(Duration::from_secs(30));
    // A full H2 push is ~30 KB; rumqttc's default limit is 10 KB.
    opts.set_max_packet_size(1 << 20, 1 << 16);
    opts.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(
        params.tls.clone(),
    )));
    let (client, mut eventloop) = AsyncClient::new(opts, 10);
    let report = report_topic(&params.serial);
    let request = request_topic(&params.serial);
    let mut connected = false;
    params.rejection.take();
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                connected = true;
                backoff.reset();
                events
                    .send(ClientEvent::Connection(ConnectionState::Connected))
                    .await
                    .ok()?;
                let _ = client.try_subscribe(report.clone(), QoS::AtMostOnce);
                *seq += 1;
                let _ = client.try_publish(
                    request.clone(),
                    QoS::AtMostOnce,
                    false,
                    get_version_request(*seq),
                );
                if gate.try_take(Instant::now()) {
                    *seq += 1;
                    let _ = client.try_publish(
                        request.clone(),
                        QoS::AtMostOnce,
                        false,
                        pushall_request(*seq),
                    );
                }
            }
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == report => {
                events
                    .send(ClientEvent::Report(p.payload.to_vec()))
                    .await
                    .ok()?;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(serial = %params.serial, "printer MQTT error: {e}");
                return Some(classify(&e, params.rejection.take(), connected));
            }
        }
    }
}

fn classify(
    err: &ConnectionError,
    rejection: Option<Rejection>,
    was_connected: bool,
) -> ConnectionState {
    match (err, rejection) {
        (
            ConnectionError::ConnectionRefused(
                ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized,
            ),
            _,
        ) => ConnectionState::AuthFailed,
        (_, Some(Rejection::Untrusted { fingerprint })) => {
            ConnectionState::CertUntrusted { fingerprint }
        }
        (_, Some(Rejection::WrongSerial { presented })) => {
            ConnectionState::WrongSerial { presented }
        }
        _ if was_connected => ConnectionState::Disconnected,
        _ => ConnectionState::Unreachable,
    }
}

/// The result of Settings → Printer → Test connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestOutcome {
    pub connection: ConnectionState,
    /// True once a full status report arrived.
    pub got_report: bool,
    /// The model from `get_version`, e.g. `H2D`.
    pub model: Option<String>,
}

/// Connects once, waits for the first full report or a failure, then
/// disconnects.
pub async fn test_connection(params: ClientParams, timing: Timing, wait: Duration) -> TestOutcome {
    let (tx, mut rx) = mpsc::channel(64);
    let task = tokio::spawn(run(params, tx, timing));
    let mut outcome = TestOutcome {
        connection: ConnectionState::Unreachable,
        got_report: false,
        model: None,
    };
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match ev {
            ClientEvent::Connection(ConnectionState::Connecting) => {}
            ClientEvent::Connection(state) => {
                let done = state != ConnectionState::Connected;
                outcome.connection = state;
                if done {
                    break;
                }
            }
            ClientEvent::Report(bytes) => match super::state::parse_report(&bytes) {
                Ok(super::state::Report::Version { model, .. }) => outcome.model = model,
                Ok(super::state::Report::Status { full: true, .. }) => {
                    outcome.got_report = true;
                    // get_version is sent first, so its answer is usually in.
                    let grace = tokio::time::Instant::now() + Duration::from_millis(500);
                    while let Ok(Some(ClientEvent::Report(b))) =
                        tokio::time::timeout_at(grace, rx.recv()).await
                    {
                        if let Ok(super::state::Report::Version { model, .. }) =
                            super::state::parse_report(&b)
                        {
                            outcome.model = model;
                        }
                    }
                    break;
                }
                _ => {}
            },
        }
    }
    task.abort();
    outcome
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib printer::`
Expected: `test result: ok. 35 passed; 0 failed`. That is state 15, tls 9 and client 11. The broker tests take about half a second and use only 127.0.0.1.

Run each broker test three more times to rule out flakiness: `cd src-tauri && for i in 1 2 3; do cargo test --lib printer::client 2>&1 | grep "test result"; done`
Expected: three `ok. 11 passed` lines.

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/printer/mod.rs src-tauri/src/printer/client.rs src-tauri/src/printer/testbroker.rs
git commit -m "Add the read-only printer MQTT client with reconnect and connection test"
```

---

### Task 4: HMS error text with a seven-day cache

**Files:**
- Create: `src-tauri/src/printer/hms.rs`
- Modify: `src-tauri/src/printer/mod.rs`

**Interfaces:**
- Consumes: Task 1 `state::{PrinterState, HmsCode}`.
- Produces (in `crate::printer::hms`):
  - `pub const HMS_URL: &str = "https://e.bambulab.com/query.php"`
  - `pub struct ErrorView { pub kind: String /* "hms" | "print_error" */, pub code: String, pub text: Option<String>, pub wiki_url: String }` (`Debug, Clone, PartialEq, Serialize, Deserialize`)
  - `pub struct HmsCatalog` with:
    - `new(cache_dir: PathBuf, serial: Option<&str>)` and `with_base_url(cache_dir: PathBuf, serial: Option<&str>, base_url: &str)`
    - `is_loaded(&self) -> bool`
    - `async ensure_loaded(&self, now_secs: i64) -> bool`
    - `describe(&self, state: &PrinterState, model: Option<&str>) -> Vec<ErrorView>`
  - `pub fn hms_wiki_url(code: &str, model: Option<&str>) -> String`

- [ ] **Step 1: Write the failing tests**

Replace `src-tauri/src/printer/mod.rs` with:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod client;
pub mod hms;
pub mod state;
#[cfg(test)]
mod testbroker;
pub mod tls;
```

Create `src-tauri/src/printer/hms.rs` with only the test code for now. The stub is a one-route HTTP server on 127.0.0.1, so the tests never reach Bambu:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::HmsCode;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const NOW: i64 = 1_790_000_000;
    const BODY: &str = r#"{"result":0,"t":1790657978,"ver":202609231145,"data":{
        "device_hms":{"ver":202609231145,"en":[
            {"ecode":"0300010000010007","intro":"The heatbed temperature is abnormal; the sensor may have an open circuit."},
            {"ecode":"0500060000020045","intro":""}]},
        "device_error":{"ver":202609231145,"en":[
            {"ecode":"0300400C","intro":"The task was canceled."}]}}}"#;

    /// A one-route HTTP server that counts requests and records the query.
    async fn stub() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/query.php", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (h, p) = (hits.clone(), paths.clone());
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                let mut req = vec![0u8; 4096];
                let n = sock.read(&mut req).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&req[..n])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                p.lock().unwrap().push(line);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    BODY.len(),
                    BODY
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (url, hits, paths)
    }

    fn state_with_errors() -> PrinterState {
        PrinterState {
            hms: vec![HmsCode {
                attr: 0x0300_0100,
                code: 0x0001_0007,
            }],
            print_error: Some(0x0300_400C),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn fetches_on_first_need_and_writes_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, paths) = stub().await;
        let cat = HmsCatalog::with_base_url(dir.path().into(), Some("0948AB000000001"), &url);
        assert!(!cat.is_loaded());
        assert!(cat.ensure_loaded(NOW).await);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(paths.lock().unwrap()[0].contains("lang=en&d=094"));
        assert!(dir.path().join("hms_en_094.json").exists());

        let errors = cat.describe(&state_with_errors(), Some("H2D"));
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].code, "0300_0100_0001_0007");
        assert_eq!(
            errors[0].text.as_deref(),
            Some("The heatbed temperature is abnormal; the sensor may have an open circuit.")
        );
        assert_eq!(errors[1].kind, "print_error");
        assert_eq!(errors[1].code, "0300_400C");
        assert_eq!(errors[1].text.as_deref(), Some("The task was canceled."));

        assert!(cat.ensure_loaded(NOW + 60).await);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "memory hit");
    }

    #[tokio::test]
    async fn a_fresh_disk_cache_is_used_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, _) = stub().await;
        HmsCatalog::with_base_url(dir.path().into(), None, &url)
            .ensure_loaded(NOW)
            .await;
        let again = HmsCatalog::with_base_url(dir.path().into(), None, &url);
        assert!(again.ensure_loaded(NOW + 6 * 24 * 3600).await);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cache_older_than_seven_days_is_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, _) = stub().await;
        HmsCatalog::with_base_url(dir.path().into(), None, &url)
            .ensure_loaded(NOW)
            .await;
        let again = HmsCatalog::with_base_url(dir.path().into(), None, &url);
        assert!(again.ensure_loaded(NOW + 8 * 24 * 3600).await);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn offline_without_a_cache_falls_back_to_the_wiki_link() {
        let dir = tempfile::tempdir().unwrap();
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/query.php", spare.local_addr().unwrap());
        drop(spare);
        let cat = HmsCatalog::with_base_url(dir.path().into(), None, &url);
        assert!(!cat.ensure_loaded(NOW).await);
        let errors = cat.describe(&state_with_errors(), Some("H2D"));
        assert_eq!(errors[0].text, None);
        assert_eq!(
            errors[0].wiki_url,
            "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0100_0001_0007"
        );
    }

    #[tokio::test]
    async fn offline_with_a_stale_cache_still_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _, _) = stub().await;
        HmsCatalog::with_base_url(dir.path().into(), None, &url)
            .ensure_loaded(NOW)
            .await;
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}/query.php", spare.local_addr().unwrap());
        drop(spare);
        let cat = HmsCatalog::with_base_url(dir.path().into(), None, &dead);
        assert!(cat.ensure_loaded(NOW + 30 * 24 * 3600).await);
        assert!(cat.describe(&state_with_errors(), None)[0].text.is_some());
    }

    #[test]
    fn wiki_links_follow_the_model_family() {
        let code = "0300_0100_0001_0007";
        assert!(hms_wiki_url(code, Some("H2D Pro")).contains("/en/h2/"));
        assert!(hms_wiki_url(code, Some("H2C")).contains("/en/h2c/"));
        assert!(hms_wiki_url(code, Some("X1 Carbon")).contains("/en/x1/"));
        assert!(hms_wiki_url(code, None).ends_with("/hmscode/0300_0100_0001_0007"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib printer::hms`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `HmsCatalog` and the other items not written yet.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/hms.rs`:

```rust
//! Text for HMS and `print_error` codes.
//!
//! The text comes from Bambu's public error list: a static JSON document
//! over plain HTTPS that needs no account (`https://e.bambulab.com/query.php`,
//! the source ha-bambulab's `scripts/update_error_text.py` uses). It is
//! fetched on first need and cached in app data for 7 days. With no network
//! and no cache, an error shows its code and a link to Bambu's wiki.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::state::PrinterState;

pub const HMS_URL: &str = "https://e.bambulab.com/query.php";
const CACHE_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// After a failed fetch, wait this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(600);
const LANG: &str = "en";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HmsCache {
    fetched_at: i64,
    device_hms: HashMap<String, String>,
    device_error: HashMap<String, String>,
}

/// One active error, ready to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorView {
    /// `hms` or `print_error`.
    pub kind: String,
    /// `0300_0100_0001_0007` (HMS) or `0300_400C` (print error).
    pub code: String,
    /// `None` when the text is unknown or not loaded.
    pub text: Option<String>,
    pub wiki_url: String,
}

pub struct HmsCatalog {
    base_url: String,
    cache_path: PathBuf,
    /// The first three characters of the serial: Bambu's list for that model
    /// has codes the generic list lacks. `None` fetches the generic list.
    model_prefix: Option<String>,
    cache: Mutex<Option<HmsCache>>,
    last_failed_fetch: Mutex<Option<Instant>>,
    http: reqwest::Client,
}

impl HmsCatalog {
    /// `cache_dir` is the app data dir; the file is `hms_en[_<prefix>].json`.
    pub fn new(cache_dir: PathBuf, serial: Option<&str>) -> Self {
        Self::with_base_url(cache_dir, serial, HMS_URL)
    }

    pub fn with_base_url(cache_dir: PathBuf, serial: Option<&str>, base_url: &str) -> Self {
        let model_prefix = serial
            .map(|s| s.chars().take(3).collect::<String>())
            .filter(|p| p.len() == 3 && p.chars().all(|c| c.is_ascii_alphanumeric()));
        let file = match &model_prefix {
            Some(p) => format!("hms_{LANG}_{p}.json"),
            None => format!("hms_{LANG}.json"),
        };
        Self {
            base_url: base_url.to_string(),
            cache_path: cache_dir.join(file),
            model_prefix,
            cache: Mutex::new(None),
            last_failed_fetch: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        }
    }

    /// True when error text is available in memory.
    pub fn is_loaded(&self) -> bool {
        self.cache.lock().unwrap().is_some()
    }

    /// Makes the text available: memory, then a fresh disk cache, then the
    /// network, then a stale disk cache. Returns whether text is available.
    pub async fn ensure_loaded(&self, now_secs: i64) -> bool {
        if let Some(c) = self.cache.lock().unwrap().as_ref() {
            if now_secs - c.fetched_at < CACHE_TTL_SECS {
                return true;
            }
        }
        let on_disk = std::fs::read(&self.cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<HmsCache>(&b).ok());
        if let Some(c) = &on_disk {
            if now_secs - c.fetched_at < CACHE_TTL_SECS {
                *self.cache.lock().unwrap() = on_disk;
                return true;
            }
        }
        let recently_failed = self
            .last_failed_fetch
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < RETRY_AFTER);
        if !recently_failed {
            match self.fetch(now_secs).await {
                Ok(fresh) => {
                    if let Some(dir) = self.cache_path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Ok(bytes) = serde_json::to_vec(&fresh) {
                        if let Err(e) = std::fs::write(&self.cache_path, bytes) {
                            tracing::debug!("could not write HMS cache: {e}");
                        }
                    }
                    *self.cache.lock().unwrap() = Some(fresh);
                    return true;
                }
                Err(e) => {
                    tracing::debug!("HMS list fetch failed: {e}");
                    *self.last_failed_fetch.lock().unwrap() = Some(Instant::now());
                }
            }
        }
        if on_disk.is_some() {
            *self.cache.lock().unwrap() = on_disk;
            return true;
        }
        self.is_loaded()
    }

    async fn fetch(&self, now_secs: i64) -> Result<HmsCache, String> {
        let mut query = vec![("lang", LANG.to_string())];
        if let Some(p) = &self.model_prefix {
            query.push(("d", p.clone()));
        }
        let body: Value = self
            .http
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if body.get("result").and_then(Value::as_i64) != Some(0) {
            return Err("HMS list request was refused".into());
        }
        let table = |kind: &str| -> HashMap<String, String> {
            body.pointer(&format!("/data/{kind}/{LANG}"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|e| {
                            let code = e.get("ecode")?.as_str()?.to_ascii_uppercase();
                            let text = e.get("intro")?.as_str()?.trim().to_string();
                            (!text.is_empty()).then_some((code, text))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let cache = HmsCache {
            fetched_at: now_secs,
            device_hms: table("device_hms"),
            device_error: table("device_error"),
        };
        if cache.device_hms.is_empty() && cache.device_error.is_empty() {
            return Err("HMS list was empty".into());
        }
        Ok(cache)
    }

    /// Every active error in `state`, with text when it's known.
    pub fn describe(&self, state: &PrinterState, model: Option<&str>) -> Vec<ErrorView> {
        let cache = self.cache.lock().unwrap();
        let mut out: Vec<ErrorView> = state
            .hms
            .iter()
            .map(|h| {
                let code = h.display();
                ErrorView {
                    kind: "hms".into(),
                    text: cache
                        .as_ref()
                        .and_then(|c| c.device_hms.get(&code.replace('_', "")).cloned()),
                    wiki_url: hms_wiki_url(&code, model),
                    code,
                }
            })
            .collect();
        if let Some(err) = state.print_error {
            let hex = format!("{err:08X}");
            out.push(ErrorView {
                kind: "print_error".into(),
                code: format!("{}_{}", &hex[..4], &hex[4..]),
                text: cache
                    .as_ref()
                    .and_then(|c| c.device_error.get(&hex).cloned()),
                wiki_url: WIKI_HMS_HOME.into(),
            });
        }
        out
    }
}

const WIKI_HMS_HOME: &str = "https://wiki.bambulab.com/en/hms/home";

/// The wiki page for one HMS code. The path segment is the model family,
/// following the links ha-bambulab collects in `hms_error_text/wiki_links.json`.
pub fn hms_wiki_url(code: &str, model: Option<&str>) -> String {
    let m = model.unwrap_or("").to_ascii_uppercase().replace(' ', "");
    let family = match m.as_str() {
        "H2D" | "H2DPRO" => "h2",
        "H2C" => "h2c",
        "H2S" => "h2s",
        "X2D" => "x2d",
        "P2S" => "p2s",
        "A1" | "A1MINI" => "a1",
        "P1P" | "P1S" => "p1",
        _ => "x1",
    };
    format!("https://wiki.bambulab.com/en/{family}/troubleshooting/hmscode/{code}")
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib printer::hms`
Expected: `test result: ok. 6 passed; 0 failed`

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/printer/mod.rs src-tauri/src/printer/hms.rs
git commit -m "Look up HMS and print error text with a seven-day cache and a wiki fallback"
```

---

### Task 5: Slot assignments and slot status

**Files:**
- Modify: `src-tauri/src/history/types.rs` (append `SlotAssignment`)
- Modify: `src-tauri/src/history/store.rs:6` (import), `:54-57` (table), `:159-160` (methods), test module (one test)
- Create: `src-tauri/src/printer/slots.rs`
- Modify: `src-tauri/src/printer/mod.rs`

**Interfaces:**
- Consumes:
  - Task 1: `state::{PrinterState, Tray}` and `state::fixtures::{state_after, H2D_FULL, H2D_DELTA_AMS}`
  - Existing: `profile::{reader::read_profile_metadata, FilamentProfile, ProfileRegistry}`
- Produces:
  - `crate::history::SlotAssignment { pub serial: String, pub ams_id: u32, pub tray_id: u32, pub preset_name: String, pub filament_id: Option<String>, pub preset_path: Option<String>, pub assigned_at: String }` (`Debug, Clone, PartialEq, Serialize, Deserialize`)
  - `RefinementHistory::assign_slot(&self, serial: &str, ams_id: u32, tray_id: u32, preset_name: &str, filament_id: Option<&str>, preset_path: Option<&str>) -> Result<(), String>`
  - `RefinementHistory::clear_slot(&self, serial: &str, ams_id: u32, tray_id: u32) -> Result<(), String>`
  - `RefinementHistory::list_slot_assignments(&self, serial: &str) -> Result<Vec<SlotAssignment>, String>`
  - In `crate::printer::slots`:
    - `pub const EXTERNAL_AMS_ID: u32 = 255`
    - `pub enum SlotStatus { Matches, Rfid, Different, Empty, Unassigned }` (`Copy`, serde `snake_case`)
    - `pub struct SlotView { pub ams_id: u32, pub tray_id: u32, pub label: String, pub tray: Tray, pub rfid: bool, pub assigned_preset: Option<String>, pub assigned_filament_id: Option<String>, pub status: SlotStatus, pub needs_cloud_sync: bool }`
    - `pub fn slot_label(ams_id: u32, tray_id: u32, external_count: usize) -> String`
    - `pub fn slot_status(tray: &Tray, assigned: Option<&SlotAssignment>) -> SlotStatus`
    - `pub fn compute_slots(state: &PrinterState, assignments: &[SlotAssignment], needs_cloud_sync: &dyn Fn(&str) -> bool) -> Vec<SlotView>`
    - `pub fn preset_needs_cloud_sync(json_path: &Path) -> bool`
    - `pub fn resolve_filament_id(profile: &FilamentProfile, registry: &ProfileRegistry) -> Option<String>`
    - `pub fn set_on_printer_steps(label: &str, preset: &str, needs_cloud_sync: bool) -> Vec<String>`

- [ ] **Step 1: Write the failing store test**

Append to `src-tauri/src/history/types.rs`:

```rust
/// The filament preset the user says is loaded in one printer slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotAssignment {
    pub serial: String,
    /// 0-3 for AMS units, 128+ for AMS HT, 255 for external spools.
    pub ams_id: u32,
    /// 0-3 within an AMS; 254/255 for external spools.
    pub tray_id: u32,
    pub preset_name: String,
    /// The preset's `filament_id`, resolved through `inherits`.
    pub filament_id: Option<String>,
    /// The preset's JSON file, for checking its cloud-sync state later.
    pub preset_path: Option<String>,
    pub assigned_at: String,
}
```

In `src-tauri/src/history/store.rs`, add this test at the end of `mod tests`, after `test_get_nonexistent_session` or whichever test comes last:

```rust
    #[test]
    fn slot_assignments_round_trip_and_replace() {
        let (store, _dir) = create_test_store();
        store
            .assign_slot(
                "SN1",
                0,
                2,
                "Acme PLA",
                Some("P1234567"),
                Some("/u/Acme PLA.json"),
            )
            .unwrap();
        store
            .assign_slot("SN1", 255, 254, "Generic PETG", None, None)
            .unwrap();
        store
            .assign_slot("SN2", 0, 0, "Other printer", None, None)
            .unwrap();
        // Assigning the same slot again replaces the preset.
        store
            .assign_slot("SN1", 0, 2, "Acme PLA Silk", Some("P7654321"), None)
            .unwrap();

        let rows = store.list_slot_assignments("SN1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].ams_id, rows[0].tray_id), (0, 2));
        assert_eq!(rows[0].preset_name, "Acme PLA Silk");
        assert_eq!(rows[0].filament_id.as_deref(), Some("P7654321"));
        assert_eq!(rows[0].preset_path, None);
        assert!(!rows[0].assigned_at.is_empty());
        assert_eq!((rows[1].ams_id, rows[1].tray_id), (255, 254));

        store.clear_slot("SN1", 0, 2).unwrap();
        store.clear_slot("SN1", 3, 3).unwrap();
        assert_eq!(store.list_slot_assignments("SN1").unwrap().len(), 1);
    }
```

Run: `cd src-tauri && cargo test --lib history::store`
Expected: FAIL to compile: `` no method named `assign_slot` found for struct `RefinementHistory` ``.

- [ ] **Step 2: Add the table and methods**

In `src-tauri/src/history/store.rs`, change the types import to:

```rust
use super::types::{AppliedChange, SessionDetail, SessionSummary, SlotAssignment};
```

In `RefinementHistory::new`, after the `idx_sessions_created` index is created and before `info!("Opened refinement history database …")`, add:

```rust
        // Which filament preset the user says is loaded in each printer slot.
        // External spools use ams_id 255 with their own tray ids (254, 255).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS slot_assignments (
                serial TEXT NOT NULL,
                ams_id INTEGER NOT NULL,
                tray_id INTEGER NOT NULL,
                preset_name TEXT NOT NULL,
                filament_id TEXT,
                preset_path TEXT,
                assigned_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (serial, ams_id, tray_id)
            )",
            [],
        )
        .map_err(|e| format!("Failed to create slot_assignments table: {}", e))?;
```

After `get_session` (the last method in `impl RefinementHistory`), add:

```rust
    /// Records (or replaces) the preset the user says is in one slot.
    pub fn assign_slot(
        &self,
        serial: &str,
        ams_id: u32,
        tray_id: u32,
        preset_name: &str,
        filament_id: Option<&str>,
        preset_path: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO slot_assignments
                 (serial, ams_id, tray_id, preset_name, filament_id, preset_path, assigned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
                params![
                    serial,
                    ams_id,
                    tray_id,
                    preset_name,
                    filament_id,
                    preset_path
                ],
            )
            .map_err(|e| format!("Failed to save slot assignment: {}", e))?;
        Ok(())
    }

    /// Forgets the preset assigned to one slot. Clearing an unassigned slot
    /// is not an error.
    pub fn clear_slot(&self, serial: &str, ams_id: u32, tray_id: u32) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM slot_assignments WHERE serial = ?1 AND ams_id = ?2 AND tray_id = ?3",
                params![serial, ams_id, tray_id],
            )
            .map_err(|e| format!("Failed to clear slot assignment: {}", e))?;
        Ok(())
    }

    /// Every slot assignment for one printer, in slot order.
    pub fn list_slot_assignments(&self, serial: &str) -> Result<Vec<SlotAssignment>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT serial, ams_id, tray_id, preset_name, filament_id, preset_path, assigned_at
                 FROM slot_assignments WHERE serial = ?1 ORDER BY ams_id, tray_id",
            )
            .map_err(|e| format!("Failed to prepare query: {}", e))?;
        let rows = stmt
            .query_map(params![serial], |row| {
                Ok(SlotAssignment {
                    serial: row.get(0)?,
                    ams_id: row.get(1)?,
                    tray_id: row.get(2)?,
                    preset_name: row.get(3)?,
                    filament_id: row.get(4)?,
                    preset_path: row.get(5)?,
                    assigned_at: row.get(6)?,
                })
            })
            .map_err(|e| format!("Failed to query slot assignments: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read slot assignments: {}", e))
    }
```

Run: `cd src-tauri && cargo test --lib history::`
Expected: `test result: ok. 6 passed; 0 failed`

- [ ] **Step 3: Write the failing slot tests**

Replace `src-tauri/src/printer/mod.rs` with:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod client;
pub mod hms;
pub mod slots;
pub mod state;
#[cfg(test)]
mod testbroker;
pub mod tls;
```

Create `src-tauri/src/printer/slots.rs` with only the test code for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::fixtures::{state_after, H2D_DELTA_AMS, H2D_FULL};

    fn assignment(ams_id: u32, tray_id: u32, filament_id: Option<&str>) -> SlotAssignment {
        SlotAssignment {
            serial: "SN".into(),
            ams_id,
            tray_id,
            preset_name: "Acme PLA".into(),
            filament_id: filament_id.map(str::to_string),
            preset_path: Some("/u/Acme PLA.json".into()),
            assigned_at: "2026-09-28 10:00:00".into(),
        }
    }

    fn tray(idx: &str, rfid: bool) -> Tray {
        Tray {
            id: 0,
            empty: false,
            tray_type: "PLA".into(),
            tray_info_idx: idx.into(),
            tag_uid: if rfid {
                "9AD3FBAC00000100"
            } else {
                "0000000000000000"
            }
            .into(),
            tray_uuid: "00000000000000000000000000000000".into(),
            ..Default::default()
        }
    }

    #[test]
    fn labels_cover_ams_units_ams_ht_and_external_spools() {
        assert_eq!(slot_label(0, 0, 2), "A1");
        assert_eq!(slot_label(1, 3, 2), "B4");
        assert_eq!(slot_label(3, 3, 2), "D4");
        assert_eq!(slot_label(128, 0, 2), "HT1");
        assert_eq!(slot_label(EXTERNAL_AMS_ID, 254, 2), "Ext-L");
        assert_eq!(slot_label(EXTERNAL_AMS_ID, 255, 2), "Ext-R");
        assert_eq!(slot_label(EXTERNAL_AMS_ID, 254, 1), "Ext");
    }

    #[test]
    fn matching_filament_id_is_set() {
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("p1234567", false), Some(&a)),
            SlotStatus::Matches
        );
    }

    #[test]
    fn an_rfid_spool_needs_no_assignment() {
        assert_eq!(slot_status(&tray("GFA00", true), None), SlotStatus::Rfid);
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("GFA00", true), Some(&a)),
            SlotStatus::Rfid
        );
    }

    #[test]
    fn a_different_reported_filament_needs_setting_on_the_printer() {
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("GFL99", false), Some(&a)),
            SlotStatus::Different
        );
        let unresolved = assignment(0, 0, None);
        assert_eq!(
            slot_status(&tray("", false), Some(&unresolved)),
            SlotStatus::Different
        );
    }

    #[test]
    fn an_empty_slot_is_empty_even_with_an_assignment() {
        let mut t = tray("", false);
        t.empty = true;
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(slot_status(&t, Some(&a)), SlotStatus::Empty);
    }

    #[test]
    fn no_assignment_and_no_rfid_is_not_set() {
        assert_eq!(
            slot_status(&tray("GFL99", false), None),
            SlotStatus::Unassigned
        );
    }

    #[test]
    fn compute_slots_walks_every_unit_then_the_external_spools() {
        let state = state_after(&[H2D_FULL]);
        let assignments = vec![assignment(1, 1, Some("P1234567"))];
        let slots = compute_slots(&state, &assignments, &|_| true);
        let labels: Vec<&str> = slots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["A1", "A2", "A3", "A4", "B1", "B2", "B3", "B4", "Ext-L", "Ext-R"]
        );
        let b2 = &slots[5];
        assert_eq!(b2.status, SlotStatus::Different);
        assert_eq!(b2.assigned_preset.as_deref(), Some("Acme PLA"));
        assert!(b2.needs_cloud_sync);
        assert_eq!(slots[1].status, SlotStatus::Rfid);
        assert_eq!(slots[2].status, SlotStatus::Unassigned);
        assert_eq!(slots[7].status, SlotStatus::Empty);
        assert_eq!(slots[9].status, SlotStatus::Empty);
    }

    #[test]
    fn a_report_that_changes_to_match_flips_the_slot_to_set() {
        let assignments = vec![assignment(1, 1, Some("P1234567"))];
        let before = compute_slots(&state_after(&[H2D_FULL]), &assignments, &|_| false);
        let after = compute_slots(
            &state_after(&[H2D_FULL, H2D_DELTA_AMS]),
            &assignments,
            &|_| false,
        );
        assert_eq!(before[5].status, SlotStatus::Different);
        assert_eq!(after[5].status, SlotStatus::Matches);
    }

    #[test]
    fn cloud_sync_is_needed_only_for_a_user_preset_without_a_setting_id() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("Acme PLA.json");
        std::fs::write(&json, r#"{"name":"Acme PLA"}"#).unwrap();
        assert!(!preset_needs_cloud_sync(&json), "no .info: a system preset");
        std::fs::write(
            json.with_extension("info"),
            "sync_info =\nuser_id = 1\nsetting_id =\nbase_id =\nupdated_time = 1\n",
        )
        .unwrap();
        assert!(preset_needs_cloud_sync(&json));
        std::fs::write(
            json.with_extension("info"),
            "sync_info =\nuser_id = 1\nsetting_id = PFUS123\nbase_id =\nupdated_time = 1\n",
        )
        .unwrap();
        assert!(!preset_needs_cloud_sync(&json));
    }

    #[test]
    fn filament_id_is_inherited_from_the_parent_preset() {
        let mut registry = ProfileRegistry::new();
        registry.insert(
            FilamentProfile::from_json(r#"{"name":"Bambu PLA Basic @base","filament_id":"GFA00"}"#)
                .unwrap(),
        );
        registry.insert(
            FilamentProfile::from_json(
                r#"{"name":"Bambu PLA Basic @BBL H2D","inherits":"Bambu PLA Basic @base"}"#,
            )
            .unwrap(),
        );
        let leaf = FilamentProfile::from_json(
            r#"{"name":"My PLA","inherits":"Bambu PLA Basic @BBL H2D"}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_filament_id(&leaf, &registry).as_deref(),
            Some("GFA00")
        );
        let own =
            FilamentProfile::from_json(r#"{"name":"Mine","filament_id":"P1234567"}"#).unwrap();
        assert_eq!(
            resolve_filament_id(&own, &registry).as_deref(),
            Some("P1234567")
        );
        let orphan =
            FilamentProfile::from_json(r#"{"name":"Orphan","inherits":"Missing"}"#).unwrap();
        assert_eq!(resolve_filament_id(&orphan, &registry), None);
    }

    #[test]
    fn steps_add_the_cloud_sync_note_only_when_needed() {
        let steps = set_on_printer_steps("B2", "Acme PLA", false);
        assert_eq!(
            steps,
            vec![
                "On the printer: Filament → B2 → choose Acme PLA.",
                "Or in Bambu Studio: Device → AMS → B2 → Acme PLA.",
            ]
        );
        assert_eq!(set_on_printer_steps("B2", "Acme PLA", true).len(), 3);
    }
}
```

Run: `cd src-tauri && cargo test --lib printer::slots`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `slot_label` and the other items not written yet.

- [ ] **Step 4: Write the implementation**

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/slots.rs`:

```rust
//! Slot labels and the status of each slot: does the printer report the
//! preset the user assigned to it?

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::state::{PrinterState, Tray};
use crate::history::SlotAssignment;
use crate::profile::{reader, FilamentProfile, ProfileRegistry};

/// `ams_id` for external spools in `slot_assignments`.
pub const EXTERNAL_AMS_ID: u32 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStatus {
    /// The printer reports the assigned preset's filament id. "✓ Set".
    Matches,
    /// A Bambu RFID spool. "✓ Bambu spool".
    Rfid,
    /// The printer reports a different filament. "Set on printer".
    Different,
    /// No filament in the slot. "Empty".
    Empty,
    /// No assignment and no RFID. "Not set".
    Unassigned,
}

/// One slot as the Printer page and `bm_ams_slots` show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotView {
    pub ams_id: u32,
    pub tray_id: u32,
    /// `A1`…`D4`, `HT1`, `Ext-L`, `Ext-R` or `Ext`.
    pub label: String,
    /// What the printer reports for the slot.
    pub tray: Tray,
    pub rfid: bool,
    pub assigned_preset: Option<String>,
    pub assigned_filament_id: Option<String>,
    pub status: SlotStatus,
    /// The assigned preset is a user preset Bambu Cloud doesn't have yet,
    /// so the printer can't select it.
    pub needs_cloud_sync: bool,
}

/// `A1`…`D4` for AMS units (lettered by id), `HT1`… for AMS HT units
/// (ids 128+), and `Ext-L`/`Ext-R` for the two H2 external spools (`Ext`
/// when there is only one). On the H2 series 254 is the left holder.
pub fn slot_label(ams_id: u32, tray_id: u32, external_count: usize) -> String {
    if ams_id == EXTERNAL_AMS_ID {
        return match (external_count >= 2, tray_id) {
            (true, 254) => "Ext-L".into(),
            (true, 255) => "Ext-R".into(),
            _ => "Ext".into(),
        };
    }
    if ams_id >= 128 {
        return format!("HT{}", ams_id - 127);
    }
    let letter = char::from_u32('A' as u32 + ams_id).unwrap_or('?');
    format!("{letter}{}", tray_id + 1)
}

/// The status rules, in order: an empty slot is Empty; a reported filament
/// id equal to the assigned preset's is Matches; an RFID spool is Rfid;
/// any other assignment is Different; otherwise Unassigned.
pub fn slot_status(tray: &Tray, assigned: Option<&SlotAssignment>) -> SlotStatus {
    if tray.empty {
        return SlotStatus::Empty;
    }
    let reported = tray.tray_info_idx.trim();
    let assigned_id = assigned
        .and_then(|a| a.filament_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if assigned_id.is_some_and(|id| !reported.is_empty() && id.eq_ignore_ascii_case(reported)) {
        return SlotStatus::Matches;
    }
    if tray.has_rfid() {
        return SlotStatus::Rfid;
    }
    if assigned.is_some() {
        return SlotStatus::Different;
    }
    SlotStatus::Unassigned
}

/// Every AMS slot, then the external spools, with its status.
/// `needs_cloud_sync` answers for an assigned preset's JSON path.
pub fn compute_slots(
    state: &PrinterState,
    assignments: &[SlotAssignment],
    needs_cloud_sync: &dyn Fn(&str) -> bool,
) -> Vec<SlotView> {
    let find = |ams_id: u32, tray_id: u32| {
        assignments
            .iter()
            .find(|a| a.ams_id == ams_id && a.tray_id == tray_id)
    };
    let external_count = state.external_spools.len();
    let mut out = Vec::new();
    let slots = state
        .ams_units
        .iter()
        .flat_map(|u| u.trays.iter().map(move |t| (u.id, t)))
        .chain(state.external_spools.iter().map(|t| (EXTERNAL_AMS_ID, t)));
    for (ams_id, tray) in slots {
        let assigned = find(ams_id, tray.id);
        out.push(SlotView {
            ams_id,
            tray_id: tray.id,
            label: slot_label(ams_id, tray.id, external_count),
            tray: tray.clone(),
            rfid: tray.has_rfid(),
            assigned_preset: assigned.map(|a| a.preset_name.clone()),
            assigned_filament_id: assigned.and_then(|a| a.filament_id.clone()),
            status: slot_status(tray, assigned),
            needs_cloud_sync: assigned
                .and_then(|a| a.preset_path.as_deref())
                .is_some_and(needs_cloud_sync),
        });
    }
    out
}

/// A user preset (it has a `.info`) whose `setting_id` is still empty has
/// never reached Bambu Cloud, so the printer can't offer it yet.
pub fn preset_needs_cloud_sync(json_path: &Path) -> bool {
    matches!(
        reader::read_profile_metadata(json_path),
        Ok(Some(meta)) if meta.setting_id.trim().is_empty()
    )
}

/// The preset's `filament_id`, following `inherits` through `registry`
/// when the preset doesn't set one itself.
pub fn resolve_filament_id(
    profile: &FilamentProfile,
    registry: &ProfileRegistry,
) -> Option<String> {
    let mut current = profile;
    for _ in 0..10 {
        if let Some(id) = current
            .filament_id()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(id.to_string());
        }
        let parent = current.inherits().filter(|p| !p.is_empty())?;
        current = registry.get_by_name(parent)?;
    }
    None
}

/// Plain-text steps for a slot whose printer setting differs from the
/// assignment. The Printer page renders the same copy with the preset name
/// in italics.
pub fn set_on_printer_steps(label: &str, preset: &str, needs_cloud_sync: bool) -> Vec<String> {
    let mut steps = vec![
        format!("On the printer: Filament → {label} → choose {preset}."),
        format!("Or in Bambu Studio: Device → AMS → {label} → {preset}."),
    ];
    if needs_cloud_sync {
        steps.push("It must sync to Bambu Cloud first — open Bambu Studio while signed in.".into());
    }
    steps
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib printer::slots`
Expected: `test result: ok. 11 passed; 0 failed`

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/history/types.rs src-tauri/src/history/store.rs src-tauri/src/printer/mod.rs src-tauri/src/printer/slots.rs
git commit -m "Store slot assignments and compute each slot's status against the printer"
```

---

### Task 6: Printer discovery and settings storage

**Files:**
- Create: `src-tauri/src/printer/discovery.rs`
- Create: `src-tauri/src/printer/settings.rs`
- Modify: `src-tauri/src/printer/mod.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: existing `tauri_plugin_store::StoreExt` and `keyring`.
- Produces:
  - In `crate::printer::discovery`:
    - `pub const DISCOVERY_PORTS: &[u16] = &[2021, 1990, 1900]`
    - `pub struct DiscoveredPrinter { pub ip: String, pub serial: String, pub name: String, pub model: String }` (serde)
    - `pub fn model_name(code: &str) -> String`
    - `pub fn parse_notify(datagram: &[u8]) -> Option<DiscoveredPrinter>`
    - `pub async fn discover(ports: &[u16], window: Duration) -> Vec<DiscoveredPrinter>`
  - In `crate::printer::settings`:
    - `pub const STORE_FILE: &str = "preferences.json"`, `STORE_KEY: &str = "printer"`, `KEYCHAIN_SERVICE: &str = "bambumate-printer-access-code"`
    - `pub struct PrinterConfig { pub ip: String, pub serial: String, pub name: String, pub model: String, pub pinned_fingerprint: Option<String> }` (`Default`, serde) with `pub fn normalized(self) -> Result<Self, String>`
    - `pub struct PrinterConfigView { pub ip, serial, name, model: String, pub pinned_fingerprint: Option<String>, pub has_access_code: bool }` with `pub fn new(config: &PrinterConfig, has_access_code: bool) -> Self`
    - `pub fn check_access_code(code: &str) -> Result<&str, String>`
    - `pub fn load_config(app: &AppHandle) -> Option<PrinterConfig>`, `save_config(app, &PrinterConfig) -> Result<(), String>`, `remove_config(app) -> Result<(), String>`
    - `pub fn get_access_code(serial: &str) -> Result<Option<String>, String>`, `set_access_code(serial, code) -> Result<(), String>`, `delete_access_code(serial) -> Result<(), String>`

- [ ] **Step 1: Add socket2**

In `src-tauri/Cargo.toml` `[dependencies]`, after `sha2 = "0.10"`, add:

```toml
socket2 = { version = "0.6", features = ["all"] }
```

- [ ] **Step 2: Write the failing tests**

Replace `src-tauri/src/printer/mod.rs` with:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod client;
pub mod discovery;
pub mod hms;
pub mod settings;
pub mod slots;
pub mod state;
#[cfg(test)]
mod testbroker;
pub mod tls;
```

Create `src-tauri/src/printer/discovery.rs` with only its tests. The listen test uses an ephemeral port on 127.0.0.1, not 2021:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const NOTIFY: &str = "NOTIFY * HTTP/1.1\r\n\
        HOST: 239.255.255.250:1900\r\n\
        Server: UPnP/1.0\r\n\
        Location: 192.168.1.20\r\n\
        NT: urn:bambulab-com:device:3dprinter:1\r\n\
        NTS: ssdp:alive\r\n\
        USN: 0948AB000000001\r\n\
        Cache-Control: max-age=1800\r\n\
        DevModel.bambu.com: O1D\r\n\
        DevName.bambu.com: Workshop H2D\r\n\
        DevSignal.bambu.com: -45\r\n\
        DevConnect.bambu.com: cloud\r\n\
        DevBind.bambu.com: occupied\r\n\r\n";

    #[test]
    fn parses_a_bambu_notify() {
        assert_eq!(
            parse_notify(NOTIFY.as_bytes()),
            Some(DiscoveredPrinter {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                name: "Workshop H2D".into(),
                model: "H2D".into(),
            })
        );
    }

    #[test]
    fn ignores_other_ssdp_traffic_and_junk() {
        let router = NOTIFY.replace(
            "urn:bambulab-com:device:3dprinter:1",
            "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        );
        assert_eq!(parse_notify(router.as_bytes()), None);
        assert_eq!(
            parse_notify(NOTIFY.replace("192.168.1.20", "not-an-ip").as_bytes()),
            None
        );
        assert_eq!(parse_notify(&[0xff, 0xfe, 0x00]), None);
    }

    #[test]
    fn unknown_model_codes_are_shown_as_sent() {
        assert_eq!(model_name("C12"), "P1S");
        assert_eq!(model_name("Z9"), "Z9");
    }

    #[tokio::test]
    async fn collects_announcements_heard_during_the_window() {
        let port = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let listen =
            tokio::spawn(async move { discover(&[port], Duration::from_millis(600)).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        for _ in 0..2 {
            sender
                .send_to(NOTIFY.as_bytes(), ("127.0.0.1", port))
                .unwrap();
        }
        let found = listen.await.unwrap();
        assert_eq!(found.len(), 1, "the same printer twice is listed once");
        assert_eq!(found[0].serial, "0948AB000000001");
    }
}
```

Create `src-tauri/src/printer/settings.rs` with only its tests. The keychain helpers aren't unit tested: CI runners have no unlocked keychain, the same reason `bambumate-doctor` runs with `--no-keychain`.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PrinterConfig {
        PrinterConfig {
            ip: " 192.168.1.20 ".into(),
            serial: " 0948ab000000001".into(),
            name: "Workshop H2D".into(),
            model: "H2D".into(),
            pinned_fingerprint: Some("  ".into()),
        }
    }

    #[test]
    fn normalizes_ip_serial_and_an_empty_pin() {
        let c = config().normalized().unwrap();
        assert_eq!(c.ip, "192.168.1.20");
        assert_eq!(c.serial, "0948AB000000001");
        assert_eq!(c.pinned_fingerprint, None);
    }

    #[test]
    fn rejects_a_hostname_or_a_bad_serial() {
        let mut c = config();
        c.ip = "printer.local".into();
        assert!(c.normalized().is_err());
        let mut c = config();
        c.serial = "09/48".into();
        assert!(c.normalized().is_err());
    }

    #[test]
    fn the_stored_json_has_no_access_code_field() {
        let v = serde_json::to_value(config().normalized().unwrap()).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["ip", "serial", "name", "model", "pinned_fingerprint"]
        );
    }

    #[test]
    fn access_codes_are_checked_without_being_echoed() {
        assert_eq!(check_access_code(" 12345678 ").unwrap(), "12345678");
        let err = check_access_code("12 34").unwrap_err();
        assert!(!err.contains("12 34"));
        assert!(check_access_code("").is_err());
    }
}
```

Run: `cd src-tauri && cargo test --lib printer::`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `parse_notify` and the other items not written yet.

- [ ] **Step 3: Write the implementations**

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/discovery.rs`:

```rust
//! Finds Bambu printers on the local network by listening for the SSDP
//! `NOTIFY` messages they multicast. Discovery only listens; it sends nothing.
//!
//! Printers announce `NT: urn:bambulab-com:device:3dprinter:1` with the IP in
//! `Location`, the serial in `USN`, and `DevName.bambu.com` /
//! `DevModel.bambu.com` headers, to UDP 2021 and 1990 (some sources say
//! 1900, so all three are joined).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub const DISCOVERY_PORTS: &[u16] = &[2021, 1990, 1900];
const SSDP_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const BAMBU_URN: &str = "urn:bambulab-com:device:3dprinter";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPrinter {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
}

/// A readable model name for a `DevModel.bambu.com` code. Codes are the
/// `model_id`s in Bambu Studio's `resources/profiles/BBL/machine/*.json`;
/// unknown codes are shown as sent.
pub fn model_name(code: &str) -> String {
    match code {
        "O1D" => "H2D",
        "O1E" => "H2D Pro",
        "O1C2" => "H2C",
        "O1S" => "H2S",
        "BL-P001" | "3DPrinter-X1-Carbon" => "X1 Carbon",
        "BL-P002" | "3DPrinter-X1" => "X1",
        "C13" => "X1E",
        "C11" => "P1P",
        "C12" => "P1S",
        "N7" => "P2S",
        "N1" => "A1 mini",
        "N2S" => "A1",
        other => other,
    }
    .to_string()
}

/// Parses one SSDP datagram. `None` unless it is a Bambu printer
/// announcement with an IP address and a serial.
pub fn parse_notify(datagram: &[u8]) -> Option<DiscoveredPrinter> {
    let text = std::str::from_utf8(datagram).ok()?;
    let headers: HashMap<String, String> = text
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let kind = headers.get("nt").or_else(|| headers.get("st"))?;
    if !kind.starts_with(BAMBU_URN) {
        return None;
    }
    let ip: IpAddr = headers.get("location")?.parse().ok()?;
    let serial = headers.get("usn")?.trim().to_string();
    if serial.is_empty() {
        return None;
    }
    Some(DiscoveredPrinter {
        ip: ip.to_string(),
        name: headers
            .get("devname.bambu.com")
            .cloned()
            .unwrap_or_default(),
        model: headers
            .get("devmodel.bambu.com")
            .map(|m| model_name(m))
            .unwrap_or_default(),
        serial,
    })
}

fn bind_listener(port: u16) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Bambu Studio may already be listening on 2021.
    socket.set_reuse_address(true)?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    if let Err(e) = socket.join_multicast_v4(&SSDP_GROUP, &Ipv4Addr::UNSPECIFIED) {
        tracing::debug!("SSDP multicast join on {port} failed: {e}");
    }
    UdpSocket::from_std(socket.into())
}

/// Listens on `ports` for `window` and returns each printer heard, once,
/// sorted by name. Ports that can't be bound are skipped.
pub async fn discover(ports: &[u16], window: Duration) -> Vec<DiscoveredPrinter> {
    let (tx, mut rx) = mpsc::channel::<DiscoveredPrinter>(64);
    let mut tasks = Vec::new();
    for &port in ports {
        match bind_listener(port) {
            Ok(socket) => {
                let tx = tx.clone();
                tasks.push(tokio::spawn(async move {
                    let mut buf = vec![0u8; 2048];
                    while let Ok((n, _)) = socket.recv_from(&mut buf).await {
                        if let Some(p) = parse_notify(&buf[..n]) {
                            if tx.send(p).await.is_err() {
                                return;
                            }
                        }
                    }
                }));
            }
            Err(e) => tracing::debug!("cannot listen for printers on UDP {port}: {e}"),
        }
    }
    drop(tx);
    let mut found: HashMap<String, DiscoveredPrinter> = HashMap::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(p)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        found.insert(p.serial.clone(), p);
    }
    for t in tasks {
        t.abort();
    }
    let mut out: Vec<DiscoveredPrinter> = found.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.serial.cmp(&b.serial)));
    out
}
```

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/settings.rs`:

```rust
//! Where the printer's settings live. IP, serial, name, model and the pinned
//! certificate fingerprint go in the app settings store; the LAN access code
//! goes only in the system keychain, keyed by serial.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub const STORE_FILE: &str = "preferences.json";
pub const STORE_KEY: &str = "printer";
pub const KEYCHAIN_SERVICE: &str = "bambumate-printer-access-code";

/// Stored settings. The access code is deliberately not a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrinterConfig {
    pub ip: String,
    pub serial: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
    /// SHA-256 of a certificate the user chose to trust (`AB:CD:…`).
    #[serde(default)]
    pub pinned_fingerprint: Option<String>,
}

impl PrinterConfig {
    /// Trims fields and rejects an IP or serial the client can't use.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.ip = self.ip.trim().to_string();
        self.serial = self.serial.trim().to_ascii_uppercase();
        self.name = self.name.trim().to_string();
        self.model = self.model.trim().to_string();
        self.pinned_fingerprint = self
            .pinned_fingerprint
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty());
        if self.ip.parse::<IpAddr>().is_err() {
            return Err("Enter the printer's IP address, like 192.168.1.20.".into());
        }
        let serial_ok = !self.serial.is_empty()
            && self.serial.len() <= 32
            && self.serial.chars().all(|c| c.is_ascii_alphanumeric());
        if !serial_ok {
            return Err("Enter the printer's serial number (letters and digits).".into());
        }
        Ok(self)
    }
}

/// What the frontend sees: never the access code, only whether one is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterConfigView {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    pub pinned_fingerprint: Option<String>,
    pub has_access_code: bool,
}

impl PrinterConfigView {
    pub fn new(config: &PrinterConfig, has_access_code: bool) -> Self {
        Self {
            ip: config.ip.clone(),
            serial: config.serial.clone(),
            name: config.name.clone(),
            model: config.model.clone(),
            pinned_fingerprint: config.pinned_fingerprint.clone(),
            has_access_code,
        }
    }
}

/// Rejects a blank or oversized access code. Never echoes it back.
pub fn check_access_code(code: &str) -> Result<&str, String> {
    let code = code.trim();
    if code.is_empty() || code.len() > 32 || code.chars().any(char::is_whitespace) {
        return Err("Enter the access code shown on the printer screen.".into());
    }
    Ok(code)
}

pub fn load_config(app: &AppHandle) -> Option<PrinterConfig> {
    let store = app.store(STORE_FILE).ok()?;
    serde_json::from_value(store.get(STORE_KEY)?).ok()
}

pub fn save_config(app: &AppHandle, config: &PrinterConfig) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        STORE_KEY,
        serde_json::to_value(config).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

pub fn remove_config(app: &AppHandle) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.delete(STORE_KEY);
    store.save().map_err(|e| e.to_string())
}

/// Reads the access code for `serial` from the system keychain
/// (service `bambumate-printer-access-code`, account = serial).
pub fn get_access_code(serial: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, serial).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(code) => Ok(Some(code)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the keychain: {e}")),
    }
}

pub fn set_access_code(serial: &str, code: &str) -> Result<(), String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, serial)
        .and_then(|e| e.set_password(code))
        .map_err(|e| format!("Could not save the access code to the keychain: {e}"))
}

/// Removing a code that isn't there is not an error.
pub fn delete_access_code(serial: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, serial).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove the access code: {e}")),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib printer::discovery`
Expected: `test result: ok. 4 passed; 0 failed`

Run: `cd src-tauri && cargo test --lib printer::settings`
Expected: `test result: ok. 4 passed; 0 failed`

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/printer/mod.rs src-tauri/src/printer/discovery.rs src-tauri/src/printer/settings.rs
git commit -m "Discover printers by listening for SSDP and store printer settings and the access code"
```

---

### Task 7: Printer service, commands and app wiring

**Files:**
- Create: `src-tauri/src/printer/service.rs`
- Create: `src-tauri/src/commands/printer.rs`
- Modify: `src-tauri/src/printer/mod.rs`, `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs` (invoke handler and setup), `src-tauri/src/commands/config.rs:183-203` (`reset_to_clean_install`)

**Interfaces:**
- Consumes:
  - Task 1: `state::{parse_report, Report, ReportMerger, PrinterState}`
  - Task 2: `tls::{client_config, PrinterCertVerifier, RejectionSlot}`
  - Task 3: `client::{self, run, test_connection, ClientEvent, ClientParams, ConnectionState, Timing, TestOutcome, MQTT_PORT}`
  - Task 4: `hms::{HmsCatalog, ErrorView, HMS_URL}`
  - Task 5: `slots::{compute_slots, preset_needs_cloud_sync, resolve_filament_id, SlotView}` and `RefinementHistory::{assign_slot, clear_slot, list_slot_assignments}`
  - Task 6: `settings::*` and `discovery::{discover, DISCOVERY_PORTS, DiscoveredPrinter}`
- Produces (in `crate::printer::service`):
  - `pub const STATE_EVENT: &str = "printer://state"`, `pub const CONNECTION_EVENT: &str = "printer://connection"`
  - `pub trait PrinterEvents: Send + Sync + 'static { fn state(&self, view: &PrinterView); fn connection(&self, state: &ConnectionState); }`; `pub struct TauriEvents(pub tauri::AppHandle)`
  - `pub struct PrinterView { pub configured: bool, pub printer: Option<PrinterSummary>, pub connection: ConnectionState, pub state: Option<PrinterState>, pub slots: Vec<SlotView>, pub errors: Vec<ErrorView> }` with `pub fn unconfigured() -> Self`
  - `pub struct PrinterSummary { pub ip: String, pub serial: String, pub name: String, pub model: String, pub firmware: Option<String> }`
  - `pub struct PrinterService` (`Clone`), managed as Tauri state, with:
    - `new(events: Arc<dyn PrinterEvents>, history_db: Option<PathBuf>, data_dir: PathBuf, hms_url: &str, timing: Timing) -> Self`
    - `start(&self, config: PrinterConfig, access_code: String) -> Result<(), String>`, `start_with(&self, config, params: ClientParams)` and `stop(&self)`
    - `connection()`, `config()` and `view()`
    - `assign_slot(&self, ams_id: u32, tray_id: u32, preset_name: &str, filament_id: Option<&str>, preset_path: Option<&str>) -> Result<PrinterView, String>`
    - `clear_slot(&self, ams_id: u32, tray_id: u32) -> Result<PrinterView, String>` and `refresh_assignments(&self)`
  - Tauri commands (JS names; Tauri maps the camelCase JS arguments to these snake_case parameters):
    - `printer_get_config() -> Option<PrinterConfigView>`
    - `printer_discover() -> Vec<DiscoveredPrinter>`
    - `printer_test_connection(ip, serial, access_code?, pinned_fingerprint?) -> TestOutcome`
    - `printer_save(ip, serial, name?, model?, access_code?, pinned_fingerprint?) -> PrinterConfigView`
    - `printer_remove() -> ()`
    - `printer_view() -> PrinterView`
    - `printer_assign_slot(ams_id, tray_id, preset_path) -> PrinterView`
    - `printer_clear_slot(ams_id, tray_id) -> PrinterView`

- [ ] **Step 1: Write the failing service tests**

Replace `src-tauri/src/printer/mod.rs` with:

```rust
//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod client;
pub mod discovery;
pub mod hms;
pub mod service;
pub mod settings;
pub mod slots;
pub mod state;
#[cfg(test)]
mod testbroker;
pub mod tls;
```

Create `src-tauri/src/printer/service.rs` with only the test code for now. The service tests pass `http://127.0.0.1:9/query.php` as the HMS URL, so nothing reaches the internet:

```rust
    #[cfg(test)]
    pub(crate) fn configure_for_test(&self, config: PrinterConfig) -> u64 {
        self.configure(config)
    }
}

fn cloud_sync_map(assignments: &[SlotAssignment]) -> HashMap<String, bool> {
    assignments
        .iter()
        .filter_map(|a| a.preset_path.clone())
        .map(|p| {
            let needs = preset_needs_cloud_sync(std::path::Path::new(&p));
            (p, needs)
        })
        .collect()
}

async fn consume(service: PrinterService, generation: u64, mut rx: mpsc::Receiver<ClientEvent>) {
    while let Some(event) = rx.recv().await {
        if let ClientEvent::Report(bytes) = &event {
            // Debug level at most; capture with
            // RUST_LOG=info,bambumate_tauri::printer::payload=debug
            tracing::debug!(
                target: "bambumate_tauri::printer::payload",
                serial = ?service.config().map(|c| c.serial),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
        service.handle_event(generation, event);
    }
}

fn live_serial(live: &Live) -> Option<String> {
    live.config.as_ref().map(|c| c.serial.clone())
}

/// Emits the view after each change, then waits, so bursts of reports
/// collapse into at most two events a second.
async fn emit_loop(inner: Weak<Inner>, mut dirty: watch::Receiver<u64>) {
    while dirty.changed().await.is_ok() {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let view = PrinterService {
            inner: inner.clone(),
        }
        .view();
        inner.events.state(&view);
        drop(inner);
        tokio::time::sleep(EMIT_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::client::ConnectionState;
    use crate::printer::slots::SlotStatus;
    use crate::printer::state::fixtures::{GET_VERSION_H2D, H2D_DELTA_PROGRESS, H2D_FULL};
    use crate::printer::testbroker::FakeBroker;
    use crate::printer::tls::testpki::TestCa;

    const SERIAL: &str = "0948AB000000001";

    #[derive(Default)]
    struct Recorder {
        states: Mutex<Vec<PrinterView>>,
        connections: Mutex<Vec<ConnectionState>>,
    }

    impl PrinterEvents for Recorder {
        fn state(&self, view: &PrinterView) {
            self.states.lock().unwrap().push(view.clone());
        }
        fn connection(&self, state: &ConnectionState) {
            self.connections.lock().unwrap().push(state.clone());
        }
    }

    fn config() -> PrinterConfig {
        PrinterConfig {
            ip: "127.0.0.1".into(),
            serial: SERIAL.into(),
            name: "Workshop".into(),
            model: "H2D".into(),
            pinned_fingerprint: None,
        }
    }

    fn service(dir: &tempfile::TempDir) -> (PrinterService, Arc<Recorder>) {
        let rec = Arc::new(Recorder::default());
        // Port 9 (discard) on loopback: the HMS fetch fails fast, offline.
        let svc = PrinterService::new(
            rec.clone(),
            Some(dir.path().join("history.db")),
            dir.path().to_path_buf(),
            "http://127.0.0.1:9/query.php",
            Timing::default(),
        );
        (svc, rec)
    }

    async fn wait_for(mut check: impl FnMut() -> bool) {
        for _ in 0..100 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition not met in 5 s");
    }

    #[tokio::test]
    async fn connects_and_emits_the_state_with_slots() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, "12345678", H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, rejection.clone()).unwrap();
        svc.start_with(
            config(),
            ClientParams {
                host: "127.0.0.1".into(),
                port: broker.addr.port(),
                serial: SERIAL.into(),
                access_code: "12345678".into(),
                tls: client_config(Arc::new(verifier)).unwrap(),
                rejection,
            },
        );
        wait_for(|| {
            rec.connections
                .lock()
                .unwrap()
                .contains(&ConnectionState::Connected)
        })
        .await;
        wait_for(|| svc.view().slots.len() == 10).await;
        let view = svc.view();
        assert!(view.configured);
        assert_eq!(view.connection, ConnectionState::Connected);
        assert_eq!(view.printer.as_ref().unwrap().serial, SERIAL);
        assert_eq!(view.state.as_ref().unwrap().mc_percent, Some(6));
        assert_eq!(view.errors.len(), 1, "the fixture has one HMS code");
        wait_for(|| {
            rec.states
                .lock()
                .unwrap()
                .iter()
                .any(|v| v.slots.len() == 10)
        })
        .await;
        svc.stop();
    }

    #[tokio::test]
    async fn a_burst_of_reports_is_emitted_at_most_twice_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        for _ in 0..30 {
            svc.handle_event(
                generation,
                ClientEvent::Report(H2D_DELTA_PROGRESS.as_bytes().to_vec()),
            );
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let n = rec.states.lock().unwrap().len();
        assert!((1..=3).contains(&n), "{n} state events in ~1 s");
        let last = rec.states.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state.unwrap().mc_percent, Some(7));
    }

    #[tokio::test]
    async fn malformed_reports_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(generation, ClientEvent::Report(b"{not json".to_vec()));
        assert!(svc.view().state.is_none());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        assert!(svc.view().state.is_some());
    }

    #[tokio::test]
    async fn events_from_a_stopped_connection_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let old = svc.configure_for_test(config());
        let new = svc.configure_for_test(config());
        svc.handle_event(old, ClientEvent::Report(H2D_FULL.as_bytes().to_vec()));
        assert!(svc.view().state.is_none());
        svc.handle_event(new, ClientEvent::Connection(ConnectionState::Connected));
        assert_eq!(svc.connection(), ConnectionState::Connected);
    }

    #[tokio::test]
    async fn assigning_a_slot_persists_and_shows_its_status() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );

        let view = svc
            .assign_slot(1, 1, "Acme PLA", Some("P4d6ae04"), None)
            .unwrap();
        let b2 = view.slots.iter().find(|s| s.label == "B2").unwrap();
        assert_eq!(b2.status, SlotStatus::Matches);
        let view = svc
            .assign_slot(0, 2, "Acme PETG", Some("P0000001"), None)
            .unwrap();
        let a3 = view.slots.iter().find(|s| s.label == "A3").unwrap();
        assert_eq!(a3.status, SlotStatus::Different);

        // A restart reloads assignments from the database.
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        assert_eq!(
            svc.view()
                .slots
                .iter()
                .filter(|s| s.assigned_preset.is_some())
                .count(),
            2
        );
        let view = svc.clear_slot(0, 2).unwrap();
        let a3 = view.slots.iter().find(|s| s.label == "A3").unwrap();
        assert_eq!(a3.status, SlotStatus::Unassigned);
    }

    #[tokio::test]
    async fn without_a_printer_assignments_are_refused_and_stop_clears_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        assert_eq!(
            svc.assign_slot(0, 0, "X", None, None).unwrap_err(),
            "No printer configured"
        );
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        svc.stop();
        let view = svc.view();
        assert!(!view.configured);
        assert!(view.state.is_none());
        assert_eq!(
            rec.connections.lock().unwrap().last(),
            Some(&ConnectionState::Disconnected)
        );
    }
}
```

Run: `cd src-tauri && cargo test --lib printer::service`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `PrinterEvents` and the other items not written yet.

- [ ] **Step 2: Write the service**

Insert above the `#[cfg(test)]` line of `src-tauri/src/printer/service.rs`:

```rust
//! Owns the printer connection. Keeps the latest state and connection
//! state, and emits `printer://state` (at most twice a second) and
//! `printer://connection`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{mpsc, watch};

use super::client::{self, ClientEvent, ClientParams, ConnectionState, Timing};
use super::hms::{ErrorView, HmsCatalog};
use super::settings::PrinterConfig;
use super::slots::{compute_slots, preset_needs_cloud_sync, SlotView};
use super::state::{parse_report, PrinterState, Report, ReportMerger};
use super::tls::{client_config, PrinterCertVerifier, RejectionSlot};
use crate::history::{RefinementHistory, SlotAssignment};

pub const STATE_EVENT: &str = "printer://state";
pub const CONNECTION_EVENT: &str = "printer://connection";
/// `printer://state` is emitted at most once per this interval.
const EMIT_INTERVAL: Duration = Duration::from_millis(500);

/// Where the service's events go: the webview in the app, a recorder in tests.
pub trait PrinterEvents: Send + Sync + 'static {
    fn state(&self, view: &PrinterView);
    fn connection(&self, state: &ConnectionState);
}

pub struct TauriEvents(pub tauri::AppHandle);

impl PrinterEvents for TauriEvents {
    fn state(&self, view: &PrinterView) {
        use tauri::Emitter;
        let _ = self.0.emit(STATE_EVENT, view);
    }
    fn connection(&self, state: &ConnectionState) {
        use tauri::Emitter;
        let _ = self.0.emit(CONNECTION_EVENT, state);
    }
}

/// The printer as the Printer page shows it. Payload of `printer://state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterView {
    pub configured: bool,
    pub printer: Option<PrinterSummary>,
    pub connection: ConnectionState,
    /// The last known state; kept while disconnected.
    pub state: Option<PrinterState>,
    pub slots: Vec<SlotView>,
    pub errors: Vec<ErrorView>,
}

impl PrinterView {
    /// The view when no printer is set up.
    pub fn unconfigured() -> Self {
        Self {
            configured: false,
            printer: None,
            connection: ConnectionState::Disconnected,
            state: None,
            slots: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterSummary {
    pub ip: String,
    pub serial: String,
    pub name: String,
    /// From `get_version` when connected, else from settings.
    pub model: String,
    pub firmware: Option<String>,
}

struct Live {
    config: Option<PrinterConfig>,
    connection: ConnectionState,
    merger: ReportMerger,
    state: Option<PrinterState>,
    model: Option<String>,
    firmware: Option<String>,
    assignments: Vec<SlotAssignment>,
    /// preset_path → needs cloud sync, refreshed when assignments load.
    cloud_sync: HashMap<String, bool>,
    hms: Option<Arc<HmsCatalog>>,
    hms_loading: bool,
    /// Bumped on every start and stop, so a stopped client's late events
    /// are ignored.
    generation: u64,
}

impl Live {
    fn empty(generation: u64) -> Self {
        Self {
            config: None,
            connection: ConnectionState::Disconnected,
            merger: ReportMerger::default(),
            state: None,
            model: None,
            firmware: None,
            assignments: Vec::new(),
            cloud_sync: HashMap::new(),
            hms: None,
            hms_loading: false,
            generation,
        }
    }
}

struct Inner {
    events: Arc<dyn PrinterEvents>,
    history_db: Option<PathBuf>,
    data_dir: PathBuf,
    hms_url: String,
    timing: Timing,
    live: Mutex<Live>,
    dirty: watch::Sender<u64>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct PrinterService {
    inner: Arc<Inner>,
}

impl PrinterService {
    /// `history_db` holds slot assignments; `data_dir` holds the HMS cache;
    /// `hms_url` is `hms::HMS_URL` in the app.
    pub fn new(
        events: Arc<dyn PrinterEvents>,
        history_db: Option<PathBuf>,
        data_dir: PathBuf,
        hms_url: &str,
        timing: Timing,
    ) -> Self {
        let (dirty, dirty_rx) = watch::channel(0u64);
        let inner = Arc::new(Inner {
            events,
            history_db,
            data_dir,
            hms_url: hms_url.to_string(),
            timing,
            live: Mutex::new(Live::empty(0)),
            dirty,
            tasks: Mutex::new(Vec::new()),
        });
        tauri::async_runtime::spawn(emit_loop(Arc::downgrade(&inner), dirty_rx));
        Self { inner }
    }

    /// Connects to the configured printer, replacing any running connection.
    pub fn start(&self, config: PrinterConfig, access_code: String) -> Result<(), String> {
        let rejection = RejectionSlot::default();
        let verifier = PrinterCertVerifier::bambu(
            &config.serial,
            config.pinned_fingerprint.as_deref(),
            rejection.clone(),
        )?;
        let params = ClientParams {
            host: config.ip.clone(),
            port: client::MQTT_PORT,
            serial: config.serial.clone(),
            access_code,
            tls: client_config(Arc::new(verifier))?,
            rejection,
        };
        self.start_with(config, params);
        Ok(())
    }

    /// `start` with explicit client parameters. Tests point it at a local broker.
    pub fn start_with(&self, config: PrinterConfig, params: ClientParams) {
        self.abort_tasks();
        let generation = self.configure(config);
        let (tx, rx) = mpsc::channel(256);
        let client = tauri::async_runtime::spawn(client::run(params, tx, self.inner.timing));
        let consumer = tauri::async_runtime::spawn(consume(self.clone(), generation, rx));
        self.inner.tasks.lock().unwrap().extend([client, consumer]);
    }

    /// Resets the live state for `config` and returns the new generation.
    fn configure(&self, config: PrinterConfig) -> u64 {
        let assignments = self.load_assignments(&config.serial);
        let cloud_sync = cloud_sync_map(&assignments);
        let generation = {
            let mut live = self.inner.live.lock().unwrap();
            let generation = live.generation + 1;
            *live = Live::empty(generation);
            live.hms = Some(Arc::new(HmsCatalog::with_base_url(
                self.inner.data_dir.clone(),
                Some(&config.serial),
                &self.inner.hms_url,
            )));
            live.config = Some(config);
            live.connection = ConnectionState::Connecting;
            live.assignments = assignments;
            live.cloud_sync = cloud_sync;
            generation
        };
        self.inner.events.connection(&ConnectionState::Connecting);
        self.mark_dirty();
        generation
    }

    /// Disconnects and forgets the printer's live state.
    pub fn stop(&self) {
        self.abort_tasks();
        {
            let mut live = self.inner.live.lock().unwrap();
            let generation = live.generation + 1;
            *live = Live::empty(generation);
        }
        self.inner.events.connection(&ConnectionState::Disconnected);
        self.mark_dirty();
    }

    fn abort_tasks(&self) {
        for task in self.inner.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    pub fn connection(&self) -> ConnectionState {
        self.inner.live.lock().unwrap().connection.clone()
    }

    pub fn config(&self) -> Option<PrinterConfig> {
        self.inner.live.lock().unwrap().config.clone()
    }

    pub fn view(&self) -> PrinterView {
        let live = self.inner.live.lock().unwrap();
        let model = live
            .model
            .clone()
            .or_else(|| live.config.as_ref().map(|c| c.model.clone()))
            .unwrap_or_default();
        let slots = live
            .state
            .as_ref()
            .map(|s| {
                compute_slots(s, &live.assignments, &|path| {
                    live.cloud_sync.get(path).copied().unwrap_or(false)
                })
            })
            .unwrap_or_default();
        let errors = match (&live.state, &live.hms) {
            (Some(s), Some(hms)) => hms.describe(s, Some(model.as_str())),
            _ => Vec::new(),
        };
        PrinterView {
            configured: live.config.is_some(),
            printer: live.config.as_ref().map(|c| PrinterSummary {
                ip: c.ip.clone(),
                serial: c.serial.clone(),
                name: c.name.clone(),
                model: model.clone(),
                firmware: live.firmware.clone(),
            }),
            connection: live.connection.clone(),
            state: live.state.clone(),
            slots,
            errors,
        }
    }

    /// Records which preset is loaded in a slot. `filament_id` is already
    /// resolved through `inherits`.
    pub fn assign_slot(
        &self,
        ams_id: u32,
        tray_id: u32,
        preset_name: &str,
        filament_id: Option<&str>,
        preset_path: Option<&str>,
    ) -> Result<PrinterView, String> {
        let serial = self.serial()?;
        self.history()?.assign_slot(
            &serial,
            ams_id,
            tray_id,
            preset_name,
            filament_id,
            preset_path,
        )?;
        self.reload_assignments(&serial);
        Ok(self.view())
    }

    pub fn clear_slot(&self, ams_id: u32, tray_id: u32) -> Result<PrinterView, String> {
        let serial = self.serial()?;
        self.history()?.clear_slot(&serial, ams_id, tray_id)?;
        self.reload_assignments(&serial);
        Ok(self.view())
    }

    /// Re-reads assignments and their presets' cloud-sync state, e.g. when
    /// the Printer page opens after the user synced in Bambu Studio.
    pub fn refresh_assignments(&self) {
        if let Ok(serial) = self.serial() {
            self.reload_assignments(&serial);
        }
    }

    fn serial(&self) -> Result<String, String> {
        self.config()
            .map(|c| c.serial)
            .ok_or_else(|| "No printer configured".to_string())
    }

    fn history(&self) -> Result<RefinementHistory, String> {
        let path = self
            .inner
            .history_db
            .as_ref()
            .ok_or("The app data folder is unavailable")?;
        RefinementHistory::new(path)
    }

    fn load_assignments(&self, serial: &str) -> Vec<SlotAssignment> {
        match self.history().and_then(|h| h.list_slot_assignments(serial)) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("could not load slot assignments: {e}");
                Vec::new()
            }
        }
    }

    fn reload_assignments(&self, serial: &str) {
        let assignments = self.load_assignments(serial);
        let cloud_sync = cloud_sync_map(&assignments);
        {
            let mut live = self.inner.live.lock().unwrap();
            live.assignments = assignments;
            live.cloud_sync = cloud_sync;
        }
        self.mark_dirty();
    }

    fn mark_dirty(&self) {
        self.inner.dirty.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Applies one client event if it belongs to the current connection.
    pub(crate) fn handle_event(&self, generation: u64, event: ClientEvent) {
        let mut load_hms: Option<Arc<HmsCatalog>> = None;
        {
            let mut live = self.inner.live.lock().unwrap();
            if live.generation != generation {
                return;
            }
            match event {
                ClientEvent::Connection(state) => {
                    live.connection = state.clone();
                    drop(live);
                    self.inner.events.connection(&state);
                    self.mark_dirty();
                    return;
                }
                ClientEvent::Report(bytes) => match parse_report(&bytes) {
                    Err(e) => {
                        tracing::debug!(serial = ?live_serial(&live), "skipping malformed printer report: {e}");
                        return;
                    }
                    Ok(Report::Status { print, full }) => {
                        let state = live.merger.apply(&print, full);
                        let has_errors = !state.hms.is_empty() || state.print_error.is_some();
                        live.state = Some(state);
                        if has_errors && !live.hms_loading {
                            if let Some(hms) = live.hms.clone().filter(|h| !h.is_loaded()) {
                                live.hms_loading = true;
                                load_hms = Some(hms);
                            }
                        }
                    }
                    Ok(Report::Version { model, firmware }) => {
                        if model.is_some() {
                            live.model = model;
                        }
                        live.firmware = firmware;
                    }
                    Ok(Report::Other) => return,
                },
            }
        }
        self.mark_dirty();
        if let Some(hms) = load_hms {
            let service = self.clone();
            tauri::async_runtime::spawn(async move {
                hms.ensure_loaded(chrono::Utc::now().timestamp()).await;
                service.inner.live.lock().unwrap().hms_loading = false;
                service.mark_dirty();
            });
        }
    }
```

Run: `cd src-tauri && cargo test --lib printer::service`
Expected: `test result: ok. 6 passed; 0 failed`. The throttle test takes about 1.1 s.

- [ ] **Step 3: Write the commands with their failing test**

In `src-tauri/src/commands/mod.rs`, add `pub mod printer;` after `pub mod models;`:

```rust
pub mod models;
pub mod printer;
pub mod profile;
```

Create `src-tauri/src/commands/printer.rs`:

```rust
//! Commands for Settings → Printer and the Printer page. The access code
//! comes in from the setup form and goes only to the keychain; no command
//! ever returns it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, State};

use crate::printer::client::{self, ClientParams, TestOutcome, Timing};
use crate::printer::discovery::{self, DiscoveredPrinter};
use crate::printer::service::{PrinterService, PrinterView};
use crate::printer::settings::{self, PrinterConfig, PrinterConfigView};
use crate::printer::slots;
use crate::printer::tls::{client_config, PrinterCertVerifier, RejectionSlot};
use crate::profile::{reader, BambuPaths, ProfileRegistry};

const DISCOVERY_WINDOW: Duration = Duration::from_secs(5);
const TEST_WAIT: Duration = Duration::from_secs(15);
const NEED_CODE: &str = "Enter the access code shown on the printer screen.";

/// The access code typed into the form, else the one in the keychain.
fn access_code_for(serial: &str, typed: Option<&str>) -> Result<String, String> {
    match typed.map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Ok(settings::check_access_code(code)?.to_string()),
        None => settings::get_access_code(serial)?.ok_or_else(|| NEED_CODE.to_string()),
    }
}

#[tauri::command]
pub fn printer_get_config(app: AppHandle) -> Result<Option<PrinterConfigView>, String> {
    let Some(config) = settings::load_config(&app) else {
        return Ok(None);
    };
    let has_code = matches!(settings::get_access_code(&config.serial), Ok(Some(_)));
    Ok(Some(PrinterConfigView::new(&config, has_code)))
}

/// Listens for printer announcements for five seconds. Sends nothing.
#[tauri::command]
pub async fn printer_discover() -> Result<Vec<DiscoveredPrinter>, String> {
    Ok(discovery::discover(discovery::DISCOVERY_PORTS, DISCOVERY_WINDOW).await)
}

#[tauri::command]
pub async fn printer_test_connection(
    ip: String,
    serial: String,
    access_code: Option<String>,
    pinned_fingerprint: Option<String>,
) -> Result<TestOutcome, String> {
    let config = PrinterConfig {
        ip,
        serial,
        pinned_fingerprint,
        ..Default::default()
    }
    .normalized()?;
    let code = access_code_for(&config.serial, access_code.as_deref())?;
    let rejection = RejectionSlot::default();
    let verifier = PrinterCertVerifier::bambu(
        &config.serial,
        config.pinned_fingerprint.as_deref(),
        rejection.clone(),
    )?;
    let params = ClientParams {
        host: config.ip.clone(),
        port: client::MQTT_PORT,
        serial: config.serial.clone(),
        access_code: code,
        tls: client_config(Arc::new(verifier))?,
        rejection,
    };
    tracing::info!(serial = %config.serial, ip = %config.ip, "testing the printer connection");
    Ok(client::test_connection(params, Timing::default(), TEST_WAIT).await)
}

/// Saves the printer and (re)starts the live connection.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn printer_save(
    app: AppHandle,
    service: State<'_, PrinterService>,
    ip: String,
    serial: String,
    name: Option<String>,
    model: Option<String>,
    access_code: Option<String>,
    pinned_fingerprint: Option<String>,
) -> Result<PrinterConfigView, String> {
    let config = PrinterConfig {
        ip,
        serial,
        name: name.unwrap_or_default(),
        model: model.unwrap_or_default(),
        pinned_fingerprint,
    }
    .normalized()?;
    let code = access_code_for(&config.serial, access_code.as_deref())?;
    if access_code.as_deref().is_some_and(|c| !c.trim().is_empty()) {
        settings::set_access_code(&config.serial, &code)?;
    }
    if let Some(previous) = settings::load_config(&app).filter(|p| p.serial != config.serial) {
        if let Err(e) = settings::delete_access_code(&previous.serial) {
            tracing::warn!("could not remove the old printer's access code: {e}");
        }
    }
    settings::save_config(&app, &config)?;
    tracing::info!(serial = %config.serial, ip = %config.ip, "printer saved");
    service.start(config.clone(), code)?;
    Ok(PrinterConfigView::new(&config, true))
}

#[tauri::command]
pub fn printer_remove(app: AppHandle, service: State<'_, PrinterService>) -> Result<(), String> {
    service.stop();
    if let Some(config) = settings::load_config(&app) {
        settings::delete_access_code(&config.serial)?;
    }
    settings::remove_config(&app)
}

/// The current view. Also re-checks assigned presets' cloud-sync state.
#[tauri::command]
pub fn printer_view(service: State<'_, PrinterService>) -> PrinterView {
    service.refresh_assignments();
    service.view()
}

#[tauri::command]
pub async fn printer_assign_slot(
    service: State<'_, PrinterService>,
    ams_id: u32,
    tray_id: u32,
    preset_path: String,
) -> Result<PrinterView, String> {
    let path = PathBuf::from(&preset_path);
    let (name, filament_id) = tauri::async_runtime::spawn_blocking(move || resolve_preset(&path))
        .await
        .map_err(|e| e.to_string())??;
    service.assign_slot(
        ams_id,
        tray_id,
        &name,
        filament_id.as_deref(),
        Some(&preset_path),
    )
}

#[tauri::command]
pub fn printer_clear_slot(
    service: State<'_, PrinterService>,
    ams_id: u32,
    tray_id: u32,
) -> Result<PrinterView, String> {
    service.clear_slot(ams_id, tray_id)
}

/// A preset's name and filament id. Only presets in Bambu Studio's system
/// or user filament folders are read.
fn resolve_preset(path: &Path) -> Result<(String, Option<String>), String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {e}"))?;
    let system_dir = paths.system_filament_dir();
    let user_dir = paths.user_filament_dir();
    let allowed: Vec<PathBuf> = std::iter::once(system_dir.clone())
        .chain(user_dir.clone())
        .collect();
    if !is_within_any(path, &allowed) {
        return Err("That preset isn't in Bambu Studio's filament folders.".into());
    }
    let profile =
        reader::read_profile(path).map_err(|e| format!("Could not read the preset: {e}"))?;
    let name = profile.name().ok_or("The preset has no name")?.to_string();
    let mut registry = ProfileRegistry::new();
    if profile.filament_id().is_none_or(|id| id.trim().is_empty()) {
        registry = ProfileRegistry::discover_system_profiles(&system_dir)
            .unwrap_or_else(|_| ProfileRegistry::new());
        if let Some(dir) = &user_dir {
            let _ = registry.discover_user_profiles(dir);
        }
    }
    Ok((name, slots::resolve_filament_id(&profile, &registry)))
}

fn is_within_any(path: &Path, dirs: &[PathBuf]) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    dirs.iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| path.starts_with(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_inside_the_filament_folders_are_accepted() {
        let root = tempfile::tempdir().unwrap();
        let inside_dir = root.path().join("user/1/filament");
        std::fs::create_dir_all(&inside_dir).unwrap();
        let inside = inside_dir.join("A.json");
        std::fs::write(&inside, "{}").unwrap();
        let outside = root.path().join("B.json");
        std::fs::write(&outside, "{}").unwrap();
        let dirs = vec![inside_dir.clone()];
        assert!(is_within_any(&inside, &dirs));
        assert!(!is_within_any(&outside, &dirs));
        assert!(!is_within_any(&inside_dir.join("../../../B.json"), &dirs));
        assert!(!is_within_any(&inside_dir.join("missing.json"), &dirs));
    }
}
```

Run: `cd src-tauri && cargo test --lib commands::printer`
Expected: `test result: ok. 1 passed; 0 failed`. The commands compile but aren't registered yet.

- [ ] **Step 4: Register the commands and start the service at launch**

In `src-tauri/src/lib.rs`, add to `tauri::generate_handler![…]` after `commands::agent::agent_set_settings,`:

```rust
            commands::printer::printer_get_config,
            commands::printer::printer_discover,
            commands::printer::printer_test_connection,
            commands::printer::printer_save,
            commands::printer::printer_remove,
            commands::printer::printer_view,
            commands::printer::printer_assign_slot,
            commands::printer::printer_clear_slot,
```

In the `.setup(|app| { … })` closure, after the `// -- Agent backends --` block's closing `}` and before `Ok(())`, add:

```rust

            // -- Printer live connection -------------------------------------
            {
                use std::sync::Arc;

                let data_dir = app.path().app_data_dir().ok();
                let service = printer::service::PrinterService::new(
                    Arc::new(printer::service::TauriEvents(app.handle().clone())),
                    data_dir.as_ref().map(|d| d.join("refinement_history.db")),
                    data_dir.unwrap_or_else(std::env::temp_dir),
                    printer::hms::HMS_URL,
                    printer::client::Timing::default(),
                );
                if let Some(config) = printer::settings::load_config(app.handle()) {
                    match printer::settings::get_access_code(&config.serial) {
                        Ok(Some(code)) => {
                            if let Err(e) = service.start(config, code) {
                                tracing::warn!("printer connection not started: {e}");
                            }
                        }
                        Ok(None) => tracing::info!("printer configured without an access code"),
                        Err(e) => tracing::warn!("printer access code unavailable: {e}"),
                    }
                }
                app.manage(service);
            }
```

`tauri::Manager` is already in scope there, imported by the STL watcher block.

- [ ] **Step 5: Remove the printer on Reset for Clean Installation**

In `src-tauri/src/commands/config.rs` `reset_to_clean_install`, directly after `info!("Resetting BambuMate to clean installation state");`, add:

```rust

    // The printer's access code is keyed by its serial, which lives in the
    // store about to be cleared, so remove it first.
    let mut keychain_errors = Vec::new();
    if let Some(printer) = crate::printer::settings::load_config(&app) {
        if let Err(e) = crate::printer::settings::delete_access_code(&printer.serial) {
            keychain_errors.push(format!("printer access code: {}", e));
        }
    }
    {
        use tauri::Manager;
        if let Some(service) = app.try_state::<crate::printer::service::PrinterService>() {
            service.stop();
        }
    }
```

Then delete the now-duplicate declaration below `// Delete all API keys from the system keychain`:

```rust
    let mut keychain_errors = Vec::new();
```

- [ ] **Step 6: Run the whole backend suite**

Run: `cd src-tauri && cargo test`
Expected: every test binary reports `ok`; the lib reports `test result: ok. 471 passed; 0 failed`. The count is 403 on `main` plus 68 new, assuming nothing else landed.

Run: `cd src-tauri && cargo test --features claude-subscription 2>&1 | grep -E "test result|FAILED"`
Expected: only `ok` lines.

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -A3 -E "^(warning|error)" | grep -E "src/(printer|commands/printer|history/store|lib)"`
Expected: no output (no new clippy findings in the files this plan touches).

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/printer/mod.rs src-tauri/src/printer/service.rs src-tauri/src/commands/printer.rs src-tauri/src/commands/mod.rs src-tauri/src/commands/config.rs src-tauri/src/lib.rs
git commit -m "Run the printer service, emit printer events and add the printer commands"
```

---

### Task 8: Read-only agent tools `bm_printer_status` and `bm_ams_slots`

**Files:**
- Create: `src-tauri/src/agent/tools/printer.rs`
- Modify: `src-tauri/src/agent/tools/mod.rs` (module list, `ToolHost::printer_view`, `specs`, `call`)
- Modify: `src-tauri/src/agent/tools/fake_host.rs`, `src-tauri/src/agent/host.rs`
- Modify: `src-tauri/src/agent/tools/app.rs` (`/printer` route, tool count 18 → 20)
- Modify: `src-tauri/src/agent/mod.rs` (`AGENT_INSTRUCTIONS`)
- Modify: `src-tauri/src/agent/codex/mod.rs:996` (dynamic tool count 18 → 20), `src-tauri/src/agent/claude/mcp_server.rs:327` (MCP tool count 19 → 21)

**Interfaces:**
- Consumes (Task 7):
  - `service::{PrinterService, PrinterView, PrinterSummary}` and `PrinterView::unconfigured()`
  - `client::ConnectionState`, `slots::{set_on_printer_steps, SlotStatus, compute_slots}`, `hms::ErrorView`
- Produces:
  - `ToolHost::printer_view(&self) -> crate::printer::service::PrinterView` (a new trait method)
  - In `crate::agent::tools::printer`:
    - `pub const NO_PRINTER: &str = "No printer configured"`, `pub const NOT_CONNECTED: &str = "Printer not connected"`
    - `pub fn specs() -> Vec<ToolSpec>`
    - `pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput>`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/agent/tools/mod.rs`, add the module after `pub mod interact;`:

```rust
pub mod interact;
pub mod printer;
pub mod profiles;
```

Add to `trait ToolHost`, after `fn bambu_studio_running(&self) -> bool;`:

```rust
    /// The live printer view. The printer tools strip its IP before
    /// anything reaches the agent.
    fn printer_view(&self) -> crate::printer::service::PrinterView;
```

In `ToolRegistry::specs`, after `all.extend(app::specs());`, add `all.extend(printer::specs());`. In `ToolRegistry::call`, after the `app::handle` block, add:

```rust
        if let Some(out) = printer::handle(self, name, &args).await {
            return out;
        }
```

In `src-tauri/src/agent/tools/fake_host.rs`:
- Add `use crate::printer::service::PrinterView;` after the `crate::agent::types` import.
- Add the field `pub printer: Mutex<PrinterView>,` after `pub bs_running: AtomicBool,`.
- Initialise it in `FakeHost::new` with `printer: Mutex::new(PrinterView::unconfigured()),`.
- Implement the method after `bambu_studio_running`:

```rust
    fn printer_view(&self) -> PrinterView {
        self.printer.lock().unwrap().clone()
    }
```

In `src-tauri/src/agent/host.rs`, in `impl ToolHost for TauriToolHost`, after `bambu_studio_running`, add:

```rust
    fn printer_view(&self) -> crate::printer::service::PrinterView {
        use tauri::Manager;
        self.app
            .try_state::<crate::printer::service::PrinterService>()
            .map(|s| s.view())
            .unwrap_or_else(crate::printer::service::PrinterView::unconfigured)
    }
```

Create `src-tauri/src/agent/tools/printer.rs` with only the test code for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::history::SlotAssignment;
    use crate::printer::hms::ErrorView;
    use crate::printer::service::PrinterSummary;
    use crate::printer::slots::compute_slots;
    use crate::printer::state::fixtures::{state_after, H2D_FULL};
    use std::sync::Arc;

    fn connected_view() -> PrinterView {
        let state = state_after(&[H2D_FULL]);
        let assignments = vec![SlotAssignment {
            serial: "0948AB000000001".into(),
            ams_id: 0,
            tray_id: 2,
            preset_name: "Acme PETG".into(),
            filament_id: Some("P0000001".into()),
            preset_path: Some("/u/Acme PETG.json".into()),
            assigned_at: "2026-09-28 10:00:00".into(),
        }];
        PrinterView {
            configured: true,
            printer: Some(PrinterSummary {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                name: "Workshop".into(),
                model: "H2D".into(),
                firmware: Some("01.01.01.00".into()),
            }),
            connection: ConnectionState::Connected,
            slots: compute_slots(&state, &assignments, &|_| true),
            state: Some(state),
            errors: vec![ErrorView {
                kind: "hms".into(),
                code: "0300_0100_0001_0007".into(),
                text: Some("The heatbed temperature is abnormal.".into()),
                wiki_url:
                    "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0100_0001_0007"
                        .into(),
            }],
        }
    }

    async fn call(view: PrinterView, tool: &str) -> String {
        let host = Arc::new(FakeHost::new());
        *host.printer.lock().unwrap() = view;
        let (reg, _rx) = registry_with(host);
        let out = reg.call(tool, json!({})).await;
        assert!(out.ok);
        match &out.content[0] {
            crate::agent::tools::ToolContent::Text(t) => t.clone(),
            other => panic!("unexpected content {other:?}"),
        }
    }

    #[tokio::test]
    async fn both_tools_say_when_no_printer_is_configured() {
        for tool in ["bm_printer_status", "bm_ams_slots"] {
            assert_eq!(call(PrinterView::unconfigured(), tool).await, NO_PRINTER);
        }
    }

    #[tokio::test]
    async fn both_tools_say_when_the_printer_is_not_connected() {
        let mut view = connected_view();
        view.connection = ConnectionState::Unreachable;
        for tool in ["bm_printer_status", "bm_ams_slots"] {
            assert_eq!(call(view.clone(), tool).await, NOT_CONNECTED);
        }
    }

    #[tokio::test]
    async fn status_reports_the_print_temperatures_and_errors_without_the_ip() {
        let text = call(connected_view(), "bm_printer_status").await;
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["print"]["state"], "RUNNING");
        assert_eq!(v["print"]["percent"], 6);
        assert_eq!(v["temperatures"]["nozzles"][0]["nozzle"], "right");
        assert_eq!(v["temperatures"]["nozzles"][1]["nozzle"], "left");
        assert_eq!(v["temperatures"]["active_nozzle"], "right");
        assert_eq!(v["errors"][0]["code"], "0300_0100_0001_0007");
        assert_eq!(v["printer"]["model"], "H2D");
        assert!(
            !text.contains("192.168.1.20"),
            "the IP must not reach the agent"
        );
    }

    #[tokio::test]
    async fn slots_list_status_and_steps() {
        let text = call(connected_view(), "bm_ams_slots").await;
        let v: Value = serde_json::from_str(&text).unwrap();
        let slots = v["slots"].as_array().unwrap();
        assert_eq!(slots.len(), 10);
        assert_eq!(slots[1]["slot"], "A2");
        assert_eq!(slots[1]["status"], "bambu_spool");
        assert_eq!(slots[1]["remaining_percent"], 31);
        assert_eq!(slots[2]["status"], "set_on_printer");
        assert_eq!(slots[2]["assigned_preset"], "Acme PETG");
        assert_eq!(slots[2]["steps"].as_array().unwrap().len(), 3);
        assert!(slots[2]["remaining_percent"].is_null());
        assert_eq!(slots[7]["status"], "empty");
        assert!(slots[7]["reported"].is_null());
        assert_eq!(slots[8]["slot"], "Ext-L");
        assert!(!text.contains("192.168.1.20"));
    }
}
```

Run: `cd src-tauri && cargo test --lib agent::tools::printer`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `specs` and the other items not written yet.

- [ ] **Step 2: Write the tools**

Insert above the `#[cfg(test)]` line of `src-tauri/src/agent/tools/printer.rs`:

```rust
//! Read-only printer tools. They never include the printer's IP address or
//! access code, and there are no write or control tools.

use serde_json::{json, Value};

use super::{ToolOutput, ToolRegistry, ToolSpec};
use crate::printer::client::ConnectionState;
use crate::printer::service::PrinterView;
use crate::printer::slots::{set_on_printer_steps, SlotStatus};

pub const NO_PRINTER: &str = "No printer configured";
pub const NOT_CONNECTED: &str = "Printer not connected";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_printer_status",
            description: "Read-only status of the user's Bambu printer: connection, the current print (state, file, progress, layers, time left), bed and nozzle temperatures, active errors with their text, and model/serial.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolSpec {
            name: "bm_ams_slots",
            description: "Read-only list of every AMS slot and external spool: label (A1-D4, HT1, Ext-L/Ext-R), the filament the printer reports, whether it is a Bambu RFID spool, remaining %, the preset the user assigned in BambuMate, and whether the printer's setting matches it (with the steps to fix it when it doesn't).",
            input_schema: json!({"type":"object","properties":{}}),
        },
    ]
}

pub async fn handle(reg: &ToolRegistry, name: &str, _args: &Value) -> Option<ToolOutput> {
    match name {
        "bm_printer_status" => Some(status(&reg.host().printer_view())),
        "bm_ams_slots" => Some(slots(&reg.host().printer_view())),
        _ => None,
    }
}

fn unavailable(view: &PrinterView) -> Option<ToolOutput> {
    if !view.configured {
        return Some(ToolOutput::text(NO_PRINTER));
    }
    if view.connection != ConnectionState::Connected {
        return Some(ToolOutput::text(NOT_CONNECTED));
    }
    None
}

/// On the H2 series extruder 0 is the right nozzle and 1 the left.
fn nozzle_name(id: u32, dual: bool) -> Value {
    match (dual, id) {
        (true, 0) => json!("right"),
        (true, 1) => json!("left"),
        _ => json!(id),
    }
}

fn status(view: &PrinterView) -> ToolOutput {
    if let Some(out) = unavailable(view) {
        return out;
    }
    let s = view.state.clone().unwrap_or_default();
    let dual = s.nozzles.len() >= 2;
    let printer = view.printer.as_ref();
    ToolOutput::json(&json!({
        "connection": "connected",
        "printer": {
            "model": printer.map(|p| p.model.clone()),
            "serial": printer.map(|p| p.serial.clone()),
            "name": printer.map(|p| p.name.clone()),
            "firmware": printer.and_then(|p| p.firmware.clone()),
        },
        "print": {
            "state": s.gcode_state,
            "file": s.subtask_name,
            "percent": s.mc_percent,
            "layer": s.layer_num,
            "total_layers": s.total_layer_num,
            "remaining_minutes": s.mc_remaining_time,
        },
        "temperatures": {
            "bed": {"current": s.bed_temp, "target": s.bed_target_temp},
            "nozzles": s.nozzles.iter().map(|n| json!({
                "nozzle": nozzle_name(n.id, dual),
                "current": n.temp,
                "target": n.target_temp,
                "diameter": n.diameter,
                "type": n.nozzle_type,
            })).collect::<Vec<_>>(),
            "active_nozzle": s.active_nozzle.map(|id| nozzle_name(id, dual)),
        },
        "errors": view.errors.iter().map(|e| json!({
            "code": e.code,
            "text": e.text,
            "wiki_url": e.wiki_url,
        })).collect::<Vec<_>>(),
    }))
}

fn status_name(status: SlotStatus) -> &'static str {
    match status {
        SlotStatus::Matches => "set",
        SlotStatus::Rfid => "bambu_spool",
        SlotStatus::Different => "set_on_printer",
        SlotStatus::Empty => "empty",
        SlotStatus::Unassigned => "not_set",
    }
}

fn slots(view: &PrinterView) -> ToolOutput {
    if let Some(out) = unavailable(view) {
        return out;
    }
    let list: Vec<Value> = view
        .slots
        .iter()
        .map(|s| {
            let reported = (!s.tray.empty).then(|| {
                json!({
                    "type": s.tray.tray_type,
                    "brand": s.tray.tray_sub_brands,
                    "color": s.tray.tray_color,
                    "filament_id": s.tray.tray_info_idx,
                })
            });
            let steps = match (&s.status, &s.assigned_preset) {
                (SlotStatus::Different, Some(preset)) => {
                    set_on_printer_steps(&s.label, preset, s.needs_cloud_sync)
                }
                _ => Vec::new(),
            };
            json!({
                "slot": s.label,
                "reported": reported,
                "rfid": s.rfid,
                "remaining_percent": if s.rfid { s.tray.remain } else { None },
                "assigned_preset": s.assigned_preset,
                "status": status_name(s.status),
                "steps": steps,
            })
        })
        .collect();
    ToolOutput::json(&json!({ "slots": list }))
}
```

- [ ] **Step 3: Add the route, the instructions line, and the new tool counts**

In `src-tauri/src/agent/tools/app.rs`:
- Add `"/printer",` as the last entry of `ROUTES`, after `"/about",`.
- End the `bm_navigate` description with `…, /health, /about, /printer.`.
- In `total_tool_count_stays_small`, change `18` to `20`.

In `src-tauri/src/agent/mod.rs` `AGENT_INSTRUCTIONS`, after the `- Use bm_todo …` line, add:

```rust
- bm_printer_status and bm_ams_slots read the user's printer live. They are \
read-only: you cannot control the printer or change what is loaded in a slot.
```

In `src-tauri/src/agent/codex/mod.rs`, in `async fn started`, change:

```rust
            assert_eq!(ts["params"]["dynamicTools"].as_array().unwrap().len(), 18);
```

to:

```rust
            assert_eq!(ts["params"]["dynamicTools"].as_array().unwrap().len(), 20);
```

In `src-tauri/src/agent/claude/mcp_server.rs`, in `lists_registry_tools_plus_permission_tool`, change `assert_eq!(names.len(), 19);` to `assert_eq!(names.len(), 21);`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib agent::`
Expected: `test result: ok.` with 0 failed. The new `agent::tools::printer` module has 4 tests.

Run: `cd src-tauri && cargo test`
Expected: all `ok`; the lib reports 475 passed.

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/tools/printer.rs src-tauri/src/agent/tools/mod.rs src-tauri/src/agent/tools/fake_host.rs src-tauri/src/agent/tools/app.rs src-tauri/src/agent/host.rs src-tauri/src/agent/mod.rs src-tauri/src/agent/codex/mod.rs src-tauri/src/agent/claude/mcp_server.rs
git commit -m "Give the agent read-only printer status and AMS slot tools"
```

---

### Task 9: Frontend printer state and Settings → Printer

**Files:**
- Create: `src/printer/mod.rs`, `src/printer/types.rs`, `src/printer/bridge.rs`
- Create: `src/components/printer_settings.rs`
- Create: `style/printer.css` (Settings part only; Task 10 replaces it with the full file)
- Modify: `src/main.rs` (module list), `src/app.rs` (context and `<PrinterEvents />`), `src/components/mod.rs`, `src/pages/settings.rs` (render the section), `index.html` (stylesheet)
- Modify: `tests/webkit/fixtures.mjs` (printer fixtures), `tests/webkit/app-flows.mjs` (`window.__fixtures`, Settings → Printer steps)

**Interfaces:**
- Consumes: the Task 7 command names and JSON shapes (`PrinterView`, `PrinterConfigView`, `DiscoveredPrinter`, `TestOutcome`, `ConnectionState` as `{"state": "..."}`), and the events `printer://state` (payload `PrinterView`) and `printer://connection` (payload `ConnectionState`). Also `crate::agent::bridge::listen`.
- Produces:
  - In `crate::printer::types`, mirror types that all `#[serde(default)]`:
    - `ConnectionState` (with `dot()`), `PrinterView`, `PrinterSummary`, `PrinterState`, `Nozzle`, `AmsUnit`, `Tray`
    - `SlotStatus` (with `badge()` and `key()`), `SlotView` (with `preset_name()`), `ErrorView`, `PrinterConfigView`, `DiscoveredPrinter`, `TestOutcome`
    - `connection_message(&ConnectionState, ip: &str) -> Option<String>`, `row_title(u32) -> String`, `format_remaining(u32) -> String`, `format_temp(Option<f64>, Option<f64>) -> String`, `nozzle_label(u32, usize) -> &'static str`, `swatch_color(&str) -> String`
  - In `crate::printer::bridge`:
    - `get_config()`, `discover()`
    - `test_connection(ip, serial, access_code, pinned_fingerprint)`
    - `save(ip, serial, name, model, access_code, pinned_fingerprint)`
    - `remove()`, `view()`, `assign_slot(ams_id, tray_id, preset_path)`, `clear_slot(ams_id, tray_id)`
    - All take `&str` and return `Result<_, String>`; a blank string is sent as `None`.
  - In `crate::printer`: `#[derive(Clone, Copy)] pub struct PrinterShared { pub view: RwSignal<PrinterView> }` with `new()` and `refresh(self)`, and `#[component] pub fn PrinterEvents()`.
  - `crate::components::printer_settings::{PrinterSettings, outcome_message}`.
  - In `tests/webkit/fixtures.mjs`:
    - `PRINTER_SERIAL`, `PRINTER_FINGERPRINT`, `PRINTER_SLOTS`, `PRINTER_VIEW`, `PRINTER_UNCONFIGURED`, `A3_ASSIGNED`
    - `withSlot(label, patch)`
    - Eight `printer_*` fixtures
  - In `tests/webkit/app-flows.mjs`: `window.__fixtures` (the live fixture map), and the helpers `ipcCalls(cmd)` and `setFixture(cmd, value)` inside `driveApp`.

- [ ] **Step 1: Write the failing frontend tests**

In `src/main.rs`, add `mod printer;` after `mod pages;`:

```rust
mod pages;
mod printer;
mod theme;
```

Create `src/printer/mod.rs`:

```rust
//! Frontend side of the live printer connection: shared state fed by the
//! `printer://state` and `printer://connection` events.

pub mod bridge;
pub mod types;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use types::{ConnectionState, PrinterView};

/// The latest printer view, shared by the rail dot, the Printer page and
/// Settings → Printer.
#[derive(Clone, Copy)]
pub struct PrinterShared {
    pub view: RwSignal<PrinterView>,
}

impl PrinterShared {
    pub fn new() -> Self {
        Self {
            view: RwSignal::new(PrinterView::default()),
        }
    }

    /// Re-reads the view from the backend.
    pub fn refresh(self) {
        spawn_local(async move {
            if let Ok(v) = bridge::view().await {
                self.view.set(v);
            }
        });
    }
}

impl Default for PrinterShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Registers the printer event listeners. Like `AgentEvents`, mount it
/// exactly once, outside anything that can unmount.
#[component]
pub fn PrinterEvents() -> impl IntoView {
    let shared = expect_context::<PrinterShared>();
    crate::agent::bridge::listen::<PrinterView>("printer://state", move |v| shared.view.set(v));
    crate::agent::bridge::listen::<ConnectionState>("printer://connection", move |c| {
        shared.view.update(|v| v.connection = c)
    });
    shared.refresh();
}
```

Create `src/printer/types.rs` with only the test code for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_states_deserialize_from_the_backend_shape() {
        let s: ConnectionState =
            serde_json::from_str(r#"{"state":"cert_untrusted","fingerprint":"AB:CD"}"#).unwrap();
        assert_eq!(
            s,
            ConnectionState::CertUntrusted {
                fingerprint: "AB:CD".into()
            }
        );
        let s: ConnectionState = serde_json::from_str(r#"{"state":"auth_failed"}"#).unwrap();
        assert_eq!(s.dot(), "error");
        assert_eq!(ConnectionState::Connected.dot(), "connected");
        assert_eq!(ConnectionState::Connecting.dot(), "connecting");
    }

    #[test]
    fn messages_use_the_spec_copy() {
        assert_eq!(
            connection_message(&ConnectionState::AuthFailed, "10.0.0.2").unwrap(),
            "The access code was rejected. Check it on the printer screen (Settings → LAN)."
        );
        assert_eq!(
            connection_message(&ConnectionState::Unreachable, "10.0.0.2").unwrap(),
            "Can't reach the printer at 10.0.0.2."
        );
        assert_eq!(connection_message(&ConnectionState::Connected, "x"), None);
    }

    #[test]
    fn badges_use_the_spec_copy() {
        assert_eq!(SlotStatus::Matches.badge(), "✓ Set");
        assert_eq!(SlotStatus::Rfid.badge(), "✓ Bambu spool");
        assert_eq!(SlotStatus::Different.badge(), "Set on printer");
        assert_eq!(SlotStatus::Empty.badge(), "Empty");
        assert_eq!(SlotStatus::Unassigned.badge(), "Not set");
    }

    #[test]
    fn a_view_with_missing_fields_still_deserializes() {
        let v: PrinterView = serde_json::from_str(
            r#"{"configured":true,"connection":{"state":"connected"},
                "slots":[{"label":"A1","status":"rfid","tray":{"tray_type":"PLA"},"future":1}]}"#,
        )
        .unwrap();
        assert!(v.configured);
        assert_eq!(v.slots[0].status, SlotStatus::Rfid);
        assert_eq!(v.slots[0].tray.tray_type, "PLA");
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(row_title(0), "AMS A");
        assert_eq!(row_title(3), "AMS D");
        assert_eq!(row_title(128), "AMS HT1");
        assert_eq!(row_title(255), "External");
        assert_eq!(format_remaining(549), "9 h 09 m");
        assert_eq!(format_remaining(9), "9 m");
        assert_eq!(format_temp(Some(245.4), Some(250.0)), "245 / 250 °C");
        assert_eq!(format_temp(None, Some(0.0)), "— / 0 °C");
        assert_eq!(nozzle_label(0, 2), "Right nozzle");
        assert_eq!(nozzle_label(1, 2), "Left nozzle");
        assert_eq!(nozzle_label(0, 1), "Nozzle");
        assert_eq!(swatch_color("F95959FF"), "#f95959ff");
        assert_eq!(swatch_color("nope"), "transparent");
    }

    #[test]
    fn a_card_prefers_the_assigned_preset_name() {
        let mut s = SlotView {
            tray: Tray {
                tray_sub_brands: "PLA Basic".into(),
                tray_info_idx: "GFA00".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(s.preset_name(), "PLA Basic");
        s.assigned_preset = Some("Acme PLA".into());
        assert_eq!(s.preset_name(), "Acme PLA");
    }
}
```

Run: `cargo test --bin bambumate printer::types`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `ConnectionState` and the other items not written yet. `bridge.rs` is missing too.

- [ ] **Step 2: Write the types and the invoke wrappers**

Insert above the `#[cfg(test)]` line of `src/printer/types.rs`:

```rust
//! Mirrors of the backend's printer types (`src-tauri/src/printer/`) and
//! the copy the printer UI shows. Fields default when missing, so a newer
//! backend field never breaks deserialization.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    AuthFailed,
    CertUntrusted {
        fingerprint: String,
    },
    WrongSerial {
        presented: String,
    },
    Unreachable,
}

impl ConnectionState {
    /// `data-state` for the rail dot: connected, connecting or error.
    pub fn dot(&self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Connecting | Self::Disconnected => "connecting",
            _ => "error",
        }
    }
}

/// What to tell the user about a connection state; `None` when connected.
pub fn connection_message(state: &ConnectionState, ip: &str) -> Option<String> {
    match state {
        ConnectionState::Connected => None,
        ConnectionState::Connecting => Some("Connecting to the printer…".into()),
        ConnectionState::Disconnected => Some("Not connected.".into()),
        ConnectionState::AuthFailed => Some(
            "The access code was rejected. Check it on the printer screen (Settings → LAN)."
                .into(),
        ),
        ConnectionState::Unreachable => Some(format!("Can't reach the printer at {ip}.")),
        ConnectionState::CertUntrusted { .. } => Some(
            "The printer's certificate isn't from a Bambu CA that BambuMate knows. Trust it in Settings → Printer."
                .into(),
        ),
        ConnectionState::WrongSerial { presented } => Some(format!(
            "The printer at {ip} reports serial {presented}. Check the serial in Settings → Printer."
        )),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterView {
    pub configured: bool,
    pub printer: Option<PrinterSummary>,
    pub connection: ConnectionState,
    pub state: Option<PrinterState>,
    pub slots: Vec<SlotView>,
    pub errors: Vec<ErrorView>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterSummary {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    pub firmware: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterState {
    pub gcode_state: Option<String>,
    pub subtask_name: Option<String>,
    pub mc_percent: Option<u32>,
    pub mc_remaining_time: Option<u32>,
    pub layer_num: Option<u32>,
    pub total_layer_num: Option<u32>,
    pub bed_temp: Option<f64>,
    pub bed_target_temp: Option<f64>,
    pub nozzles: Vec<Nozzle>,
    pub active_nozzle: Option<u32>,
    pub ams_units: Vec<AmsUnit>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Nozzle {
    pub id: u32,
    pub temp: Option<f64>,
    pub target_temp: Option<f64>,
    pub diameter: Option<f64>,
    pub nozzle_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AmsUnit {
    pub id: u32,
    pub humidity_level: Option<u32>,
    pub humidity_pct: Option<u32>,
    pub temp: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Tray {
    pub id: u32,
    pub empty: bool,
    pub tray_type: String,
    pub tray_color: String,
    pub tray_info_idx: String,
    pub tray_sub_brands: String,
    pub remain: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStatus {
    Matches,
    Rfid,
    Different,
    Empty,
    #[default]
    Unassigned,
}

impl SlotStatus {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Matches => "✓ Set",
            Self::Rfid => "✓ Bambu spool",
            Self::Different => "Set on printer",
            Self::Empty => "Empty",
            Self::Unassigned => "Not set",
        }
    }
    /// `data-status` on the slot card.
    pub fn key(self) -> &'static str {
        match self {
            Self::Matches => "matches",
            Self::Rfid => "rfid",
            Self::Different => "different",
            Self::Empty => "empty",
            Self::Unassigned => "unassigned",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SlotView {
    pub ams_id: u32,
    pub tray_id: u32,
    pub label: String,
    pub tray: Tray,
    pub rfid: bool,
    pub assigned_preset: Option<String>,
    pub assigned_filament_id: Option<String>,
    pub status: SlotStatus,
    pub needs_cloud_sync: bool,
}

impl SlotView {
    /// The preset name a card shows: the assigned one, else what the
    /// printer reports.
    pub fn preset_name(&self) -> String {
        if let Some(p) = &self.assigned_preset {
            return p.clone();
        }
        if !self.tray.tray_sub_brands.is_empty() {
            return self.tray.tray_sub_brands.clone();
        }
        self.tray.tray_info_idx.clone()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ErrorView {
    pub kind: String,
    pub code: String,
    pub text: Option<String>,
    pub wiki_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterConfigView {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    pub pinned_fingerprint: Option<String>,
    pub has_access_code: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct DiscoveredPrinter {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TestOutcome {
    pub connection: ConnectionState,
    pub got_report: bool,
    pub model: Option<String>,
}

/// `AMS A`, `AMS HT1` or `External`, for a row of slot cards.
pub fn row_title(ams_id: u32) -> String {
    match ams_id {
        255 => "External".into(),
        id if id >= 128 => format!("AMS HT{}", id - 127),
        id => format!("AMS {}", char::from_u32('A' as u32 + id).unwrap_or('?')),
    }
}

/// `1 h 05 m` or `9 m`.
pub fn format_remaining(minutes: u32) -> String {
    if minutes >= 60 {
        format!("{} h {:02} m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} m")
    }
}

/// `245 / 250 °C`, with `—` for a missing value.
pub fn format_temp(current: Option<f64>, target: Option<f64>) -> String {
    let t = |v: Option<f64>| v.map(|x| format!("{x:.0}")).unwrap_or_else(|| "—".into());
    format!("{} / {} °C", t(current), t(target))
}

/// On the H2 series extruder 0 is the right nozzle and 1 the left.
pub fn nozzle_label(id: u32, count: usize) -> &'static str {
    match (count >= 2, id) {
        (true, 0) => "Right nozzle",
        (true, 1) => "Left nozzle",
        _ => "Nozzle",
    }
}

/// A CSS colour for `RRGGBBAA`; transparent when the value is unusable.
pub fn swatch_color(tray_color: &str) -> String {
    let hex = tray_color.trim();
    if (hex.len() == 8 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", hex.to_ascii_lowercase())
    } else {
        "transparent".into()
    }
}
```

Create `src/printer/bridge.rs`:

```rust
//! Invoke wrappers for the `printer_*` commands. Tauri expects camelCase
//! argument names.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{DiscoveredPrinter, PrinterConfigView, PrinterView, TestOutcome};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let out = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{cmd} failed")))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct Empty {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestArgs<'a> {
    ip: &'a str,
    serial: &'a str,
    access_code: Option<&'a str>,
    pinned_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveArgs<'a> {
    ip: &'a str,
    serial: &'a str,
    name: &'a str,
    model: &'a str,
    access_code: Option<&'a str>,
    pinned_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssignArgs<'a> {
    ams_id: u32,
    tray_id: u32,
    preset_path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SlotArgs {
    ams_id: u32,
    tray_id: u32,
}

/// Blank strings go to the backend as `None`.
fn opt(s: &str) -> Option<&str> {
    Some(s.trim()).filter(|s| !s.is_empty())
}

pub async fn get_config() -> Result<Option<PrinterConfigView>, String> {
    call("printer_get_config", &Empty {}).await
}

pub async fn discover() -> Result<Vec<DiscoveredPrinter>, String> {
    call("printer_discover", &Empty {}).await
}

pub async fn test_connection(
    ip: &str,
    serial: &str,
    access_code: &str,
    pinned_fingerprint: &str,
) -> Result<TestOutcome, String> {
    call(
        "printer_test_connection",
        &TestArgs {
            ip,
            serial,
            access_code: opt(access_code),
            pinned_fingerprint: opt(pinned_fingerprint),
        },
    )
    .await
}

pub async fn save(
    ip: &str,
    serial: &str,
    name: &str,
    model: &str,
    access_code: &str,
    pinned_fingerprint: &str,
) -> Result<PrinterConfigView, String> {
    call(
        "printer_save",
        &SaveArgs {
            ip,
            serial,
            name,
            model,
            access_code: opt(access_code),
            pinned_fingerprint: opt(pinned_fingerprint),
        },
    )
    .await
}

pub async fn remove() -> Result<(), String> {
    call("printer_remove", &Empty {}).await
}

pub async fn view() -> Result<PrinterView, String> {
    call("printer_view", &Empty {}).await
}

pub async fn assign_slot(
    ams_id: u32,
    tray_id: u32,
    preset_path: &str,
) -> Result<PrinterView, String> {
    call(
        "printer_assign_slot",
        &AssignArgs {
            ams_id,
            tray_id,
            preset_path,
        },
    )
    .await
}

pub async fn clear_slot(ams_id: u32, tray_id: u32) -> Result<PrinterView, String> {
    call("printer_clear_slot", &SlotArgs { ams_id, tray_id }).await
}
```

Run: `cargo test --bin bambumate printer::types`
Expected: `test result: ok. 6 passed; 0 failed`. Dead-code warnings for the formatting helpers are expected until Task 10 uses them.

- [ ] **Step 3: Mount the shared state**

In `src/app.rs`:
- Add `use crate::printer::{PrinterEvents, PrinterShared};` after the `setup_wizard` import.
- Add `provide_context(PrinterShared::new());` after `provide_context(AgentShared::default());`.
- Mount the listeners right after `<AgentEvents />`:

```rust
            <AgentEvents />
            <PrinterEvents />
```

- [ ] **Step 4: Write the Settings section with its failing test**

Create `src/components/printer_settings.rs` with only its test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_messages() {
        let ok = TestOutcome {
            connection: ConnectionState::Connected,
            got_report: true,
            model: Some("H2D".into()),
        };
        assert_eq!(
            outcome_message(&ok, "10.0.0.2"),
            "Connected to H2D. Live status is on the Printer page."
        );
        let refused = TestOutcome {
            connection: ConnectionState::AuthFailed,
            ..Default::default()
        };
        assert_eq!(
            outcome_message(&refused, "10.0.0.2"),
            "The access code was rejected. Check it on the printer screen (Settings → LAN)."
        );
    }
}
```

In `src/components/mod.rs`, add `pub mod printer_settings;` after `pub mod history_panel;`.

Run: `cargo test --bin bambumate components::printer_settings`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `outcome_message` and the other items not written yet.

Insert above the `#[cfg(test)]` line of `src/components/printer_settings.rs`:

```rust
//! Settings → Printer: discovery, manual entry, the access code, Test
//! connection and Trust this printer. The access code is sent to the
//! backend once and never shown again.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::printer::bridge;
use crate::printer::types::{
    connection_message, ConnectionState, DiscoveredPrinter, PrinterConfigView, TestOutcome,
};
use crate::printer::PrinterShared;

/// The line shown after Test connection.
pub fn outcome_message(outcome: &TestOutcome, ip: &str) -> String {
    match (&outcome.connection, outcome.got_report) {
        (ConnectionState::Connected, true) => format!(
            "Connected to {}. Live status is on the Printer page.",
            outcome
                .model
                .clone()
                .unwrap_or_else(|| "the printer".into())
        ),
        (ConnectionState::Connected, false) => {
            "Connected, but the printer hasn't sent its status yet.".into()
        }
        (state, _) => connection_message(state, ip).unwrap_or_default(),
    }
}

#[component]
pub fn PrinterSettings() -> impl IntoView {
    let shared = use_context::<PrinterShared>();
    let ip = RwSignal::new(String::new());
    let serial = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let model = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let pin = RwSignal::new(String::new());
    let saved = RwSignal::new(Option::<PrinterConfigView>::None);
    let found = RwSignal::new(Option::<Vec<DiscoveredPrinter>>::None);
    let scanning = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(Option::<(String, bool)>::None);
    let untrusted = RwSignal::new(Option::<String>::None);

    let load = move || {
        spawn_local(async move {
            if let Ok(Some(c)) = bridge::get_config().await {
                ip.set(c.ip.clone());
                serial.set(c.serial.clone());
                name.set(c.name.clone());
                model.set(c.model.clone());
                pin.set(c.pinned_fingerprint.clone().unwrap_or_default());
                saved.set(Some(c));
            }
        });
    };
    load();

    let scan = move |_| {
        scanning.set(true);
        found.set(None);
        spawn_local(async move {
            let list = bridge::discover().await.unwrap_or_default();
            found.set(Some(list));
            scanning.set(false);
        });
    };

    let pick = move |p: DiscoveredPrinter| {
        ip.set(p.ip);
        serial.set(p.serial);
        name.set(p.name);
        model.set(p.model);
        message.set(None);
        untrusted.set(None);
    };

    // Tests with the form's values; the stored code is used when the field is blank.
    let run_test = move || {
        busy.set(true);
        message.set(None);
        untrusted.set(None);
        spawn_local(async move {
            let (i, s) = (ip.get_untracked(), serial.get_untracked());
            match bridge::test_connection(&i, &s, &code.get_untracked(), &pin.get_untracked()).await
            {
                Ok(outcome) => {
                    if let Some(m) = outcome.model.clone() {
                        model.set(m);
                    }
                    if let ConnectionState::CertUntrusted { fingerprint } = &outcome.connection {
                        untrusted.set(Some(fingerprint.clone()));
                    }
                    let ok = outcome.connection == ConnectionState::Connected;
                    message.set(Some((outcome_message(&outcome, &i), ok)));
                }
                Err(e) => message.set(Some((e, false))),
            }
            busy.set(false);
        });
    };

    // Saves the form (and a pin, when given), then refreshes the shared view.
    let run_save = move |then_test: bool| {
        busy.set(true);
        spawn_local(async move {
            let result = bridge::save(
                &ip.get_untracked(),
                &serial.get_untracked(),
                &name.get_untracked(),
                &model.get_untracked(),
                &code.get_untracked(),
                &pin.get_untracked(),
            )
            .await;
            busy.set(false);
            match result {
                Ok(c) => {
                    code.set(String::new());
                    saved.set(Some(c));
                    if let Some(s) = shared {
                        s.refresh();
                    }
                    if then_test {
                        run_test();
                    } else {
                        message.set(Some(("Saved.".into(), true)));
                    }
                }
                Err(e) => message.set(Some((e, false))),
            }
        });
    };

    let trust = move |_| {
        if let Some(fp) = untrusted.get_untracked() {
            pin.set(fp);
            run_save(true);
        }
    };

    let remove = move |_| {
        spawn_local(async move {
            match bridge::remove().await {
                Ok(()) => {
                    for s in [ip, serial, name, model, code, pin] {
                        s.set(String::new());
                    }
                    saved.set(None);
                    untrusted.set(None);
                    message.set(Some(("Printer removed.".into(), true)));
                    if let Some(s) = shared {
                        s.refresh();
                    }
                }
                Err(e) => message.set(Some((e, false))),
            }
        });
    };

    let code_placeholder = move || {
        if saved.get().is_some_and(|c| c.has_access_code) {
            "Saved in the system keychain"
        } else {
            "Shown on the printer screen (Settings → LAN)"
        }
    };

    view! {
        <section class="settings-section printer-settings" id="printer">
            <h3>"Printer"</h3>
            <p class="section-description">
                "Connect to your Bambu printer on the local network to see live status and what is loaded in each AMS slot. BambuMate only reads from the printer."
            </p>

            <div class="form-group">
                <button class="btn btn-secondary btn-sm printer-scan" on:click=scan disabled=move || scanning.get()>
                    {move || if scanning.get() { "Searching…" } else { "Find printers" }}
                </button>
                {move || found.get().map(|list| {
                    if list.is_empty() {
                        view! {
                            <p class="section-description printer-none">
                                "No printers found. Enter the IP address and serial number below."
                            </p>
                        }
                        .into_any()
                    } else {
                        view! {
                            <ul class="printer-found">
                                {list.into_iter().map(|p| {
                                    let label = format!("{} · {} · {} · {}", p.name, p.model, p.serial, p.ip);
                                    view! {
                                        <li>
                                            <button class="printer-found-item" on:click=move |_| pick(p.clone())>
                                                {label}
                                            </button>
                                        </li>
                                    }
                                }).collect::<Vec<_>>()}
                            </ul>
                        }
                        .into_any()
                    }
                })}
            </div>

            <div class="form-group">
                <label for="printer-ip">"IP address"</label>
                <input id="printer-ip" class="input" type="text" placeholder="192.168.1.20"
                    prop:value=move || ip.get()
                    on:input=move |ev| ip.set(event_target_value(&ev)) />
            </div>
            <div class="form-group">
                <label for="printer-serial">"Serial number"</label>
                <input id="printer-serial" class="input" type="text"
                    prop:value=move || serial.get()
                    on:input=move |ev| serial.set(event_target_value(&ev)) />
            </div>
            <div class="form-group">
                <label for="printer-code">"Access code"</label>
                <input id="printer-code" class="input" type="password" autocomplete="off"
                    placeholder=code_placeholder
                    prop:value=move || code.get()
                    on:input=move |ev| code.set(event_target_value(&ev)) />
            </div>

            <div class="input-row">
                <button class="btn btn-secondary printer-test" on:click=move |_| run_test() disabled=move || busy.get()>
                    "Test connection"
                </button>
                <button class="btn btn-save printer-save" on:click=move |_| run_save(false) disabled=move || busy.get()>
                    "Save"
                </button>
                <Show when=move || saved.get().is_some()>
                    <button class="btn btn-danger btn-sm printer-remove" on:click=remove>"Remove printer"</button>
                </Show>
            </div>

            <Show when=move || busy.get()>
                <span class="status-text">"Connecting…"</span>
            </Show>
            {move || message.get().map(|(text, ok)| view! {
                <p class={if ok { "status-text status-success printer-result" } else { "status-text status-warning printer-result" }}>
                    {text}
                </p>
            })}

            {move || untrusted.get().map(|fp| view! {
                <div class="printer-trust">
                    <p class="section-description">
                        "This printer's certificate isn't signed by a Bambu CA that BambuMate knows. If this fingerprint matches your printer, trust it. BambuMate will accept only this certificate from now on."
                    </p>
                    <code class="printer-fingerprint">{fp}</code>
                    <button class="btn btn-primary btn-sm printer-trust-btn" on:click=trust disabled=move || busy.get()>
                        "Trust this printer"
                    </button>
                </div>
            })}
        </section>
    }
}
```

In `src/pages/settings.rs`:
- Add `use crate::components::printer_settings::PrinterSettings;` after the `ApiKeyForm` import.
- Render the section just before the `Application` section:

```rust
            <PrinterSettings />

            <section class="settings-section">
                <h3>"Application"</h3>
```

Run: `cargo test --bin bambumate components::printer_settings`
Expected: `test result: ok. 1 passed; 0 failed`

- [ ] **Step 5: Add the stylesheet**

Create `style/printer.css`:

```css
/* Printer page, rail dot and Settings → Printer. Built on the .nd tokens
   (style/tokens.css). --nd-signal arrives with the design-system branch
   (PR #25); until then the dot falls back to --nd-success. */

/* -- Settings → Printer (uses the Settings page's own classes otherwise) ----- */
.printer-found { list-style: none; margin: 8px 0 0; padding: 0; display: grid; gap: 4px; }
.printer-found-item { width: 100%; text-align: left; padding: 8px 10px; border: 1px solid var(--border-primary); border-radius: 6px; background: transparent; color: inherit; font: inherit; cursor: pointer; }
.printer-trust { display: grid; gap: 8px; margin-top: 12px; }
.printer-fingerprint { font-family: ui-monospace, monospace; font-size: 12px; overflow-wrap: anywhere; }
```

In `index.html`, add the stylesheet after `style/agent.css`:

```html
    <link data-trunk rel="css" href="style/agent.css" />
    <link data-trunk rel="css" href="style/printer.css" />
```

- [ ] **Step 6: Add the WebKit fixtures and Settings flow**

In `tests/webkit/fixtures.mjs`, insert this block just above `export const FIXTURES = {`:

```js
// -- printer ------------------------------------------------------------------
//
// Shapes mirror src/printer/types.rs, which mirrors the backend's
// PrinterView (src-tauri/src/printer/service.rs). The slots follow the H2D
// report fixture in src-tauri/src/printer/testdata/h2d_full.json.

export const PRINTER_SERIAL = "0948AB000000001";
export const PRINTER_FINGERPRINT =
  "3A:7F:10:C2:9B:44:E1:08:5D:6A:0F:92:B7:31:CE:04:88:1B:F6:2D:73:A9:50:E3:1C:47:DA:6E:02:B5:98:FF";

const tray = (type, color, idx, sub = "", remain = null) => ({
  id: 0,
  empty: !type && !idx,
  tray_type: type,
  tray_color: color,
  tray_info_idx: idx,
  tray_sub_brands: sub,
  remain,
});

const slotOf = (ams_id, tray_id, label, t, extra = {}) => ({
  ams_id,
  tray_id,
  label,
  tray: { ...t, id: tray_id },
  rfid: false,
  assigned_preset: null,
  assigned_filament_id: null,
  status: "unassigned",
  needs_cloud_sync: false,
  ...extra,
});

export const PRINTER_SLOTS = [
  slotOf(0, 0, "A1", tray("PLA-S", "FFFFFFFF", "GFS02", "Support for PLA", 1), { rfid: true, status: "rfid" }),
  slotOf(0, 1, "A2", tray("PLA", "FFFFFFFF", "GFA00", "PLA Basic", 31), { rfid: true, status: "rfid" }),
  slotOf(0, 2, "A3", tray("PETG", "1F7A3DFF", "GFG99")),
  slotOf(0, 3, "A4", tray("PLA", "FFFFFFFF", "GFA00", "PLA Basic", 100), { rfid: true, status: "rfid" }),
  slotOf(1, 0, "B1", tray("PLA", "000000FF", "GFA00", "PLA Basic", 55), { rfid: true, status: "rfid" }),
  slotOf(1, 1, "B2", tray("PLA", "F95959FF", "P4d6ae04"), {
    assigned_preset: "Acme Matte PLA",
    assigned_filament_id: "P4d6ae04",
    status: "matches",
  }),
  slotOf(1, 2, "B3", tray("PETG-CF", "000000FF", "GFG50", "PETG-CF", 100), { rfid: true, status: "rfid" }),
  slotOf(1, 3, "B4", tray("", "00000000", ""), { status: "empty" }),
  slotOf(255, 254, "Ext-L", tray("PLA", "76D9F4FF", "GFA01")),
  slotOf(255, 255, "Ext-R", tray("", "00000000", ""), { status: "empty" }),
];

export const PRINTER_VIEW = {
  configured: true,
  printer: {
    ip: "192.168.1.20",
    serial: PRINTER_SERIAL,
    name: "Workshop H2D",
    model: "H2D",
    firmware: "01.01.01.00",
  },
  connection: { state: "connected" },
  state: {
    gcode_state: "RUNNING",
    subtask_name: "T-pose - slim H2D dual AMS riser",
    mc_percent: 6,
    mc_remaining_time: 549,
    layer_num: 1,
    total_layer_num: 200,
    bed_temp: 70.0,
    bed_target_temp: 70.0,
    nozzles: [
      { id: 0, temp: 245.0, target_temp: 245.0, diameter: 0.4, nozzle_type: "HS01" },
      { id: 1, temp: 47.0, target_temp: 0.0, diameter: 0.4, nozzle_type: "HS01" },
    ],
    active_nozzle: 0,
    ams_units: [
      { id: 0, humidity_level: 5, humidity_pct: 21, temp: 27.0 },
      { id: 1, humidity_level: 5, humidity_pct: 18, temp: 29.7 },
    ],
  },
  slots: PRINTER_SLOTS,
  errors: [
    {
      kind: "hms",
      code: "0300_0100_0001_0007",
      text: "The heatbed temperature is abnormal; the sensor may have an open circuit.",
      wiki_url: "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0100_0001_0007",
    },
    {
      kind: "print_error",
      code: "0300_400C",
      text: null,
      wiki_url: "https://wiki.bambulab.com/en/hms/home",
    },
  ],
};

export const PRINTER_UNCONFIGURED = {
  configured: false,
  printer: null,
  connection: { state: "disconnected" },
  state: null,
  slots: [],
  errors: [],
};

/** PRINTER_VIEW with one slot changed. */
export const withSlot = (label, patch) => ({
  ...PRINTER_VIEW,
  slots: PRINTER_SLOTS.map((s) => (s.label === label ? { ...s, ...patch } : s)),
});

/** A3 after the user assigns PolyLite: the printer still reports PETG. */
export const A3_ASSIGNED = {
  assigned_preset: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
  assigned_filament_id: "PA-PL-WHTPA0-01",
  status: "different",
  needs_cloud_sync: true,
};
```

Then add these entries at the end of the `FIXTURES` object, after `agent_login: null,`:

```js

  // -- printer --
  // Not set up in Settings yet, so the discovery flow has an empty form.
  printer_get_config: null,
  printer_view: PRINTER_VIEW,
  printer_discover: [
    { ip: "192.168.1.20", serial: PRINTER_SERIAL, name: "Workshop H2D", model: "H2D" },
  ],
  printer_test_connection: {
    connection: { state: "cert_untrusted", fingerprint: PRINTER_FINGERPRINT },
    got_report: false,
    model: null,
  },
  printer_save: {
    ip: "192.168.1.20",
    serial: PRINTER_SERIAL,
    name: "Workshop H2D",
    model: "H2D",
    pinned_fingerprint: PRINTER_FINGERPRINT,
    has_access_code: true,
  },
  printer_remove: null,
  printer_assign_slot: withSlot("A3", A3_ASSIGNED),
  printer_clear_slot: PRINTER_VIEW,
```

In `tests/webkit/app-flows.mjs`:
- Change the fixtures import to:

```js
import {
  FIXTURES,
  GIF_1X1,
  makePng,
  PRINTER_FINGERPRINT,
  PRINTER_SERIAL,
} from "./fixtures.mjs";
```

- In `installTauriMock`, directly after `window.__ipc = { calls, unknown };`, add:

```js
  // Steps swap a command's canned answer mid-run through this.
  window.__fixtures = fixtures;
```

- After the `"an API key form renders per provider"` step, and before the ``await page.screenshot({ path: `flow-${engine}-settings.png`, … })`` line, add:

```js
  // -- Settings → Printer -------------------------------------------------------
  const ipcCalls = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c), cmd);
  const setFixture = (cmd, value) =>
    page.evaluate(([c, v]) => {
      window.__fixtures[c] = v;
    }, [cmd, value]);

  await step(run, page, "printer settings find a printer on the network", async () => {
    await page.click(".printer-scan");
    await page.waitForSelector(".printer-found-item", { timeout: 10000 });
    await page.click(".printer-found-item");
    const ip = await page.inputValue("#printer-ip");
    const serial = await page.inputValue("#printer-serial");
    if (ip !== "192.168.1.20" || serial !== PRINTER_SERIAL) throw new Error(`form has ${ip} / ${serial}`);
    return `${ip} ${serial}`;
  });

  await step(run, page, "an untrusted certificate offers Trust this printer", async () => {
    await page.fill("#printer-code", "12345678");
    await page.click(".printer-test");
    await page.waitForSelector(".printer-trust", { timeout: 10000 });
    const fp = (await page.locator(".printer-fingerprint").innerText()).trim();
    if (fp !== PRINTER_FINGERPRINT) throw new Error(`fingerprint shows ${fp}`);
    const [t] = await ipcCalls("printer_test_connection");
    if (t.args.accessCode !== "12345678" || t.args.serial !== PRINTER_SERIAL) {
      throw new Error(JSON.stringify(t.args));
    }
    const label = (await page.locator(".printer-trust-btn").innerText()).trim();
    if (label !== "Trust this printer") throw new Error(`button reads "${label}"`);
  });

  await step(run, page, "trusting pins the fingerprint, saves and connects", async () => {
    await setFixture("printer_test_connection", {
      connection: { state: "connected" },
      got_report: true,
      model: "H2D",
    });
    await page.click(".printer-trust-btn");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "printer_save"), null, {
      timeout: 5000,
    });
    const [save] = await ipcCalls("printer_save");
    if (save.args.pinnedFingerprint !== PRINTER_FINGERPRINT || save.args.accessCode !== "12345678") {
      throw new Error(JSON.stringify(save.args));
    }
    await page.waitForFunction(
      () => document.querySelector(".printer-result")?.innerText.includes("Connected to H2D"),
      null,
      { timeout: 5000 }
    );
    if ((await page.inputValue("#printer-code")) !== "") throw new Error("the access code is still in the field");
  });
```

- [ ] **Step 7: Verify**

Run: `cargo fmt --check`
Expected: no output.

Run: `cargo check --target wasm32-unknown-unknown`
Expected: `Finished`, with no errors. Dead-code warnings in `src/printer/` are expected until Task 10.

Run: `cargo test --bin bambumate`
Expected: `test result: ok. 17 passed; 0 failed`: 10 existing and 7 new.

Run: `trunk build`
Expected: ends with `✅ success`.

Run: `cd tests/webkit && node app-flows.mjs ../..`
Expected: every step `OK` in both engines, including `printer settings find a printer on the network`, `an untrusted certificate offers Trust this printer` and `trusting pins the fingerprint, saves and connects`, then `PASS: every flow completed in both engines.`

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/app.rs src/printer/mod.rs src/printer/types.rs src/printer/bridge.rs src/components/mod.rs src/components/printer_settings.rs src/pages/settings.rs style/printer.css index.html tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "Add Settings → Printer with discovery, test connection and trust this printer"
```

---

### Task 10: Printer page, rail dot, AMS cards and slot picker

**Files:**
- Create: `src/pages/printer.rs`
- Modify: `src/pages/mod.rs`, `src/app.rs` (route), `src/components/sidebar.rs` (Printer entry and dot)
- Modify: `style/printer.css` (replace with the full file)
- Modify: `tests/webkit/app-flows.mjs` (imports, `a3Matched` helper, printer page steps)

**Interfaces:**
- Consumes:
  - Task 9: `PrinterShared`, `printer::bridge::{assign_slot, clear_slot}` and the `printer::types::*` helpers
  - Existing: `commands::{list_profiles, list_system_profiles, ProfileInfo}`
  - Fixtures: `PRINTER_VIEW`, `PRINTER_UNCONFIGURED`, `A3_ASSIGNED`, `withSlot`, and `setFixture`/`ipcCalls` from Task 9
- Produces:
  - `crate::pages::printer::PrinterPage`, the `/printer` route
  - Classes the flows use:
    - page: `.printer-page.nd`, `.pr-empty`, `.pr-setup-link`, `.pr-hero`, `.pr-hero-percent`, `.pr-body.pr-stale`, `.pr-notice`, `.pr-ams-row`, `.pr-ams-meta`, `.pr-error`, `.pr-error-text`, `.pr-error-link`
    - cards: `.pr-slot[data-label][data-status]`, `.pr-slot-main`, `.pr-status`, `.pr-rfid`, `.pr-steps`
    - picker: `.pr-picker`, `.pr-picker-search`, `.pr-picker-item`
    - rail: `.printer-dot[data-state]`

- [ ] **Step 1: Write the failing page tests**

In `src/pages/mod.rs`, add `pub mod printer;` after `pub mod print_analysis;`.

Create `src/pages/printer.rs` with only its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{AmsUnit, PrinterState};

    fn slot(ams_id: u32, tray_id: u32) -> SlotView {
        SlotView {
            ams_id,
            tray_id,
            ..Default::default()
        }
    }

    #[test]
    fn slots_group_into_rows_in_order() {
        let v = PrinterView {
            slots: vec![slot(0, 0), slot(0, 1), slot(1, 0), slot(255, 254)],
            ..Default::default()
        };
        let rows = ams_rows(&v);
        let shape: Vec<(u32, usize)> = rows.iter().map(|(id, s)| (*id, s.len())).collect();
        assert_eq!(shape, vec![(0, 2), (1, 1), (255, 1)]);
    }

    #[test]
    fn unit_meta_prefers_percent_humidity() {
        let v = PrinterView {
            state: Some(PrinterState {
                ams_units: vec![AmsUnit {
                    id: 0,
                    humidity_level: Some(5),
                    humidity_pct: Some(21),
                    temp: Some(27.0),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(unit_meta(&v, 0), "Humidity 21% · 27.0 °C");
        assert_eq!(unit_meta(&v, 9), "");
    }

    #[test]
    fn preset_search_is_case_insensitive() {
        let p = |n: &str| ProfileInfo {
            name: n.into(),
            filament_type: None,
            filament_id: None,
            path: format!("/{n}.json"),
            is_user_profile: true,
        };
        let list = vec![p("Polymaker PolyLite PLA"), p("Bambu PETG HF")];
        assert_eq!(matching(&list, "polylite").len(), 1);
        assert_eq!(matching(&list, "").len(), 2);
    }
}
```

Run: `cargo test --bin bambumate pages::printer`
Expected: FAIL to compile with unresolved-import (E0432) or cannot-find (E0425/E0412) errors naming `ams_rows` and the other items not written yet.

- [ ] **Step 2: Write the page**

Insert above the `#[cfg(test)]` line of `src/pages/printer.rs`:

```rust
//! The Printer page: current print, AMS slots with their status and the
//! steps to set a slot on the printer, and active errors. Read-only.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, ProfileInfo};
use crate::printer::bridge;
use crate::printer::types::{
    connection_message, format_remaining, format_temp, nozzle_label, row_title, swatch_color,
    ConnectionState, PrinterView, SlotStatus, SlotView,
};
use crate::printer::PrinterShared;

/// Picker rows shown at once; the search narrows the rest.
const PICKER_LIMIT: usize = 60;

#[component]
pub fn PrinterPage() -> impl IntoView {
    let shared = expect_context::<PrinterShared>();
    let view = shared.view;
    // Picks up assignments' cloud-sync state and anything missed while away.
    shared.refresh();
    let picking = RwSignal::new(Option::<SlotView>::None);

    let connected = move || view.with(|v| v.connection == ConnectionState::Connected);

    view! {
        <div class="page printer-page nd">
            <header class="pr-head">
                <h2>"Printer"</h2>
                {move || view.with(|v| v.printer.clone()).map(|p| view! {
                    <span class="pr-ident nd-mono">
                        {format!("{} · {} · {}", if p.name.is_empty() { p.serial.clone() } else { p.name.clone() }, p.model, p.serial)}
                    </span>
                })}
            </header>

            <Show
                when=move || view.with(|v| v.configured)
                fallback=|| view! {
                    <div class="pr-empty">
                        <p>"No printer is set up yet."</p>
                        <a href="/settings#printer" class="pr-setup-link">"Set one up in Settings → Printer"</a>
                    </div>
                }
            >
                {move || {
                    let v = view.get();
                    let ip = v.printer.as_ref().map(|p| p.ip.clone()).unwrap_or_default();
                    connection_message(&v.connection, &ip).map(|m| view! {
                        <p class="pr-notice" data-state=v.connection.dot()>{m}</p>
                    })
                }}
                <div class="pr-body" class:pr-stale=move || !connected()>
                    <Hero view=view />
                    <section class="pr-section pr-ams">
                        <p class="nd-label">"AMS"</p>
                        {move || ams_rows(&view.get()).into_iter().map(|(ams_id, slots)| {
                            let meta = view.with(|v| unit_meta(v, ams_id));
                            view! {
                                <div class="pr-ams-row" data-ams=ams_id>
                                    <div class="pr-ams-head">
                                        <span class="pr-ams-title">{row_title(ams_id)}</span>
                                        <span class="pr-ams-meta nd-mono">{meta}</span>
                                    </div>
                                    <div class="pr-slots">
                                        {slots.into_iter().map(|s| view! { <SlotCard item=s picking=picking /> }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </section>
                    <Errors view=view />
                </div>
            </Show>

            {move || picking.get().map(|slot| view! { <SlotPicker item=slot picking=picking /> })}
        </div>
    }
}

/// Slots grouped by unit, AMS units first, external spools last.
fn ams_rows(v: &PrinterView) -> Vec<(u32, Vec<SlotView>)> {
    let mut rows: Vec<(u32, Vec<SlotView>)> = Vec::new();
    for s in &v.slots {
        match rows.iter_mut().find(|(id, _)| *id == s.ams_id) {
            Some((_, list)) => list.push(s.clone()),
            None => rows.push((s.ams_id, vec![s.clone()])),
        }
    }
    rows
}

/// `Humidity 21% · 27.0 °C` for an AMS unit.
fn unit_meta(v: &PrinterView, ams_id: u32) -> String {
    let Some(unit) = v
        .state
        .as_ref()
        .and_then(|s| s.ams_units.iter().find(|u| u.id == ams_id))
    else {
        return String::new();
    };
    let humidity = match (unit.humidity_pct, unit.humidity_level) {
        (Some(p), _) => format!("Humidity {p}%"),
        (None, Some(l)) => format!("Humidity level {l}"),
        _ => String::new(),
    };
    let temp = unit.temp.map(|t| format!("{t:.1} °C")).unwrap_or_default();
    [humidity, temp]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[component]
fn Hero(view: RwSignal<PrinterView>) -> impl IntoView {
    let state = move || view.with(|v| v.state.clone().unwrap_or_default());
    view! {
        <section class="pr-section pr-hero">
            <p class="nd-label">"Current print"</p>
            <div class="pr-hero-top">
                <span class="pr-hero-state nd-mono">{move || state().gcode_state.unwrap_or_else(|| "—".into())}</span>
                <span class="pr-hero-percent">{move || state().mc_percent.map(|p| format!("{p}%")).unwrap_or_else(|| "—".into())}</span>
            </div>
            <div class="pr-progress">
                <div class="pr-progress-fill" style:width=move || format!("{}%", state().mc_percent.unwrap_or(0).min(100))></div>
            </div>
            <dl class="pr-facts">
                <div><dt>"Layer"</dt><dd class="pr-layer">{move || {
                    let s = state();
                    match (s.layer_num, s.total_layer_num) {
                        (Some(l), Some(t)) if t > 0 => format!("{l} / {t}"),
                        _ => "—".into(),
                    }
                }}</dd></div>
                <div><dt>"Remaining"</dt><dd class="pr-remaining">{move || state().mc_remaining_time.map(format_remaining).unwrap_or_else(|| "—".into())}</dd></div>
                <div class="pr-file"><dt>"File"</dt><dd>{move || state().subtask_name.filter(|f| !f.is_empty()).unwrap_or_else(|| "—".into())}</dd></div>
            </dl>
            <div class="pr-temps">
                {move || {
                    let s = state();
                    let count = s.nozzles.len();
                    s.nozzles.iter().map(|n| {
                        let active = count >= 2 && s.active_nozzle == Some(n.id);
                        view! {
                            <div class="pr-temp" class:pr-temp-active=active>
                                <span class="nd-label">{nozzle_label(n.id, count)}</span>
                                <span class="nd-mono">{format_temp(n.temp, n.target_temp)}</span>
                                <span class="pr-temp-note">{n.diameter.map(|d| format!("{d} mm")).unwrap_or_default()}</span>
                            </div>
                        }
                    }).collect::<Vec<_>>()
                }}
                <div class="pr-temp">
                    <span class="nd-label">"Bed"</span>
                    <span class="nd-mono">{move || { let s = state(); format_temp(s.bed_temp, s.bed_target_temp) }}</span>
                </div>
            </div>
        </section>
    }
}

#[component]
fn SlotCard(item: SlotView, picking: RwSignal<Option<SlotView>>) -> impl IntoView {
    let slot = item;
    let status = slot.status;
    let color = swatch_color(&slot.tray.tray_color);
    let material = if slot.tray.empty {
        "—".to_string()
    } else {
        slot.tray.tray_type.clone()
    };
    let preset = slot.preset_name();
    let remain = slot.rfid.then_some(slot.tray.remain).flatten();
    let steps = (status == SlotStatus::Different)
        .then(|| slot.assigned_preset.clone())
        .flatten()
        .map(|p| (slot.label.clone(), p, slot.needs_cloud_sync));
    let open = slot.clone();
    view! {
        <div class="pr-slot" data-status=status.key() data-label=slot.label.clone()>
            <button class="pr-slot-main" on:click=move |_| picking.set(Some(open.clone()))
                title="Choose the preset loaded here">
                <span class="pr-swatch" style:background=color></span>
                <span class="pr-slot-label nd-mono">{slot.label.clone()}</span>
                <span class="pr-material">{material}</span>
                <span class="pr-preset">{preset}</span>
                <span class="pr-slot-foot">
                    {remain.map(|r| view! { <span class="pr-remain nd-mono">{format!("{r}%")}</span> })}
                    {slot.rfid.then(|| view! { <span class="pr-rfid nd-label">"RFID"</span> })}
                    <span class="pr-status">{status.badge()}</span>
                </span>
            </button>
            {steps.map(|(label, preset, needs_sync)| view! {
                <ol class="pr-steps">
                    <li>"On the printer: Filament → "{label.clone()}" → choose "<em>{preset.clone()}</em>"."</li>
                    <li>"Or in Bambu Studio: Device → AMS → "{label}" → "<em>{preset}</em>"."</li>
                    {needs_sync.then(|| view! {
                        <li class="pr-sync-note">"It must sync to Bambu Cloud first — open Bambu Studio while signed in."</li>
                    })}
                </ol>
            })}
        </div>
    }
}

#[component]
fn Errors(view: RwSignal<PrinterView>) -> impl IntoView {
    view! {
        <section class="pr-section pr-errors">
            <p class="nd-label">"Errors"</p>
            <Show
                when=move || view.with(|v| !v.errors.is_empty())
                fallback=|| view! { <p class="pr-quiet">"No active errors."</p> }
            >
                <ul class="pr-error-list">
                    {move || view.get().errors.into_iter().map(|e| view! {
                        <li class="pr-error">
                            <span class="pr-error-code nd-mono">{e.code.clone()}</span>
                            {match e.text.clone() {
                                Some(t) => view! { <span class="pr-error-text">{t}</span> }.into_any(),
                                None => view! {
                                    <a class="pr-error-link" href=e.wiki_url.clone() target="_blank" rel="noopener">
                                        "Look up this code on the Bambu wiki"
                                    </a>
                                }.into_any(),
                            }}
                        </li>
                    }).collect::<Vec<_>>()}
                </ul>
            </Show>
        </section>
    }
}

/// Filters presets by a case-insensitive search.
fn matching(list: &[ProfileInfo], query: &str) -> Vec<ProfileInfo> {
    let q = query.trim().to_lowercase();
    list.iter()
        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
        .take(PICKER_LIMIT)
        .cloned()
        .collect()
}

#[component]
fn SlotPicker(item: SlotView, picking: RwSignal<Option<SlotView>>) -> impl IntoView {
    let slot = item;
    let shared = expect_context::<PrinterShared>();
    let user = RwSignal::new(Vec::<ProfileInfo>::new());
    let system = RwSignal::new(Vec::<ProfileInfo>::new());
    let query = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    spawn_local(async move {
        if let Ok(list) = commands::list_profiles().await {
            user.set(list);
        }
        if let Ok(list) = commands::list_system_profiles().await {
            system.set(list);
        }
    });
    let (ams_id, tray_id) = (slot.ams_id, slot.tray_id);
    let choose = move |path: String| {
        spawn_local(async move {
            match bridge::assign_slot(ams_id, tray_id, &path).await {
                Ok(v) => {
                    shared.view.set(v);
                    picking.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };
    let clear = move |_| {
        spawn_local(async move {
            match bridge::clear_slot(ams_id, tray_id).await {
                Ok(v) => {
                    shared.view.set(v);
                    picking.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };
    let group = move |title: &'static str, list: Vec<ProfileInfo>| {
        (!list.is_empty()).then(|| view! {
            <p class="nd-label pr-picker-group">{title}</p>
            <ul class="pr-picker-list">
                {list.into_iter().map(|p| {
                    let path = p.path.clone();
                    view! {
                        <li><button class="pr-picker-item" on:click=move |_| choose(path.clone())>{p.name}</button></li>
                    }
                }).collect::<Vec<_>>()}
            </ul>
        })
    };
    let has_assignment = slot.assigned_preset.is_some();
    view! {
        <div class="pr-picker-backdrop" on:click=move |_| picking.set(None)>
            <div class="pr-picker" role="dialog" aria-label="Choose a preset" on:click=|e| e.stop_propagation()>
                <p class="nd-label">{format!("Slot {}", slot.label)}</p>
                <h3>"Which preset is loaded here?"</h3>
                <input class="pr-picker-search" type="search" placeholder="Search presets"
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev)) />
                <div class="pr-picker-results">
                    {move || group("Your presets", matching(&user.get(), &query.get()))}
                    {move || group("Bambu presets", matching(&system.get(), &query.get()))}
                </div>
                {move || error.get().map(|e| view! { <p class="pr-picker-error">{e}</p> })}
                <div class="pr-picker-actions">
                    {has_assignment.then(|| view! {
                        <button class="pr-picker-clear" on:click=clear>"Clear assignment"</button>
                    })}
                    <button class="pr-picker-cancel" on:click=move |_| picking.set(None)>"Cancel"</button>
                </div>
            </div>
        </div>
    }
}
```

Leptos reserves `slot` as a view attribute, so the card and picker props are named `item`.

Run: `cargo test --bin bambumate pages::printer`
Expected: `test result: ok. 3 passed; 0 failed`

- [ ] **Step 3: Add the route and the rail entry**

In `src/app.rs`:
- Add `use crate::pages::printer::PrinterPage;` after the `print_analysis` import.
- Add the route after `/about`:

```rust
                            <Route path=path!("/about") view=AboutPage />
                            <Route path=path!("/printer") view=PrinterPage />
```

In `src/components/sidebar.rs`:
- Add `use crate::printer::PrinterShared;` after the `StlIndicator` import.
- After `let update_ctx = …;`, add:

```rust
    let printer = use_context::<PrinterShared>().expect("PrinterShared not provided");
    // No dot until a printer is set up.
    let printer_dot = move || {
        printer
            .view
            .with(|v| v.configured.then(|| v.connection.dot()))
    };
```

- After the `Profiles` list item, add:

```rust
                <li class="nav-item">
                    <a href="/printer" class="nav-link nav-link-printer">
                        "Printer"
                        {move || printer_dot().map(|state| view! {
                            <span class="nd printer-dot" data-state=state title="Printer connection"></span>
                        })}
                    </a>
                </li>
```

- [ ] **Step 4: Replace the stylesheet**

Replace `style/printer.css` with:

```css
/* Printer page, rail dot and Settings → Printer. Built on the .nd tokens
   (style/tokens.css). --nd-signal arrives with the design-system branch
   (PR #25); until then the dot falls back to --nd-success. */

/* -- rail dot ------------------------------------------------------------ */
.nav-link-printer { display: inline-flex; align-items: center; gap: 8px; }
.printer-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--nd-text-disabled);
}
.printer-dot[data-state="connected"] { background: var(--nd-signal, var(--nd-success)); }
.printer-dot[data-state="connecting"] { background: var(--nd-warning); }
.printer-dot[data-state="error"] { background: var(--nd-accent); }

/* -- page ------------------------------------------------------------------ */
.printer-page { display: grid; gap: var(--nd-space-lg); max-width: 1100px; }
.pr-head { display: flex; align-items: baseline; justify-content: space-between; gap: var(--nd-space-md); flex-wrap: wrap; }
.pr-head h2 { margin: 0; color: var(--nd-text-display); }
.pr-ident { font-size: 12px; color: var(--nd-text-secondary); }
.pr-empty { padding: var(--nd-space-xl); border: 1px dashed var(--nd-border-visible); border-radius: 12px; text-align: center; }
.pr-setup-link { color: var(--nd-interactive); }
.pr-notice { margin: 0; padding: var(--nd-space-sm) var(--nd-space-md); border-left: 3px solid var(--nd-warning); background: var(--nd-surface); }
.pr-notice[data-state="error"] { border-left-color: var(--nd-accent); }
.pr-body { display: grid; gap: var(--nd-space-lg); transition: opacity 200ms var(--nd-ease); }
.pr-body.pr-stale { opacity: 0.45; filter: grayscale(1); }
.pr-section { padding: var(--nd-space-lg); border: 1px solid var(--nd-border); border-radius: 12px; background: var(--nd-surface); display: grid; gap: var(--nd-space-md); }
.pr-section > .nd-label { margin: 0; }

.pr-hero-top { display: flex; align-items: baseline; justify-content: space-between; gap: var(--nd-space-md); }
.pr-hero-state { font-size: 14px; letter-spacing: 0.08em; color: var(--nd-text-secondary); }
.pr-hero-percent { font-size: 48px; line-height: 1; font-weight: 300; color: var(--nd-text-display); }
.pr-progress { height: 4px; background: var(--nd-border); border-radius: 2px; overflow: hidden; }
.pr-progress-fill { height: 100%; background: var(--nd-text-display); }
.pr-facts { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: var(--nd-space-md); margin: 0; }
.pr-facts dt { font-family: var(--nd-font-mono); font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--nd-text-secondary); }
.pr-facts dd { margin: 2px 0 0; color: var(--nd-text-primary); overflow-wrap: anywhere; }
.pr-temps { display: flex; flex-wrap: wrap; gap: var(--nd-space-md); }
.pr-temp { display: grid; gap: 2px; padding: var(--nd-space-sm) var(--nd-space-md); border: 1px solid var(--nd-border); border-radius: 8px; min-width: 140px; }
.pr-temp-active { border-color: var(--nd-text-primary); }
.pr-temp-note { font-size: 12px; color: var(--nd-text-secondary); }

.pr-ams-row { display: grid; gap: var(--nd-space-sm); }
.pr-ams-head { display: flex; justify-content: space-between; gap: var(--nd-space-md); }
.pr-ams-title { font-weight: 600; color: var(--nd-text-display); }
.pr-ams-meta { font-size: 12px; color: var(--nd-text-secondary); }
.pr-slots { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: var(--nd-space-sm); }
.pr-slot { border: 1px solid var(--nd-border-visible); border-radius: 10px; background: var(--nd-surface-raised); overflow: hidden; }
.pr-slot[data-status="empty"] { opacity: 0.55; }
.pr-slot[data-status="different"] { border-color: var(--nd-warning); }
.pr-slot-main {
    display: grid;
    grid-template-columns: 28px 1fr;
    grid-template-areas: "swatch label" "swatch material" "preset preset" "foot foot";
    gap: 2px var(--nd-space-sm);
    width: 100%;
    padding: var(--nd-space-sm) var(--nd-space-md);
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
}
.pr-slot-main:hover { background: var(--nd-surface); }
.pr-swatch { grid-area: swatch; width: 24px; height: 24px; border-radius: 50%; border: 1px solid var(--nd-border-visible); align-self: center; }
.pr-slot-label { grid-area: label; font-size: 12px; color: var(--nd-text-secondary); }
.pr-material { grid-area: material; font-weight: 600; color: var(--nd-text-display); }
.pr-preset { grid-area: preset; font-size: 13px; color: var(--nd-text-primary); overflow-wrap: anywhere; }
.pr-slot-foot { grid-area: foot; display: flex; align-items: center; gap: var(--nd-space-sm); flex-wrap: wrap; }
.pr-remain { font-size: 12px; }
.pr-rfid { padding: 1px 6px; border: 1px solid var(--nd-border-visible); border-radius: 999px; }
.pr-status { margin-left: auto; font-size: 12px; color: var(--nd-text-secondary); }
.pr-slot[data-status="matches"] .pr-status,
.pr-slot[data-status="rfid"] .pr-status { color: var(--nd-signal, var(--nd-success)); }
.pr-slot[data-status="different"] .pr-status { color: var(--nd-text-display); font-weight: 600; }
.pr-steps { margin: 0; padding: var(--nd-space-sm) var(--nd-space-md) var(--nd-space-sm) var(--nd-space-xl); border-top: 1px solid var(--nd-border); font-size: 12px; display: grid; gap: 4px; }
.pr-sync-note { color: var(--nd-accent); }

.pr-error-list { margin: 0; padding: 0; list-style: none; display: grid; gap: var(--nd-space-sm); }
.pr-error { display: flex; gap: var(--nd-space-md); align-items: baseline; flex-wrap: wrap; }
.pr-error-code { font-size: 12px; color: var(--nd-accent); }
.pr-error-link { color: var(--nd-interactive); }
.pr-quiet { margin: 0; color: var(--nd-text-secondary); }

/* -- slot picker ------------------------------------------------------------ */
.pr-picker-backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.45); display: flex; align-items: center; justify-content: center; z-index: 50; }
.pr-picker { width: min(520px, calc(100vw - 32px)); max-height: calc(100vh - 64px); display: flex; flex-direction: column; gap: var(--nd-space-sm); padding: var(--nd-space-lg); border-radius: 12px; background: var(--nd-surface); border: 1px solid var(--nd-border-visible); }
.pr-picker h3 { margin: 0; color: var(--nd-text-display); }
.pr-picker-search { padding: 8px 10px; border: 1px solid var(--nd-border-visible); border-radius: 8px; background: var(--nd-surface-raised); color: var(--nd-text-primary); font: inherit; }
.pr-picker-results { overflow-y: auto; min-height: 120px; }
.pr-picker-group { margin: var(--nd-space-sm) 0 4px; }
.pr-picker-list { margin: 0; padding: 0; list-style: none; }
.pr-picker-item { width: 100%; padding: 6px 8px; border: 0; border-radius: 6px; background: transparent; color: var(--nd-text-primary); font: inherit; text-align: left; cursor: pointer; }
.pr-picker-item:hover { background: var(--nd-surface-raised); }
.pr-picker-error { margin: 0; color: var(--nd-accent); }
.pr-picker-actions { display: flex; justify-content: flex-end; gap: var(--nd-space-sm); }
.pr-picker-clear, .pr-picker-cancel { padding: 6px 14px; border: 1px solid var(--nd-border-visible); border-radius: 999px; background: transparent; color: var(--nd-text-primary); font: inherit; cursor: pointer; }

/* -- Settings → Printer (uses the Settings page's own classes otherwise) ----- */
.printer-found { list-style: none; margin: 8px 0 0; padding: 0; display: grid; gap: 4px; }
.printer-found-item { width: 100%; text-align: left; padding: 8px 10px; border: 1px solid var(--border-primary); border-radius: 6px; background: transparent; color: inherit; font: inherit; cursor: pointer; }
.printer-trust { display: grid; gap: 8px; margin-top: 12px; }
.printer-fingerprint { font-family: ui-monospace, monospace; font-size: 12px; overflow-wrap: anywhere; }
```

- [ ] **Step 5: Add the page flows**

In `tests/webkit/app-flows.mjs`, change the fixtures import to:

```js
import {
  A3_ASSIGNED,
  FIXTURES,
  GIF_1X1,
  makePng,
  PRINTER_FINGERPRINT,
  PRINTER_SERIAL,
  PRINTER_UNCONFIGURED,
  PRINTER_VIEW,
  withSlot,
} from "./fixtures.mjs";
```

Add this helper just above `class Run {`:

```js

/** The view once the printer reports the preset assigned to A3. */
function a3Matched() {
  const a3 = PRINTER_VIEW.slots.find((s) => s.label === "A3");
  return withSlot("A3", {
    ...A3_ASSIGNED,
    status: "matches",
    tray: { ...a3.tray, tray_type: "PLA", tray_info_idx: "PA-PL-WHTPA0-01" },
  });
}
```

After the `"update state reports up to date"` step, and before `// -- agent drawer ---…`, add:

```js
  // -- printer page -------------------------------------------------------------
  const emitEvent = (name, payload) => page.evaluate(([n, p]) => window.__emit(n, p), [name, payload]);
  const slotCard = (label) => page.locator(`.pr-slot[data-label="${label}"]`);

  await step(run, page, "printer page points to Settings when no printer is set up", async () => {
    await setFixture("printer_view", PRINTER_UNCONFIGURED);
    await page.click('a[href="/printer"]');
    await page.waitForSelector(".printer-page .pr-empty", { timeout: 15000 });
    const link = await page.locator(".pr-setup-link").innerText();
    if (!link.includes("Settings → Printer")) throw new Error(`link reads "${link}"`);
    if ((await page.locator(".printer-dot").count()) !== 0) throw new Error("rail dot shown with no printer");
  });

  await step(run, page, "printer hero shows the current print", async () => {
    await setFixture("printer_view", PRINTER_VIEW);
    await page.click('a[href="/"]');
    await page.click('a[href="/printer"]');
    await page.waitForSelector(".printer-page.nd .pr-hero", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelector(".pr-hero-percent")?.innerText === "6%", null, {
      timeout: 5000,
    });
    // Lower-cased: .nd-label uppercases its text, and innerText reports what is rendered.
    const hero = (await page.locator(".pr-hero").innerText()).replace(/\s+/g, " ").toLowerCase();
    for (const want of ["running", "1 / 200", "9 h 09 m", "t-pose - slim h2d dual ams riser", "right nozzle", "left nozzle", "245 / 245 °c", "70 / 70 °c"]) {
      if (!hero.includes(want)) throw new Error(`hero lacks "${want}": ${hero}`);
    }
  });

  await step(run, page, "rail dot shows the connection", async () => {
    const dot = page.locator('.printer-dot[data-state="connected"]');
    if ((await dot.count()) !== 1) throw new Error("no connected dot on the rail");
    const bg = await dot.evaluate((el) => getComputedStyle(el).backgroundColor);
    if (bg === "rgba(0, 0, 0, 0)" || bg === "transparent") throw new Error("dot has no colour");
    return bg;
  });

  await step(run, page, "AMS cards show every slot with its status", async () => {
    const cards = await page.locator(".pr-slot").count();
    if (cards !== 10) throw new Error(`expected 10 slot cards, got ${cards}`);
    const rows = await page.locator(".pr-ams-row").count();
    if (rows !== 3) throw new Error(`expected 3 rows (A, B, External), got ${rows}`);
    const expect = { A2: "✓ Bambu spool", A3: "Not set", B2: "✓ Set", B4: "Empty", "Ext-L": "Not set" };
    for (const [label, badge] of Object.entries(expect)) {
      const got = (await slotCard(label).locator(".pr-status").innerText()).trim();
      if (got !== badge) throw new Error(`${label} shows "${got}", expected "${badge}"`);
    }
    if ((await slotCard("A2").locator(".pr-rfid").count()) !== 1) throw new Error("A2 lacks its RFID badge");
    const meta = await page.locator(".pr-ams-meta").first().innerText();
    if (!meta.includes("Humidity 21%")) throw new Error(`AMS A meta: ${meta}`);
  });

  await step(run, page, "errors show their text or the wiki link", async () => {
    const errors = await page.locator(".pr-error").count();
    if (errors !== 2) throw new Error(`expected 2 errors, got ${errors}`);
    const text = await page.locator(".pr-error-text").first().innerText();
    if (!text.includes("heatbed")) throw new Error(`error text: ${text}`);
    const href = await page.locator(".pr-error-link").getAttribute("href");
    if (!href.startsWith("https://wiki.bambulab.com/")) throw new Error(`wiki link: ${href}`);
  });

  await page.screenshot({ path: `flow-${engine}-printer.png`, fullPage: true });

  await step(run, page, "assigning a preset shows the steps to set it on the printer", async () => {
    await slotCard("A3").locator(".pr-slot-main").click();
    await page.waitForSelector(".pr-picker .pr-picker-item", { timeout: 10000 });
    await page.fill(".pr-picker-search", "PolyLite");
    await page.locator(".pr-picker-item", { hasText: "PolyLite" }).first().click();
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "printer_assign_slot"), null, {
      timeout: 5000,
    });
    const [assign] = await ipcCalls("printer_assign_slot");
    if (assign.args.amsId !== 0 || assign.args.trayId !== 2 || !assign.args.presetPath.endsWith("PolyLite PLA @BBL X1C 0.4 nozzle.json")) {
      throw new Error(JSON.stringify(assign.args));
    }
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
    const badge = (await slotCard("A3").locator(".pr-status").innerText()).trim();
    if (badge !== "Set on printer") throw new Error(`A3 shows "${badge}"`);
    const steps = (await slotCard("A3").locator(".pr-steps").innerText()).replace(/\s+/g, " ");
    for (const want of [
      "On the printer: Filament → A3 → choose Polymaker PolyLite PLA @BBL X1C 0.4 nozzle.",
      "Or in Bambu Studio: Device → AMS → A3 → Polymaker PolyLite PLA @BBL X1C 0.4 nozzle.",
      "It must sync to Bambu Cloud first — open Bambu Studio while signed in.",
    ]) {
      if (!steps.includes(want)) throw new Error(`steps lack "${want}": ${steps}`);
    }
  });

  await step(run, page, "the card flips to ✓ Set when the printer reports the preset", async () => {
    const matched = a3Matched();
    await emitEvent("printer://state", matched);
    await page.waitForFunction(
      () => document.querySelector('.pr-slot[data-label="A3"] .pr-status')?.innerText.trim() === "✓ Set",
      null,
      { timeout: 5000 }
    );
    if ((await slotCard("A3").locator(".pr-steps").count()) !== 0) throw new Error("steps still shown");
  });

  await step(run, page, "losing the printer greys the page and says why", async () => {
    await emitEvent("printer://connection", { state: "unreachable" });
    await page.waitForSelector(".pr-body.pr-stale", { timeout: 5000 });
    const notice = (await page.locator(".pr-notice").innerText()).trim();
    if (notice !== "Can't reach the printer at 192.168.1.20.") throw new Error(`notice: ${notice}`);
    if ((await page.locator('.printer-dot[data-state="error"]').count()) !== 1) throw new Error("dot not in error");
    await emitEvent("printer://connection", { state: "connected" });
    await page.waitForSelector(".pr-body:not(.pr-stale)", { timeout: 5000 });
  });
```

- [ ] **Step 6: Verify everything**

Run: `cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: no output.

Run: `cargo check --target wasm32-unknown-unknown`
Expected: `Finished`, and no warnings from `src/printer/`, `src/pages/printer.rs` or `src/components/printer_settings.rs`.

Run: `cargo test --bin bambumate`
Expected: `test result: ok. 20 passed; 0 failed`

Run: `cd src-tauri && cargo test`
Expected: all `ok`.

Run: `trunk build`
Expected: `✅ success`

Run: `cd tests/webkit && node app-flows.mjs ../..`
Expected output includes, for both engines:

```
  OK   printer page points to Settings when no printer is set up
  OK   printer hero shows the current print
  OK   rail dot shows the connection  rgb(74, 158, 92)
  OK   AMS cards show every slot with its status
  OK   errors show their text or the wiki link
  OK   assigning a preset shows the steps to set it on the printer
  OK   the card flips to ✓ Set when the printer reports the preset
  OK   losing the printer greys the page and says why
```

and ends with `webkit: 55/55 steps passed`, `chromium: 55/55 steps passed` and `PASS: every flow completed in both engines.` The dot colour is `--nd-success` on `main`; after PR #25 it becomes `--nd-signal`.

Run: `cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../..`
Expected: both pass (`style/printer.css` uses only properties WebKit parses).

Look at `tests/webkit/flow-webkit-printer.png`: the hero, three AMS rows and the error list should all render.

- [ ] **Step 7: Commit**

```bash
git add src/pages/printer.rs src/pages/mod.rs src/app.rs src/components/sidebar.rs style/printer.css tests/webkit/app-flows.mjs
git commit -m "Add the Printer page with live status, AMS slot cards, slot picker and rail dot"
```

---

## Manual acceptance on the user's H2 (after Task 10)

These are the spec's acceptance steps; they need the real printer.

1. Discover and connect:
   - Settings → Printer → **Find printers**, pick the H2, and type the access code from the printer screen (Settings → LAN).
   - **Test connection** should show `Connected to H2D. …`, or the fingerprint with **Trust this printer**. That is expected if the H2 uses a CA we don't bundle; trust it, and note which case happened.
2. During a print:
   - The hero percent, layer and remaining time update.
   - Both nozzle temperatures match the printer screen, and the active nozzle is outlined.
3. The AMS cards match the printer screen: material, colour, RFID badge and remaining %. Check that `Ext-L`/`Ext-R` are on the correct sides.
4. Assign a preset to a third-party spool slot:
   - The card shows **Set on printer** with the steps.
   - Set it on the printer, and the card flips to **✓ Set** with no click.
5. Capture real payloads: run with `RUST_LOG=info,bambumate_tauri::printer::payload=debug`. The service logs each report at debug level under that target. Replace `src-tauri/src/printer/testdata/h2d_full.json` and the delta fixtures with trimmed real ones. Keep the serial; remove nothing else identifying beyond the IP.
6. Run `cd src-tauri && cargo test --lib printer::` again against the real fixtures and fix any field-name drift.

## Self-review

**Spec coverage:**

| Spec | Task |
|---|---|
| §1 Transport, user `bblp`, password = access code, rumqttc + rustls | 3 |
| §1 Chain to bundled Bambu CAs; CN = serial; sources recorded | 2 (Protocol facts table) |
| §1 Trust on first use: fingerprint, pin in settings, CN still checked, changed cert asks again | 2 (verifier), 3 (`CertUntrusted`), 6 (`pinned_fingerprint`), 9 (UI) |
| §1 Topics; pushall on connect and ≤ 1 per 5 min; get_version once on connect | 3 |
| §1 Backoff 2 s → 60 s | 3 |
| §1 Connection states | 3 (+ `WrongSerial`, deviation 4) |
| §2 Discovery on UDP 2021 (+1900) | 6 (+1990, deviation 9) |
| §2 Access code in keychain under `bambumate-printer-access-code` / serial; never logged or sent back | 6, 7, 9 |
| §2 Settings: IP, serial, name, model, pinned fingerprint | 6 |
| §2 Test connection waits for the first full report | 3 (`test_connection`), 7, 9 |
| §3 `client.rs` output is raw reports + states only | 3 |
| §3 `state.rs` full replaces, deltas merge; the listed fields | 1 |
| §3 `hms.rs` public JSON, first need, 7-day cache, wiki fallback | 4 |
| §3 `service.rs`, `printer://state` ≤ 2/s, `printer://connection`; start/stop/restart | 7 |
| §3 Parsing: optional fields, unknown ignored, malformed skipped at debug | 1, 7 |
| §4 Printer page on the rail with the dot colours | 10 |
| §4 `.nd` root; hero; AMS rows and cards; external spools last; errors | 10 |
| §4 Not configured → Settings; disconnected → greyed with status | 10 |
| §5 Slot picker: user first, then system, with search | 10 |
| §5 `slot_assignments` in the history DB; external `ams_id = 255` | 5 (+ `preset_path`, deviation 5) |
| §5 Status rules and copy; cloud-sync note | 5, 10 |
| §5 Labels A1–D4, Ext-L/Ext-R/Ext | 5 |
| §5 Card flips to ✓ on a matching report | 5 (unit), 10 (flow) |
| §6 `bm_printer_status`, `bm_ams_slots`, no IP, no write tools, the two messages | 8 |
| §7 Wrong code → `AuthFailed` copy; unreachable copy; logs | 3, 9, 10 |
| Testing: parser fixtures (H2 full, H2 deltas, P1 delta), delta keeps fields, dual nozzles, multiple AMS, external, RFID, malformed | 1 |
| Testing: slot status and labels | 5 |
| Testing: TLS verifier (5 cases) | 2 |
| Testing: in-process TLS broker: connect, subscribe, pushall, report, state, reconnect, auth failure | 3 (client), 7 (service state) |
| Testing: HMS cache hit and miss with a local stub; offline fallback | 4 |
| Testing: WebKit flows (hero, cards, error text, assign, mismatch steps, flip, discovery, trust) | 9, 10 |

**Placeholder scan:** every code step contains the full code. No step says "similar to" or leaves anything to fill in.

**Type consistency:**
- `ConnectionState` serializes as `{"state": …}` in `client.rs` and deserializes the same way in `src/printer/types.rs`.
- `SlotStatus` is `snake_case` on both sides.
- `PrinterView`'s fields match on both sides, and the frontend adds `#[serde(default)]`.
- Command argument names are camelCase in `bridge.rs` and snake_case in `commands/printer.rs`.
- `PrinterView::unconfigured` (Task 7) is used by Task 8.
- `SlotAssignment` (Task 5) is used by Tasks 7 and 8.
- `testpki`, `testbroker` and `fixtures` are `pub(crate)` or test-private and are only used under `cfg(test)`.

**Concerns carried forward:**
- **Merge overlap with PR #26 (preset cloud sync).** It also edits `src-tauri/src/history/store.rs` (a `generated_presets` table) and `history/mod.rs`. Resolve by keeping both `CREATE TABLE` blocks and both method groups.
- **Merge overlap with PR #25 (design system).** It reworks `style/main.css`, `src/components/sidebar.rs` (icon rail) and tokens. After it merges, move the Printer entry onto the rail as an icon with the same `.printer-dot`, and drop the `--nd-success` fallback.
- **Printer MQTT client limit.** Bambu printers accept a small number of concurrent LAN MQTT clients. Bambu Studio, Bambu Handy (via cloud) and BambuMate's service plus a Test connection may compete. If the H2 refuses connections during acceptance, Test connection should reuse the running service's connection when the settings are unchanged.
