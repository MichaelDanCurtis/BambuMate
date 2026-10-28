# LAN Camera Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the printer's camera on Printer → Live and as a Home thumbnail, over the LAN only, and let the print journal photograph each print at key moments.

**Architecture:**
- **Backend:** a new `src-tauri/src/camera/` module.
  - Two transports: port-6000 JPEG for P1/A1, and RTSPS for X1/H2/P2S, through `retina` behind a loopback TLS relay.
  - One shared `CameraHub` session per printer, with keyframe-to-JPEG through `openh264`.
  - Tauri commands that stream frames to the webview over a raw-bytes `Channel`.
- **Journal:** the recorder asks the hub for snapshots at moments derived from the watcher's events.
- **Frontend:** a new `src/camera/` module. WebCodecs `VideoDecoder` decodes H.264 onto a canvas, and stills are the fallback.

**Tech Stack:**
- Tauri 2.10, with `tauri::ipc::Channel<InvokeResponseBody>`.
- tokio, rustls 0.23 with tokio-rustls 0.26, retina 0.4.21, openh264 0.9.9 (`source` feature), image 0.25.
- Leptos 0.8 CSR with `wasm_bindgen(inline_js)`.
- Playwright WebKit and Chromium flows.

**Spec:** `docs/superpowers/specs/2026-10-27-lan-camera-design.md`

## Global Constraints

- **Read-only:** MQTT stays read-only (`pushall`, `get_version`). The camera connections only do the stream handshake. Never send printer commands, toggle liveview, or touch the light.
- **Ports and protocols:**
  - RTSPS URL path `/streaming/live/1`, port 322, user `bblp`, password = LAN access code, Digest auth, TCP interleaved.
  - Port 6000: implicit TLS, then an 80-byte auth packet of little-endian u32s `0x40`, `0x3000`, `0`, `0`, then `bblp` NUL-padded to 32 bytes, then the access code NUL-padded to 32 bytes.
  - Each port-6000 frame is a 16-byte header: u32 LE size, then `0`, `1`, `0`. Then exactly `size` JPEG bytes, starting `FF D8` and ending `FF D9`.
- **Certificates:** always dial the configured printer IP, never the host inside `rtsp_url`. TLS uses `printer::tls::PrinterCertVerifier` and `printer::tls::client_config` (TLS 1.2). The server name doesn't matter, since the verifier checks the CN equals the serial.
- **Limits:**
  - Any frame or message larger than 4 MiB (`4 * 1024 * 1024`) is refused.
  - The session closes 10 s after the last consumer.
  - Stall or reconnect after 10 s without a frame, with backoff from 1 s doubling to 30 s.
  - A snapshot or still waits at most 15 s.
  - A cached still serves if it is at most 5 s old.
  - Keyframes are decoded to the still cache at most every 2 s, unless someone is waiting.
  - Snapshot JPEGs use quality 85, with the longest side capped at 1920 px.
- **Channel messages:** byte 0 is the tag (`1` config, `2` access unit, `3` JPEG).
  - Byte 1 is a flag: keyframe `1` or `0` for an access unit, `0` otherwise.
  - Config payload: a u16 big-endian codec-string length, the codec string (ASCII), then the avcC bytes.
  - Access-unit payload: AVC length-prefixed NAL units with 4-byte lengths.
- **Preferences:** `camera_live_view` and `camera_snapshots` are stored through the existing preferences store (`preferences.json`). Both are on unless the stored value is `"false"`.
- **Events:** `camera://state` carries `CameraState`.
- **Journal moments:**

  | Label | Fires when |
  |---|---|
  | `FIRST LAYER` | the job's own layer is 2 or 3 |
  | `50 %` | percent is 50–55 |
  | `FINISHED` | Ended Finished |
  | `FAILED` | Ended Failed |
  | `PAUSED` | PAUSE while `print_error` is set |

  - Each moment is taken at most once per entry. Calibration prints are photographed too (user ruling, 2026-10-27).
  - Nothing is taken or noted while `camera_snapshots` is off.
  - Journal schema version 3: `print_photos.label TEXT` and `prints.snapshots_json TEXT`.
- **UI strings (exact):**

  | State | Text |
  |---|---|
  | Connecting | `[CONNECTING…]` |
  | Liveview off | `Turn on LAN Mode Liveview` (title), then `On the printer screen: Settings → General (or LAN Only) → LAN Mode Liveview.` |
  | Busy | `The camera is in use by another app. Close the live view in Bambu Studio or Handy, then retry.` |
  | No camera | `This printer has no camera.` |
  | Not connected | `The printer isn't connected.` |
  | Not configured | `No printer is set up.` |
  | Waiting | `Waiting for the printer's camera details…` |
  | Unsupported | `BambuMate can't show this printer's camera yet.` |
  | Disabled | `Live view is off in Settings.` |
  | Auth | `The printer refused the camera login. Check the access code in Settings.` |
  | Tls | `Couldn't verify the printer's camera.` |
  | Refused | `The printer didn't open its camera. Turn on LAN Mode Liveview on the printer, or close the live view in another app.` |
  | Error | `[ERROR: <message>]` with a `Retry` button |

  - The still fallback note is `STILL · <HH:MM:SS>`, and the caption is `LIVE · <MODEL>`.
  - A missed snapshot shows `No camera snapshot: <reason>`.
  - Settings shows a `Camera` section with the switches `Live view` and `Journal snapshots`.
- **Logging:** the access code never reaches a log line or an error string. Logged URLs have their credentials removed.
- **Design:** Nothing design tokens only (`--nd-*`), in both themes. No shadows, gradients or toasts, and no radius above 12px on cards. One hero per page; the camera panel is not a hero. No second PageHeader inside a hub.
- **Rust conventions:**
  - Use `try_*` signal reads and writes after any `.await` in the frontend.
  - Format single files only with `rustfmt --edition 2021 <file>`, never crate-wide `cargo fmt`.
- **Tests:**
  - Tests never reach a real printer, never spend AI tokens, and never launch Bambu Studio or call `open`.
  - Bambu Studio is AGPL: no code is copied from it.
- **Commits:**
  - Stage files by name. Never commit `.omc/`, `.claude/`, `.superpowers/`, `dist/` or PNGs, and never run `git stash`.
  - No attribution lines in commits.
- **Test commands (from the repo root):**
  - Backend: `cargo test --manifest-path src-tauri/Cargo.toml <filter>` and `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` (as CI runs it). `main` already has clippy warnings under Rust 1.98, so `-D warnings` isn't usable. The rule is: **no clippy warning in a file this task creates or changes.** Check with `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets 2>&1 | grep -E "src/(camera|commands/camera)"`, plus each changed file's path.
  - Frontend native unit tests: `cargo test --bin bambumate <filter>`, and `cargo check --target wasm32-unknown-unknown`.
  - Browser flows: `trunk build`, then `cd tests/webkit && node app-flows.mjs ../..`, plus `node layout.mjs ../..` and `node contrast.mjs ../..` for UI tasks.

---

## File Structure

**Backend (`src-tauri/src/`)**

| File | Status | Responsibility |
|---|---|---|
| `printer/state.rs` | modify | `IpCam` and `LanLiveview`, parsed from `print.ipcam`; `PrinterState.ipcam` |
| `camera/mod.rs` | new | `Frame`, `CameraError`, constants, module list |
| `camera/transport.rs` | new | `Transport`, `OffReason`, `choose()` |
| `camera/target.rs` | new | `CameraTarget`, `TargetKey`, the `TargetSource` trait; `ServiceTargets` (the app's printer and keychain); `HubSnapshots` (journal glue); `camera_pref` |
| `camera/wire.rs` | new | Channel message encoding |
| `camera/jpeg6000.rs` | new | Auth packet, `FrameReader`, `run()` |
| `camera/h264.rs` | new | avcC parsing, AVC to Annex B, keyframe to JPEG (openh264 + image) |
| `camera/rtsps.rs` | new | Loopback TLS relay, retina session, `run()` |
| `camera/session.rs` | new | `CameraHub`: shared session, consumers, linger, still cache, capture, errors, retry, events |
| `camera/testcam.rs` | new, `cfg(test)` | Fake TLS camera server and test targets |
| `commands/camera.rs` | new | `camera_state`, `camera_watch`, `camera_unwatch`, `camera_still`, `camera_retry`; wire fixture test |
| `commands/mod.rs`, `lib.rs` | modify | Register the module, commands, hub and snapshot wiring |
| `printer/service.rs` | modify | `camera_facts()` accessor |
| `journal/watch.rs` | modify | `JournalEvent::Paused { key }` |
| `journal/moments.rs` | new | `Moment`, `due()` |
| `journal/types.rs` | modify | `Photo.label`, `PrintEntry.snapshot_note` |
| `journal/store.rs` | modify | Schema v3, `claim_snapshot`, `add_snapshot`, `note_snapshot_miss` |
| `journal/recorder.rs` | modify | `Snapshots` trait, `use_snapshots`, capture threads |
| `Cargo.toml` | modify | `openh264`, `retina`, `futures`; `tokio-rustls` moves to `[dependencies]` |

**Frontend (`src/`)**

| File | Status | Responsibility |
|---|---|---|
| `camera/mod.rs` | new | Module list |
| `camera/types.rs` | new | `CameraState` mirrors, `Message` decoding, `status_text()`; wire fixture test |
| `camera/wire.json` | new | Written by the backend test |
| `camera/shim.rs` | new | `inline_js` WebCodecs, Channel and blob helpers with their extern bindings |
| `camera/bridge.rs` | new | Invoke wrappers and the channel watch |
| `camera/view.rs` | new | `CameraPanel` (Printer → Live) and `CameraThumb` (Home) |
| `style/camera.css` | new, linked from `index.html` | Panel and thumbnail styles |
| `pages/printer.rs` | modify | Panel beside the hero |
| `pages/home.rs` | modify | Thumbnail in the printing hero |
| `pages/journal_detail.rs` and `journal/types.rs` | modify | Photo labels and the miss note |
| `pages/settings.rs` | modify | Camera section |
| `pages/about.rs` | modify | openh264 credit |
| `main.rs` | modify | `mod camera;` |

**Tests**

| File | Change |
|---|---|
| `tests/webkit/fixtures.mjs` | Camera fixtures |
| `tests/webkit/app-flows.mjs` | A mock `Channel` and `VideoDecoder`, and camera flows |

**Docs:** `docs/ROADMAP.md` is updated at the end by the controller.

---

### Task 1: Printer camera facts and transport choice

**Files:**
- Modify: `src-tauri/src/printer/state.rs` (`PrinterState`, `from_print`, tests)
- Create: `src-tauri/src/camera/mod.rs`, `src-tauri/src/camera/transport.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod camera;` after `pub mod calibration;`)

**Interfaces:**
- Produces:
  - `printer::state::{IpCam, LanLiveview}` and `PrinterState.ipcam: Option<IpCam>`.
  - `camera::transport::{Transport, OffReason, choose, RTSPS_PORT, JPEG_PORT, RTSPS_PATH}`.
  - `camera::{Frame, CameraError, MAX_MESSAGE, STATE_EVENT, LIVE_VIEW_PREF, SNAPSHOTS_PREF}`.
  - Later tasks use exactly these names.

- [ ] **Step 1: Write the failing state tests** (append to the `#[cfg(test)] mod tests` in `printer/state.rs`):

```rust
    #[test]
    fn ipcam_is_classified_without_keeping_the_url() {
        let cases = [
            (r#"{"ipcam":{"ipcam_dev":"1","rtsp_url":"disable"}}"#, Some(true), LanLiveview::Off),
            (
                r#"{"ipcam":{"ipcam_dev":"1","rtsp_url":"rtsps://192.168.1.9/streaming/live/1"}}"#,
                Some(true),
                LanLiveview::Rtsp,
            ),
            (r#"{"ipcam":{"ipcam_dev":"0","rtsp_url":""}}"#, Some(false), LanLiveview::Unset),
            (r#"{"ipcam":{"ipcam_dev":1}}"#, Some(true), LanLiveview::Unset),
            (r#"{"ipcam":{}}"#, None, LanLiveview::Unset),
        ];
        for (json, present, liveview) in cases {
            let p: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json).unwrap();
            let s = PrinterState::from_print(&p);
            assert_eq!(s.ipcam, Some(IpCam { present, liveview }), "{json}");
            let text = serde_json::to_string(&s).unwrap();
            assert!(!text.contains("192.168"), "the URL must not be kept: {text}");
        }
        let p: serde_json::Map<String, serde_json::Value> = serde_json::from_str("{}").unwrap();
        assert_eq!(PrinterState::from_print(&p).ipcam, None);
    }

    #[test]
    fn the_h2d_fixture_reports_its_camera() {
        let s = state_after(&["h2d_full.json"]);
        assert_eq!(
            s.ipcam,
            Some(IpCam { present: Some(true), liveview: LanLiveview::Unset })
        );
    }
```

`state_after` is the existing helper at about line 762. If its fixture argument differs (for example, a path under `testdata/`), use it exactly as the neighbouring tests do.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::state::tests::ipcam`
Expected: compile error, because `IpCam` and `LanLiveview` don't exist yet.

- [ ] **Step 3: Implement `IpCam`** in `printer/state.rs`, next to `HmsCode`:

```rust
/// What `print.ipcam` says about the camera. Only a classification of
/// `rtsp_url` is kept: the URL itself carries a host (sometimes a wrong
/// one) and is never needed, as the camera is always dialled at the
/// printer's configured IP.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IpCam {
    /// `ipcam_dev`: `Some(false)` when the printer says it has no camera.
    pub present: Option<bool>,
    pub liveview: LanLiveview,
}

/// `ipcam.rtsp_url`: `"disable"` while LAN Mode Liveview is off, an
/// `rtsp://` or `rtsps://` URL while it is on, else unset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanLiveview {
    Off,
    Rtsp,
    #[default]
    Unset,
}

impl IpCam {
    fn from_value(v: &Value) -> Option<Self> {
        let o = v.as_object()?;
        let present = match o.get("ipcam_dev") {
            Some(Value::String(s)) => match s.trim() {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            },
            Some(Value::Number(n)) => n.as_u64().map(|n| n != 0),
            _ => None,
        };
        let liveview = match o.get("rtsp_url").and_then(Value::as_str).map(str::trim) {
            Some("disable") => LanLiveview::Off,
            Some(u) if u.to_ascii_lowercase().starts_with("rtsp") => LanLiveview::Rtsp,
            _ => LanLiveview::Unset,
        };
        Some(IpCam { present, liveview })
    }
}
```

- Add `pub ipcam: Option<IpCam>,` as the last field of `PrinterState`, with the doc comment `/// \`print.ipcam\`, when the printer has sent it.`
- In `from_print`, add `ipcam: p.get("ipcam").and_then(IpCam::from_value),`.
- Fix any struct-literal `PrinterState { .. }` in tests that doesn't use `..Default::default()`.
- **Merge behaviour:** `ipcam` is an object, so `merge_into` merges it key by key. A diff that changes only `rtsp_url` keeps `ipcam_dev`, which is what we want.

- [ ] **Step 4: Run the state tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::state`
Expected: PASS, including every existing test.

- [ ] **Step 5: Write `camera/mod.rs`**

```rust
//! The printer's camera over the local network: live view and stills.
//!
//! H2, X1 and P2S printers stream H.264 over RTSPS on port 322; P1 and A1
//! printers send JPEG frames over TLS on port 6000. Both log in with the
//! LAN access code and are checked with the same certificate verifier as
//! MQTT. Nothing here ever sends the printer a command: the connections
//! only do the stream handshake.

use std::sync::Arc;

use serde::Serialize;

pub mod transport;

/// The event carrying [`session::CameraState`].
pub const STATE_EVENT: &str = "camera://state";
/// Preference keys; both are on unless set to `"false"`.
pub const LIVE_VIEW_PREF: &str = "camera_live_view";
pub const SNAPSHOTS_PREF: &str = "camera_snapshots";
/// The largest frame, JPEG or channel message accepted.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// One unit from the camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// The H.264 decoder setup: the RFC 6381 codec string (`avc1.640028`)
    /// and the avcC record. Comes before the first video frame and again
    /// whenever it changes.
    Config { codec: String, avcc: Arc<[u8]> },
    /// One H.264 access unit, AVC length-prefixed with 4-byte lengths.
    Video { key: bool, data: Arc<[u8]> },
    /// One JPEG picture (port 6000).
    Jpeg(Arc<[u8]>),
}

/// Why a camera session ended or never started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraError {
    /// The printer closed the stream before any frame: on port 6000 that
    /// is a wrong access code; on RTSPS a 401.
    Auth,
    /// The printer's certificate failed the check.
    Tls,
    /// Port 322 refused or closed the connection at once: LAN Mode
    /// Liveview is off, or another viewer has the camera.
    Refused,
    /// Connected, but no frame came in time: the printer is serving as
    /// many viewers as it can.
    Busy,
    /// Anything else, as a sentence for the page. Never holds the code.
    Other { message: String },
}

impl CameraError {
    /// The sentence the page shows.
    pub fn message(&self) -> String {
        match self {
            CameraError::Auth => {
                "The printer refused the camera login. Check the access code in Settings.".into()
            }
            CameraError::Tls => "Couldn't verify the printer's camera.".into(),
            CameraError::Refused => "The printer didn't open its camera. Turn on LAN Mode \
                 Liveview on the printer, or close the live view in another app."
                .into(),
            CameraError::Busy => "The camera is in use by another app. Close the live view \
                 in Bambu Studio or Handy, then retry."
                .into(),
            CameraError::Other { message } => message.clone(),
        }
    }

    /// Auth, Tls, Refused and Busy wait for Retry; anything else retries
    /// on its own with backoff.
    pub fn needs_retry(&self) -> bool {
        !matches!(self, CameraError::Other { .. })
    }
}
```

Later tasks add `pub mod` lines to this file. Each task adds only its own.

- [ ] **Step 6: Write the failing transport tests** at the bottom of `camera/transport.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::{IpCam, LanLiveview};

    fn cam(present: Option<bool>, liveview: LanLiveview) -> IpCam {
        IpCam { present, liveview }
    }

    #[test]
    fn the_transport_follows_the_printer_and_what_it_reports() {
        use LanLiveview::*;
        let on = cam(Some(true), Rtsp);
        let off = cam(Some(true), Off);
        let unset = cam(Some(true), Unset);
        let none = cam(Some(false), Unset);
        let cases: &[(&str, Option<&IpCam>, Result<Transport, OffReason>)] = &[
            ("H2D", Some(&on), Ok(Transport::Rtsps)),
            ("H2D", Some(&off), Err(OffReason::LiveviewOff)),
            ("H2D", Some(&unset), Ok(Transport::Rtsps)),
            ("H2D", None, Ok(Transport::Rtsps)),
            ("X1 Carbon", Some(&unset), Ok(Transport::Rtsps)),
            ("Bambu Lab P2S", Some(&unset), Ok(Transport::Rtsps)),
            ("H2D Pro", Some(&on), Ok(Transport::Rtsps)),
            ("P1S", Some(&unset), Ok(Transport::Jpeg)),
            ("A1 mini", None, Ok(Transport::Jpeg)),
            ("A1", Some(&off), Err(OffReason::LiveviewOff)),
            ("P1P", Some(&none), Err(OffReason::NoCamera)),
            ("H2S", Some(&none), Err(OffReason::NoCamera)),
            ("Mystery 9", Some(&on), Ok(Transport::Rtsps)),
            ("Mystery 9", Some(&unset), Err(OffReason::Unsupported)),
            ("Mystery 9", None, Err(OffReason::Waiting)),
            ("", None, Err(OffReason::Waiting)),
        ];
        for (model, ipcam, want) in cases {
            assert_eq!(choose(model, *ipcam), *want, "{model} {ipcam:?}");
        }
    }

    #[test]
    fn each_transport_has_its_port() {
        assert_eq!(Transport::Rtsps.port(), 322);
        assert_eq!(Transport::Jpeg.port(), 6000);
    }

    #[test]
    fn every_off_reason_has_its_sentence() {
        assert_eq!(OffReason::NoCamera.message(), "This printer has no camera.");
        assert_eq!(OffReason::NotConnected.message(), "The printer isn't connected.");
        assert_eq!(OffReason::NotConfigured.message(), "No printer is set up.");
        assert_eq!(
            OffReason::Waiting.message(),
            "Waiting for the printer's camera details…"
        );
        assert_eq!(
            OffReason::Unsupported.message(),
            "BambuMate can't show this printer's camera yet."
        );
        assert_eq!(OffReason::Disabled.message(), "Live view is off in Settings.");
        assert_eq!(OffReason::LiveviewOff.message(), "Turn on LAN Mode Liveview");
    }
}
```

- [ ] **Step 7: Implement `camera/transport.rs`** above the tests:

```rust
//! Which camera stream a printer offers, decided from its model and what
//! `print.ipcam` reports. Pure: the hub and the commands call it.

use serde::Serialize;

use crate::printer::model::model_key;
use crate::printer::state::{IpCam, LanLiveview};

pub const RTSPS_PORT: u16 = 322;
pub const JPEG_PORT: u16 = 6000;
pub const RTSPS_PATH: &str = "/streaming/live/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// H.264 over RTSPS on port 322 (H2, X1, P2S).
    Rtsps,
    /// JPEG frames over TLS on port 6000 (P1, A1).
    Jpeg,
}

impl Transport {
    pub fn port(self) -> u16 {
        match self {
            Transport::Rtsps => RTSPS_PORT,
            Transport::Jpeg => JPEG_PORT,
        }
    }
}

/// Why there is no camera to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OffReason {
    NotConfigured,
    NotConnected,
    /// Connected, but the printer hasn't said enough yet.
    Waiting,
    NoCamera,
    /// `rtsp_url` is `"disable"`.
    LiveviewOff,
    /// A model BambuMate doesn't know the camera of.
    Unsupported,
    /// Live view is switched off in Settings.
    Disabled,
}

impl OffReason {
    /// The page's sentence. `LiveviewOff` is the title; the page adds the
    /// steps under it.
    pub fn message(self) -> &'static str {
        match self {
            OffReason::NotConfigured => "No printer is set up.",
            OffReason::NotConnected => "The printer isn't connected.",
            OffReason::Waiting => "Waiting for the printer's camera details…",
            OffReason::NoCamera => "This printer has no camera.",
            OffReason::LiveviewOff => "Turn on LAN Mode Liveview",
            OffReason::Unsupported => "BambuMate can't show this printer's camera yet.",
            OffReason::Disabled => "Live view is off in Settings.",
        }
    }
}

const JPEG_MODELS: &[&str] = &["P1P", "P1S", "A1", "A1MINI"];
const RTSPS_MODELS: &[&str] = &[
    "X1", "X1CARBON", "X1E", "H2D", "H2DPRO", "H2S", "H2C", "P2S",
];

/// The stream to open for `model`, or why there is none. In order: no
/// camera, liveview off, an RTSP URL reported, the model's known family,
/// then waiting (no `ipcam` yet) or unsupported.
pub fn choose(model: &str, ipcam: Option<&IpCam>) -> Result<Transport, OffReason> {
    if ipcam.is_some_and(|c| c.present == Some(false)) {
        return Err(OffReason::NoCamera);
    }
    match ipcam.map(|c| c.liveview) {
        Some(LanLiveview::Off) => return Err(OffReason::LiveviewOff),
        Some(LanLiveview::Rtsp) => return Ok(Transport::Rtsps),
        _ => {}
    }
    let key = model_key(model);
    if JPEG_MODELS.contains(&key.as_str()) {
        Ok(Transport::Jpeg)
    } else if RTSPS_MODELS.contains(&key.as_str()) {
        Ok(Transport::Rtsps)
    } else if ipcam.is_none() {
        Err(OffReason::Waiting)
    } else {
        Err(OffReason::Unsupported)
    }
}
```

`model_key` is `pub(crate)` in `printer/model.rs`. If `model_key("X1 Carbon")` isn't `"X1CARBON"`, or `model_key("Bambu Lab P2S")` isn't `"P2S"`, print the keys in a scratch test and adjust the two lists so the table test passes. Don't change `model_key`.

- [ ] **Step 8: Run the tests and clippy**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::transport printer::state`, then the clippy check from the Global Constraints
Expected: PASS, and no clippy warning in the files this task touched. If `Frame`, `CameraError` or `MAX_MESSAGE` are reported as dead code, add `#[allow(dead_code)]` on that item with the comment `// Used from Task 2 on.`. Task 5 removes these.

- [ ] **Step 9: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/printer/state.rs src-tauri/src/camera/mod.rs src-tauri/src/camera/transport.rs src-tauri/src/lib.rs
git add src-tauri/src/printer/state.rs src-tauri/src/camera/mod.rs src-tauri/src/camera/transport.rs src-tauri/src/lib.rs
git commit -m "feat(camera): read the printer's camera facts and choose its stream"
```

---

### Task 2: Port-6000 JPEG client and the fake camera

**Files:**
- Modify: `src-tauri/Cargo.toml`: move `tokio-rustls = { version = "0.26.4", default-features = false, features = ["ring", "tls12", "logging"] }` from `[dev-dependencies]` to `[dependencies]`, after `rustls`. Keep it out of dev-dependencies.
- Create: `src-tauri/src/camera/target.rs` (only `CameraTarget` and `TargetKey` in this task)
- Create: `src-tauri/src/camera/jpeg6000.rs`
- Create: `src-tauri/src/camera/testcam.rs`
- Modify: `src-tauri/src/camera/mod.rs`: add `pub mod jpeg6000;`, `pub mod target;` and `#[cfg(test)] pub(crate) mod testcam;`

**Interfaces:**
- Consumes: `Frame`, `CameraError`, `MAX_MESSAGE` and `transport::Transport` from Task 1. Also `printer::tls::{client_config, PrinterCertVerifier, RejectionSlot}` and, in tests, `printer::tls::testpki::{TestCa, TestLeaf}` (`TestCa::new(name)`, `ca.pem`, `ca.leaf(serial) -> TestLeaf { cert_der, key_pem }`).
- Produces:
  - `camera::target::{CameraTarget, TargetKey}`.
  - `camera::jpeg6000::{auth_packet, FrameReader, FrameError, is_jpeg, run, connect_tls, FIRST_FRAME_WAIT, STALL}`.
  - `camera::testcam::{FakeCamera, Mode, test_target}`.
  - `connect_tls` is reused by RTSPS in Task 4.

- [ ] **Step 1: Write `camera/target.rs`** (this task's part):

```rust
//! Where the camera is and how to log in: built from the saved printer,
//! its live state and the keychain.

use std::sync::Arc;

use rustls::ClientConfig;
use sha2::{Digest, Sha256};

use super::transport::Transport;

/// One printer's camera. Written by hand so the code never reaches a log.
#[derive(Clone)]
pub struct CameraTarget {
    pub host: String,
    /// `transport.port()` for a real printer; a fake server's in tests.
    pub port: u16,
    pub serial: String,
    pub access_code: String,
    pub tls: Arc<ClientConfig>,
    pub transport: Transport,
    pub model: String,
}

impl std::fmt::Debug for CameraTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("serial", &self.serial)
            .field("transport", &self.transport)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

/// What a running session connects with: a target with another key needs
/// a new session. The code is kept only as a digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetKey {
    host: String,
    port: u16,
    serial: String,
    transport: Transport,
    code: [u8; 32],
}

impl CameraTarget {
    pub fn key(&self) -> TargetKey {
        TargetKey {
            host: self.host.clone(),
            port: self.port,
            serial: self.serial.clone(),
            transport: self.transport,
            code: Sha256::digest(self.access_code.as_bytes()).into(),
        }
    }
}
```

Add one test, `the_debug_form_never_shows_the_code`: build a target with `testcam::test_target` (Step 4) and assert `!format!("{t:?}").contains(code)`.

- [ ] **Step 2: Write the failing `jpeg6000` unit tests** at the bottom of `camera/jpeg6000.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn frame(jpeg: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(jpeg);
        v
    }

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xFF, 0xD9];

    #[test]
    fn the_auth_packet_is_80_bytes_little_endian_and_nul_padded() {
        let p = auth_packet("bblp", "12345678");
        assert_eq!(p.len(), 80);
        assert_eq!(&p[0..4], &0x40u32.to_le_bytes());
        assert_eq!(&p[4..8], &0x3000u32.to_le_bytes());
        assert_eq!(&p[8..16], &[0u8; 8]);
        assert_eq!(&p[16..20], b"bblp");
        assert!(p[20..48].iter().all(|b| *b == 0));
        assert_eq!(&p[48..56], b"12345678");
        assert!(p[56..80].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_long_code_is_cut_to_32_bytes() {
        let p = auth_packet("bblp", &"9".repeat(40));
        assert_eq!(&p[48..80], "9".repeat(32).as_bytes());
    }

    #[test]
    fn frames_are_read_across_split_reads() {
        let mut bytes = frame(JPEG);
        bytes.extend(frame(JPEG));
        let mut r = FrameReader::default();
        let mut got = Vec::new();
        for chunk in bytes.chunks(5) {
            r.push(chunk);
            while let Some(f) = r.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, vec![JPEG.to_vec(), JPEG.to_vec()]);
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_before_it_is_read() {
        let mut r = FrameReader::default();
        let mut header = Vec::new();
        header.extend_from_slice(&((MAX_MESSAGE as u32) + 1).to_le_bytes());
        header.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        r.push(&header);
        assert_eq!(r.next_frame(), Err(FrameError::TooLarge(MAX_MESSAGE + 1)));
    }

    #[test]
    fn a_header_with_the_wrong_constant_words_is_refused() {
        let mut bad = frame(JPEG);
        bad[8] = 2;
        let mut r = FrameReader::default();
        r.push(&bad);
        assert_eq!(r.next_frame(), Err(FrameError::BadHeader));
    }

    #[test]
    fn a_truncated_frame_waits_for_more() {
        let bytes = frame(JPEG);
        let mut r = FrameReader::default();
        r.push(&bytes[..bytes.len() - 1]);
        assert_eq!(r.next_frame(), Ok(None));
        r.push(&bytes[bytes.len() - 1..]);
        assert_eq!(r.next_frame(), Ok(Some(JPEG.to_vec())));
    }

    #[test]
    fn only_complete_jpegs_count() {
        assert!(is_jpeg(JPEG));
        assert!(!is_jpeg(&[0xFF, 0xD8, 0, 0]));
        assert!(!is_jpeg(&[0, 0, 0xFF, 0xD9]));
        assert!(!is_jpeg(&[]));
    }
}
```

- [ ] **Step 3: Implement the pure part of `camera/jpeg6000.rs`**

```rust
//! The P1 and A1 camera: JPEG frames over TLS on port 6000. After the TLS
//! handshake the client sends one 80-byte login; the printer then sends
//! frames, each a 16-byte header (u32 LE size, 0, 1, 0) and the JPEG. A
//! wrong access code makes the printer close the connection unanswered.

use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use super::target::CameraTarget;
use super::{CameraError, Frame, MAX_MESSAGE};

/// How long the first frame may take before the printer counts as busy.
pub const FIRST_FRAME_WAIT: Duration = Duration::from_secs(10);
/// How long a running stream may go without a frame.
pub const STALL: Duration = Duration::from_secs(10);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const USER: &str = "bblp";

/// The login: `0x40`, `0x3000`, `0`, `0` (u32 LE), then the user and the
/// access code, each NUL-padded (or cut) to 32 bytes.
pub fn auth_packet(user: &str, code: &str) -> [u8; 80] {
    let mut p = [0u8; 80];
    p[0..4].copy_from_slice(&0x40u32.to_le_bytes());
    p[4..8].copy_from_slice(&0x3000u32.to_le_bytes());
    for (dst, src) in [(16usize, user.as_bytes()), (48usize, code.as_bytes())] {
        let n = src.len().min(32);
        p[dst..dst + n].copy_from_slice(&src[..n]);
    }
    p
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The header's constant words aren't 0, 1, 0.
    BadHeader,
    TooLarge(usize),
}

/// Cuts the byte stream into frames. Refuses a size over [`MAX_MESSAGE`]
/// before reading it.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame's payload, `Ok(None)` when more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if self.buf.len() < 16 {
            return Ok(None);
        }
        let word = |i: usize| u32::from_le_bytes(self.buf[i..i + 4].try_into().unwrap());
        let size = word(0) as usize;
        if (word(4), word(8), word(12)) != (0, 1, 0) {
            return Err(FrameError::BadHeader);
        }
        if size > MAX_MESSAGE {
            return Err(FrameError::TooLarge(size));
        }
        if self.buf.len() < 16 + size {
            return Ok(None);
        }
        let payload = self.buf[16..16 + size].to_vec();
        self.buf.drain(..16 + size);
        Ok(Some(payload))
    }
}

/// A whole JPEG: starts with SOI (`FF D8`) and ends with EOI (`FF D9`).
pub fn is_jpeg(b: &[u8]) -> bool {
    b.len() >= 4 && b[..2] == [0xFF, 0xD8] && b[b.len() - 2..] == [0xFF, 0xD9]
}
```

- [ ] **Step 4: Write `camera/testcam.rs`**, the fake camera.

```rust
//! A fake printer camera over TLS for tests, using the test PKI. It speaks
//! the port-6000 protocol, or echoes bytes (for the RTSPS relay test).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::target::CameraTarget;
use super::transport::Transport;
use crate::printer::tls::testpki::{TestCa, TestLeaf};
use crate::printer::tls::{client_config, PrinterCertVerifier, RejectionSlot};

pub const SERIAL: &str = "01P00A123456789";
pub const CODE: &str = "12345678";

#[derive(Clone)]
pub enum Mode {
    /// Checks the login, then sends each JPEG `every` apart and stays open.
    Frames { jpegs: Vec<Vec<u8>>, every: Duration },
    /// Reads the login and closes without a byte (a wrong code).
    CloseAfterLogin,
    /// Reads the login and never sends anything (busy).
    Silent,
    /// Echoes whatever it reads (relay tests).
    Echo,
}

pub struct FakeCamera {
    pub addr: SocketAddr,
    pub ca: TestCa,
    /// Every login packet received.
    pub logins: Arc<Mutex<Vec<Vec<u8>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeCamera {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeCamera {
    pub async fn start(mode: Mode) -> Self {
        let ca = TestCa::new("Fake Camera CA");
        let leaf: TestLeaf = ca.leaf(SERIAL);
        let certs = vec![CertificateDer::from(leaf.cert_der.clone())];
        let key = PrivateKeyDer::from_pem_slice(leaf.key_pem.as_bytes()).unwrap();
        let config =
            rustls::ServerConfig::builder_with_provider(crate::printer::tls::crypto_provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let logins = Arc::new(Mutex::new(Vec::new()));
        let seen = logins.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else { return };
                let (acceptor, mode, seen) = (acceptor.clone(), mode.clone(), seen.clone());
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else { return };
                    if let Mode::Echo = mode {
                        let mut buf = [0u8; 4096];
                        while let Ok(n) = tls.read(&mut buf).await {
                            if n == 0 || tls.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                        return;
                    }
                    let mut login = [0u8; 80];
                    if tls.read_exact(&mut login).await.is_err() {
                        return;
                    }
                    seen.lock().unwrap().push(login.to_vec());
                    match mode {
                        Mode::CloseAfterLogin => {
                            let _ = tls.shutdown().await;
                        }
                        Mode::Silent => tokio::time::sleep(Duration::from_secs(3600)).await,
                        Mode::Frames { jpegs, every } => {
                            for jpeg in jpegs.iter().cycle() {
                                let mut out = Vec::with_capacity(16 + jpeg.len());
                                out.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
                                out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
                                out.extend_from_slice(jpeg);
                                if tls.write_all(&out).await.is_err() {
                                    return;
                                }
                                tokio::time::sleep(every).await;
                            }
                        }
                        Mode::Echo => unreachable!(),
                    }
                });
            }
        });
        FakeCamera { addr, ca, logins, task }
    }
}

/// A target for `cam` that trusts its CA.
pub fn test_target(cam: &FakeCamera, transport: Transport) -> CameraTarget {
    let verifier =
        PrinterCertVerifier::with_trust(&cam.ca.pem, &[], SERIAL, None, RejectionSlot::default())
            .unwrap();
    CameraTarget {
        host: "127.0.0.1".into(),
        port: cam.addr.port(),
        serial: SERIAL.into(),
        access_code: CODE.into(),
        tls: client_config(Arc::new(verifier)).unwrap(),
        transport,
        model: "P1S".into(),
    }
}

/// A tiny real JPEG for tests (8×8 grey).
pub fn tiny_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([128, 128, 128]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}
```

`crypto_provider` is `pub fn` in `printer/tls.rs`, and `testpki` is `pub(crate)`. If a name differs, use what `printer/testbroker.rs` uses.

- [ ] **Step 5: Write the failing session tests** in `jpeg6000.rs` tests. These use real sockets, so they run with real time.

```rust
    use crate::camera::testcam::{test_target, tiny_jpeg, FakeCamera, Mode, CODE};
    use crate::camera::transport::Transport;

    #[tokio::test]
    async fn it_logs_in_and_sends_each_jpeg() {
        let jpeg = tiny_jpeg();
        let cam = FakeCamera::start(Mode::Frames {
            jpegs: vec![jpeg.clone()],
            every: std::time::Duration::from_millis(20),
        })
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let target = test_target(&cam, Transport::Jpeg);
        let task = tokio::spawn(async move { run(&target, tx).await });
        for _ in 0..2 {
            let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got, Frame::Jpeg(jpeg.clone().into()));
        }
        drop(rx);
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(ended, CameraError::Other { .. }));
        let login = cam.logins.lock().unwrap()[0].clone();
        assert_eq!(login, auth_packet("bblp", CODE).to_vec());
    }

    #[tokio::test]
    async fn a_close_after_login_is_a_wrong_code() {
        let cam = FakeCamera::start(Mode::CloseAfterLogin).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(run(&test_target(&cam, Transport::Jpeg), tx).await, CameraError::Auth);
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_a_tls_error() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let other = FakeCamera::start(Mode::Silent).await;
        let mut target = test_target(&other, Transport::Jpeg);
        target.port = cam.addr.port();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(run(&target, tx).await, CameraError::Tls);
    }

    #[tokio::test]
    async fn nothing_listening_is_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let cam = FakeCamera::start(Mode::Silent).await;
        let mut target = test_target(&cam, Transport::Jpeg);
        target.port = port;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(run(&target, tx).await, CameraError::Refused);
    }

    #[tokio::test(start_paused = true)]
    async fn no_first_frame_in_time_is_busy() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(run(&test_target(&cam, Transport::Jpeg), tx).await, CameraError::Busy);
    }
```

**Paused time:** `start_paused = true` with real sockets can auto-advance while a socket is pending. If the paused test is flaky, swap it for a test of a private `async fn read_frames(stream, tx, first_wait, stall)` that takes the waits as arguments, using 200 ms waits and real time. Keep the public `run` using the constants.

- [ ] **Step 6: Implement `connect_tls` and `run`** in `jpeg6000.rs`:

```rust
/// TCP then TLS to the target, mapping failures: nothing listening (or
/// a reset/close during the handshake) is `Refused`, a certificate the
/// verifier rejects is `Tls`.
pub async fn connect_tls(target: &CameraTarget) -> Result<TlsStream<TcpStream>, CameraError> {
    let tcp = tokio::time::timeout(
        CONNECT_WAIT,
        TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await
    .map_err(|_| CameraError::Other {
        message: "The printer's camera didn't answer.".into(),
    })?
    .map_err(|_| CameraError::Refused)?;
    let name = ServerName::try_from(target.serial.clone()).map_err(|_| CameraError::Tls)?;
    let connector = TlsConnector::from(Arc::clone(&target.tls));
    match tokio::time::timeout(CONNECT_WAIT, connector.connect(name, tcp)).await {
        Err(_) => Err(CameraError::Other {
            message: "The printer's camera didn't answer.".into(),
        }),
        Ok(Ok(tls)) => Ok(tls),
        Ok(Err(e)) => Err(if is_certificate_error(&e) {
            CameraError::Tls
        } else {
            CameraError::Refused
        }),
    }
}

/// A handshake that failed on the certificate, not on the connection.
fn is_certificate_error(e: &std::io::Error) -> bool {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|r| {
            matches!(
                r,
                rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented
            )
        })
}

/// Streams JPEGs to `tx` until the stream fails or `tx` is closed; returns
/// why it ended.
pub async fn run(target: &CameraTarget, tx: mpsc::Sender<Frame>) -> CameraError {
    let mut tls = match connect_tls(target).await {
        Ok(tls) => tls,
        Err(e) => return e,
    };
    if tls
        .write_all(&auth_packet(USER, &target.access_code))
        .await
        .is_err()
    {
        return CameraError::Refused;
    }
    let mut reader = FrameReader::default();
    let mut buf = vec![0u8; 64 * 1024];
    let mut got_frame = false;
    loop {
        let wait = if got_frame { STALL } else { FIRST_FRAME_WAIT };
        let n = match tokio::time::timeout(wait, tls.read(&mut buf)).await {
            Err(_) if got_frame => {
                return CameraError::Other {
                    message: "The camera stopped sending pictures.".into(),
                }
            }
            Err(_) => return CameraError::Busy,
            Ok(Err(_)) | Ok(Ok(0)) if !got_frame => return CameraError::Auth,
            Ok(Err(_)) | Ok(Ok(0)) => {
                return CameraError::Other {
                    message: "The camera stream ended.".into(),
                }
            }
            Ok(Ok(n)) => n,
        };
        reader.push(&buf[..n]);
        loop {
            match reader.next_frame() {
                Ok(None) => break,
                Ok(Some(jpeg)) => {
                    got_frame = true;
                    if !is_jpeg(&jpeg) {
                        tracing::debug!("camera: dropped a frame that isn't a whole JPEG");
                        continue;
                    }
                    if tx.send(Frame::Jpeg(jpeg.into())).await.is_err() {
                        return CameraError::Other {
                            message: "stopped".into(),
                        };
                    }
                }
                Err(e) => {
                    tracing::warn!("camera: bad frame from the printer: {e:?}");
                    return CameraError::Other {
                        message: "The camera sent something BambuMate couldn't read.".into(),
                    };
                }
            }
        }
    }
}
```

**Stall timing:** a printer that sends nothing but keeps TCP open after some frames hits `STALL`. A partial frame that never completes also stalls, because each read waits `STALL`.

- [ ] **Step 7: Run the tests and clippy**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::`, then the clippy command.
Expected: PASS and no warnings.

- [ ] **Step 8: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/camera/mod.rs src-tauri/src/camera/target.rs src-tauri/src/camera/jpeg6000.rs src-tauri/src/camera/testcam.rs
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/camera/mod.rs src-tauri/src/camera/target.rs src-tauri/src/camera/jpeg6000.rs src-tauri/src/camera/testcam.rs
git commit -m "feat(camera): stream P1 and A1 pictures over port 6000"
```

---

### Task 3: H.264 keyframes to JPEG, and channel messages

**Files:**
- Modify: `src-tauri/Cargo.toml`: add `openh264 = "0.9.9"` (default features, which compile Cisco's source with `cc`), with the comment `# Decodes one H.264 keyframe for journal snapshots and stills (BSD-2-Clause).`
- Create: `src-tauri/src/camera/h264.rs`, `src-tauri/src/camera/wire.rs`
- Modify: `src-tauri/src/camera/mod.rs`: add `pub mod h264;` and `pub mod wire;`

**Interfaces:**
- Consumes: `Frame` and `MAX_MESSAGE` from Task 1.
- Produces:
  - `camera::h264::{AvcConfig, parse_avcc, to_annexb, keyframe_jpeg, encode_jpeg, JPEG_QUALITY, MAX_EDGE}`.
  - `camera::wire::{encode, TAG_CONFIG, TAG_VIDEO, TAG_JPEG}`.
  - `#[cfg(test)] camera::h264::test_stream(w, h) -> (Vec<u8> /*avcc*/, Vec<u8> /*AVC keyframe sample*/)` for Task 5.

- [ ] **Step 1: Write the failing `h264` tests** at the bottom of `h264.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_encoded_keyframe_comes_back_as_a_jpeg_of_the_same_size() {
        let (avcc, sample) = test_stream(64, 48);
        let jpeg = keyframe_jpeg(&avcc, &sample).unwrap();
        let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
    }

    #[test]
    fn avcc_names_its_parameter_sets() {
        let (avcc, _) = test_stream(64, 48);
        let cfg = parse_avcc(&avcc).unwrap();
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.sps.len(), 1);
        assert_eq!(cfg.pps.len(), 1);
        assert_eq!(cfg.sps[0][0] & 0x1F, 7);
        assert_eq!(cfg.pps[0][0] & 0x1F, 8);
    }

    #[test]
    fn annexb_puts_the_parameter_sets_first() {
        let (avcc, sample) = test_stream(64, 48);
        let cfg = parse_avcc(&avcc).unwrap();
        let b = to_annexb(&cfg, &sample).unwrap();
        assert_eq!(&b[..4], &[0, 0, 0, 1]);
        assert_eq!(b[4] & 0x1F, 7);
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        assert!(parse_avcc(&[]).is_err());
        assert!(parse_avcc(&[1, 0x64, 0, 0x28, 0xFF, 0xE1, 0, 50]).is_err());
        let (avcc, _) = test_stream(64, 48);
        let cfg = parse_avcc(&avcc).unwrap();
        assert!(to_annexb(&cfg, &[0, 0, 0, 99, 1, 2]).is_err());
        assert!(keyframe_jpeg(&avcc, &[0, 0, 0, 3, 0x65, 0, 0]).is_err());
    }

    #[test]
    fn a_large_picture_is_scaled_to_the_longest_edge() {
        let rgb = vec![200u8; 2400 * 1200 * 3];
        let jpeg = encode_jpeg(&rgb, 2400, 1200).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (1920, 960));
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::h264`
Expected: compile errors, because nothing is defined yet. The first build compiles openh264's C++ and takes a few minutes.

- [ ] **Step 3: Implement `h264.rs`**

```rust
//! One H.264 keyframe to a JPEG, for journal snapshots and stills. The
//! stream arrives as AVC (length-prefixed NAL units) with an avcC record;
//! openh264 wants Annex B (start codes) with SPS and PPS in front.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

pub const JPEG_QUALITY: u8 = 85;
/// Longest side of a snapshot.
pub const MAX_EDGE: u32 = 1920;

/// What an avcC record holds that decoding needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcConfig {
    /// Bytes in each NAL length prefix (1, 2 or 4).
    pub length_size: usize,
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
}

/// Reads an `AVCDecoderConfigurationRecord` (ISO 14496-15 5.2.4.1).
pub fn parse_avcc(b: &[u8]) -> Result<AvcConfig, String> {
    let bad = || "The camera's video setup couldn't be read.".to_string();
    if b.len() < 7 || b[0] != 1 {
        return Err(bad());
    }
    let length_size = usize::from(b[4] & 0x03) + 1;
    if length_size == 3 {
        return Err(bad());
    }
    let mut at = 5;
    let mut sets = |count: usize, at: &mut usize| -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let len = usize::from(u16::from_be_bytes(
                b.get(*at..*at + 2).ok_or_else(bad)?.try_into().unwrap(),
            ));
            *at += 2;
            out.push(b.get(*at..*at + len).ok_or_else(bad)?.to_vec());
            *at += len;
        }
        Ok(out)
    };
    let sps_count = usize::from(b[at] & 0x1F);
    at += 1;
    let sps = sets(sps_count, &mut at)?;
    let pps_count = usize::from(*b.get(at).ok_or_else(bad)?);
    at += 1;
    let pps = sets(pps_count, &mut at)?;
    if sps.is_empty() || pps.is_empty() {
        return Err(bad());
    }
    Ok(AvcConfig { length_size, sps, pps })
}

/// SPS, PPS, then each NAL unit of `sample`, each after a 4-byte start code.
pub fn to_annexb(cfg: &AvcConfig, sample: &[u8]) -> Result<Vec<u8>, String> {
    const START: [u8; 4] = [0, 0, 0, 1];
    let mut out = Vec::with_capacity(sample.len() + 128);
    for set in cfg.sps.iter().chain(&cfg.pps) {
        out.extend_from_slice(&START);
        out.extend_from_slice(set);
    }
    let mut at = 0;
    while at < sample.len() {
        let len_bytes = sample
            .get(at..at + cfg.length_size)
            .ok_or("The camera sent a cut-off video frame.")?;
        let len = len_bytes.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b));
        at += cfg.length_size;
        let nal = sample
            .get(at..at + len)
            .ok_or("The camera sent a cut-off video frame.")?;
        out.extend_from_slice(&START);
        out.extend_from_slice(nal);
        at += len;
    }
    Ok(out)
}

/// Decodes the keyframe `sample` and encodes it as a JPEG.
pub fn keyframe_jpeg(avcc: &[u8], sample: &[u8]) -> Result<Vec<u8>, String> {
    let cfg = parse_avcc(avcc)?;
    let annexb = to_annexb(&cfg, sample)?;
    let failed = || "The camera's picture couldn't be decoded.".to_string();
    let mut decoder = Decoder::new().map_err(|_| failed())?;
    let yuv = decoder
        .decode(&annexb)
        .map_err(|_| failed())?
        .ok_or_else(failed)?;
    let (w, h) = yuv.dimensions();
    let mut rgb = vec![0u8; w * h * 3];
    yuv.write_rgb8(&mut rgb);
    encode_jpeg(&rgb, w as u32, h as u32)
}

/// RGB8 to a JPEG at [`JPEG_QUALITY`], scaled so neither side is over
/// [`MAX_EDGE`].
pub fn encode_jpeg(rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let img = RgbImage::from_raw(w, h, rgb.to_vec())
        .ok_or("The camera's picture had the wrong size.")?;
    let mut img = DynamicImage::ImageRgb8(img);
    if w.max(h) > MAX_EDGE {
        img = img.resize(MAX_EDGE, MAX_EDGE, FilterType::Triangle);
    }
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&img.to_rgb8())
        .map_err(|_| "The picture couldn't be saved as a JPEG.".to_string())?;
    Ok(out.into_inner())
}
```

Then add the test helper inside `h264.rs`:

```rust
/// An avcC record and one AVC keyframe sample, made with openh264's
/// encoder from a gradient: the tests' stand-in for a printer's stream.
#[cfg(test)]
pub(crate) fn test_stream(w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
    use openh264::encoder::Encoder;
    use openh264::formats::{RgbSliceU8, YUVBuffer};
    let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 90, 160]).collect();
    let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (w, h)));
    let mut enc = Encoder::new().unwrap();
    let annexb = enc.encode(&yuv).unwrap().to_vec();
    let nals: Vec<&[u8]> = openh264::nal_units(&annexb)
        .map(|n| {
            let skip = if n.starts_with(&[0, 0, 0, 1]) { 4 } else { 3 };
            &n[skip..]
        })
        .collect();
    let sps = nals.iter().find(|n| n[0] & 0x1F == 7).unwrap();
    let pps = nals.iter().find(|n| n[0] & 0x1F == 8).unwrap();
    let mut avcc = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
    avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(pps);
    let mut sample = Vec::new();
    for n in nals.iter().filter(|n| !matches!(n[0] & 0x1F, 7 | 8)) {
        sample.extend_from_slice(&(n.len() as u32).to_be_bytes());
        sample.extend_from_slice(n);
    }
    (avcc, sample)
}
```

**openh264 0.9.9 API notes** (from the crate source):
- `Decoder::new()` and `decoder.decode(&[u8]) -> Result<Option<DecodedYUV>, Error>`.
- `DecodedYUV::dimensions()` comes from the `YUVSource` trait, and `write_rgb8(&mut [u8])`.
- `Encoder::new()` and `encoder.encode(&impl YUVSource) -> EncodedBitStream`, with `.to_vec()`.
- `YUVBuffer::from_rgb_source`, and `openh264::nal_units(&[u8])`, which yields NALs with their start codes.

If an import path differs (for example, `RgbSliceU8` lives in `openh264::formats::rgb`), fix the path; don't change the approach. If the decoder needs a flush to return the picture, call `decoder.flush_remaining()` and take the last frame.

- [ ] **Step 4: Write the failing `wire` tests and implement `wire.rs`**

```rust
//! Frames as Channel messages: byte 0 is the tag, byte 1 a flag, then the
//! payload. Config: u16 BE codec length, the codec, the avcC record.
//! Video: the AVC access unit, flag 1 for a keyframe. JPEG: the picture.

use super::Frame;

pub const TAG_CONFIG: u8 = 1;
pub const TAG_VIDEO: u8 = 2;
pub const TAG_JPEG: u8 = 3;

pub fn encode(frame: &Frame) -> Vec<u8> {
    match frame {
        Frame::Config { codec, avcc } => {
            let codec = codec.as_bytes();
            let mut out = Vec::with_capacity(4 + codec.len() + avcc.len());
            out.extend_from_slice(&[TAG_CONFIG, 0]);
            out.extend_from_slice(&(codec.len() as u16).to_be_bytes());
            out.extend_from_slice(codec);
            out.extend_from_slice(avcc);
            out
        }
        Frame::Video { key, data } => {
            let mut out = Vec::with_capacity(2 + data.len());
            out.extend_from_slice(&[TAG_VIDEO, u8::from(*key)]);
            out.extend_from_slice(data);
            out
        }
        Frame::Jpeg(data) => {
            let mut out = Vec::with_capacity(2 + data.len());
            out.extend_from_slice(&[TAG_JPEG, 0]);
            out.extend_from_slice(data);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_frame_kind_has_its_layout() {
        let config = Frame::Config {
            codec: "avc1.640028".into(),
            avcc: vec![1, 2, 3].into(),
        };
        let mut want = vec![1, 0, 0, 11];
        want.extend_from_slice(b"avc1.640028");
        want.extend_from_slice(&[1, 2, 3]);
        assert_eq!(encode(&config), want);
        assert_eq!(
            encode(&Frame::Video { key: true, data: vec![9, 8].into() }),
            vec![2, 1, 9, 8]
        );
        assert_eq!(
            encode(&Frame::Video { key: false, data: vec![7].into() }),
            vec![2, 0, 7]
        );
        assert_eq!(encode(&Frame::Jpeg(vec![0xFF, 0xD8].into())), vec![3, 0, 0xFF, 0xD8]);
    }
}
```

The frontend's decoder (Task 8) mirrors these bytes. Its test uses the same three vectors, so keep them as written.

- [ ] **Step 5: Run the tests and clippy**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::`, then the clippy command.
Expected: PASS.

- [ ] **Step 6: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/camera/mod.rs src-tauri/src/camera/h264.rs src-tauri/src/camera/wire.rs
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/camera/mod.rs src-tauri/src/camera/h264.rs src-tauri/src/camera/wire.rs
git commit -m "feat(camera): decode a keyframe to a JPEG and frame channel messages"
```

---

### Task 4: RTSPS client through a loopback TLS relay

**Files:**
- Modify: `src-tauri/Cargo.toml`: add `retina = "0.4.21"` with the comment `# RTSP client for the H2/X1/P2S camera (MIT OR Apache-2.0). It has no RTSPS, so it runs behind a loopback TLS relay.`, and `futures = "0.3"` if it isn't already a direct dependency.
- Create: `src-tauri/src/camera/rtsps.rs`
- Modify: `src-tauri/src/camera/mod.rs`: add `pub mod rtsps;`

**Interfaces:**
- Consumes: `jpeg6000::connect_tls` (Task 2); `CameraTarget`; `Frame`; `CameraError`; `MAX_MESSAGE`; `transport::RTSPS_PATH`. In tests, `testcam::{FakeCamera, Mode::Echo, test_target}`.
- Produces:
  - `camera::rtsps::{run, relay, Relay, STALL}`.
  - `run(target, tx) -> CameraError` has the same contract as `jpeg6000::run`.

- [ ] **Step 1: Write the failing relay tests** at the bottom of `rtsps.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::testcam::{test_target, FakeCamera, Mode};
    use crate::camera::transport::Transport;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn the_relay_carries_bytes_both_ways_over_tls() {
        let cam = FakeCamera::start(Mode::Echo).await;
        let relay = relay(&test_target(&cam, Transport::Rtsps)).await.unwrap();
        let mut local = tokio::net::TcpStream::connect(relay.addr).await.unwrap();
        local.write_all(b"OPTIONS * RTSP/1.0\r\n\r\n").await.unwrap();
        let mut got = vec![0u8; 22];
        local.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"OPTIONS * RTSP/1.0\r\n\r\n");
    }

    #[tokio::test]
    async fn the_relay_listens_on_loopback_only_and_takes_one_connection() {
        let cam = FakeCamera::start(Mode::Echo).await;
        let relay = relay(&test_target(&cam, Transport::Rtsps)).await.unwrap();
        assert!(relay.addr.ip().is_loopback());
        let _first = tokio::net::TcpStream::connect(relay.addr).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::net::TcpStream::connect(relay.addr),
        )
        .await;
        assert!(!matches!(second, Ok(Ok(_))), "a second connection must not be accepted");
    }

    #[tokio::test]
    async fn an_untrusted_camera_fails_before_any_rtsp() {
        let cam = FakeCamera::start(Mode::Echo).await;
        let other = FakeCamera::start(Mode::Echo).await;
        let mut target = test_target(&other, Transport::Rtsps);
        target.port = cam.addr.port();
        assert_eq!(relay(&target).await.err(), Some(CameraError::Tls));
    }

    #[test]
    fn the_rtsp_url_points_at_the_relay_without_credentials() {
        let u = local_url(4321);
        assert_eq!(u.as_str(), "rtsp://127.0.0.1:4321/streaming/live/1");
        assert!(u.password().is_none());
    }

    #[test]
    fn retina_errors_map_to_camera_errors() {
        assert_eq!(map_error("401 Unauthorized"), CameraError::Auth);
        assert_eq!(map_error("RTSP/1.0 401"), CameraError::Auth);
        assert!(matches!(map_error("connection reset"), CameraError::Other { .. }));
    }
}
```

**Listener close:** the second-connection test depends on the listener being dropped after the first accept. The OS may still complete the TCP handshake from its backlog for a moment. If that happens, assert instead that the second connection reads EOF within 500 ms.

- [ ] **Step 2: Implement `relay`, `local_url` and `map_error`**

```rust
//! The H2, X1 and P2S camera: H.264 over RTSPS on port 322. `retina` only
//! speaks plain RTSP, so a one-connection relay on 127.0.0.1 carries its
//! bytes through TLS (checked like MQTT) to the printer. The TLS handshake
//! happens before retina connects, so a certificate or refusal is
//! reported as such rather than as an RTSP error.
//!
//! Known risk: live555 servers may stop a stream after about 10 s without
//! RTCP receiver reports, which retina doesn't send. A stall is reported
//! and the hub reconnects; to be checked on a real H2.

use std::net::SocketAddr;
use std::time::Duration;

use futures::StreamExt;
use retina::client::{
    Credentials, PlayOptions, SessionOptions, SetupOptions, TcpTransportOptions, Transport,
};
use retina::codec::{CodecItem, ParametersRef};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use url::Url;

use super::jpeg6000::connect_tls;
use super::target::CameraTarget;
use super::transport::RTSPS_PATH;
use super::{CameraError, Frame, MAX_MESSAGE};

/// How long the stream may go without a video frame.
pub const STALL: Duration = Duration::from_secs(10);
const USER: &str = "bblp";

/// A running relay. Dropping it ends the relay task.
pub struct Relay {
    pub addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Opens TLS to the printer, then waits for one local connection on
/// 127.0.0.1 and copies bytes both ways until either side closes.
pub async fn relay(target: &CameraTarget) -> Result<Relay, CameraError> {
    let mut printer = connect_tls(target).await?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| CameraError::Other {
            message: "BambuMate couldn't open a local port for the camera.".into(),
        })?;
    let addr = listener.local_addr().map_err(|_| CameraError::Other {
        message: "BambuMate couldn't open a local port for the camera.".into(),
    })?;
    let task = tokio::spawn(async move {
        let Ok((mut local, peer)) = listener.accept().await else { return };
        drop(listener);
        if !peer.ip().is_loopback() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut local, &mut printer).await;
    });
    Ok(Relay { addr, task })
}

/// The URL retina dials: the relay, with the printer's path. Credentials go
/// in `SessionOptions`, never in the URL.
fn local_url(port: u16) -> Url {
    Url::parse(&format!("rtsp://127.0.0.1:{port}{RTSPS_PATH}")).expect("a valid URL")
}

/// A retina error as a camera error. Its text may echo the URL, never the
/// code, since the code isn't in the URL.
fn map_error(text: &str) -> CameraError {
    if text.contains("401") {
        CameraError::Auth
    } else {
        CameraError::Other {
            message: "The camera stream failed.".into(),
        }
    }
}
```

- [ ] **Step 3: Implement `run`** (not unit-tested: it needs a real RTSP server; the H2 hardware check covers it):

```rust
/// Streams the printer's H.264 to `tx`: `Frame::Config` first and whenever
/// the parameters change, then every access unit. Frames before the first
/// keyframe are dropped. Returns why it ended.
pub async fn run(target: &CameraTarget, tx: mpsc::Sender<Frame>) -> CameraError {
    let relay = match relay(target).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let creds = Credentials {
        username: USER.into(),
        password: target.access_code.clone(),
    };
    let options = SessionOptions::default()
        .creds(Some(creds))
        .user_agent("BambuMate".into());
    let mut session = match retina::client::Session::describe(local_url(relay.addr.port()), options).await {
        Ok(s) => s,
        Err(e) => return map_error(&e.to_string()),
    };
    let Some(video) = session
        .streams()
        .iter()
        .position(|s| s.media() == "video" && s.encoding_name().eq_ignore_ascii_case("h264"))
    else {
        return CameraError::Other {
            message: "The printer's camera doesn't offer H.264 video.".into(),
        };
    };
    if let Err(e) = session
        .setup(
            video,
            SetupOptions::default().transport(Transport::Tcp(TcpTransportOptions::default())),
        )
        .await
    {
        return map_error(&e.to_string());
    }
    let playing = match session.play(PlayOptions::default()).await {
        Ok(p) => p,
        Err(e) => return map_error(&e.to_string()),
    };
    let mut demuxed = match playing.demuxed() {
        Ok(d) => d,
        Err(e) => return map_error(&e.to_string()),
    };
    let mut sent_config = false;
    let mut seen_key = false;
    loop {
        let item = match tokio::time::timeout(STALL, demuxed.next()).await {
            Err(_) => {
                return CameraError::Other {
                    message: "The camera stopped sending video.".into(),
                }
            }
            Ok(None) => {
                return CameraError::Other {
                    message: "The camera stream ended.".into(),
                }
            }
            Ok(Some(Err(e))) => return map_error(&e.to_string()),
            Ok(Some(Ok(item))) => item,
        };
        let CodecItem::VideoFrame(frame) = item else { continue };
        if !sent_config || frame.has_new_parameters() {
            let params = demuxed.streams()[video].parameters();
            let Some(ParametersRef::Video(v)) = params else { continue };
            let config = Frame::Config {
                codec: v.rfc6381_codec().to_string(),
                avcc: v.extra_data().to_vec().into(),
            };
            if tx.send(config).await.is_err() {
                return CameraError::Other { message: "stopped".into() };
            }
            sent_config = true;
        }
        let key = frame.is_random_access_point();
        seen_key |= key;
        if !seen_key {
            continue;
        }
        let data = frame.into_data();
        if data.len() > MAX_MESSAGE {
            tracing::warn!("camera: dropped a {}-byte video frame", data.len());
            continue;
        }
        if tx.send(Frame::Video { key, data: data.into() }).await.is_err() {
            return CameraError::Other { message: "stopped".into() };
        }
    }
}
```

**retina 0.4.21 API notes** (read from the crate source):
- `Session::describe(Url, SessionOptions)`, `.streams()`, `Stream::media()`, `Stream::encoding_name()`.
- `.setup(i, SetupOptions::default().transport(Transport::Tcp(TcpTransportOptions::default())))`.
- `.play(PlayOptions::default())`, then `.demuxed()`, which is a `futures::Stream` of `Result<CodecItem, Error>`.
- `Demuxed::streams()` and `Stream::parameters() -> Option<ParametersRef>`.
- `ParametersRef::Video(v)` with `v.rfc6381_codec()` and `v.extra_data()`.
- `VideoFrame::{is_random_access_point, has_new_parameters, into_data}`.

If a name differs in this version, use the crate's own example or source; don't change the approach. If `rfc6381_codec()` returns `Option<&str>` for video, skip the frame on `None`. Fix the `Credentials` field names to whatever the crate's struct uses.

- [ ] **Step 4: Run the tests and clippy**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::rtsps`, then the clippy command.
Expected: PASS.

- [ ] **Step 5: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/camera/mod.rs src-tauri/src/camera/rtsps.rs
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/camera/mod.rs src-tauri/src/camera/rtsps.rs
git commit -m "feat(camera): stream H2 video over RTSPS through a loopback TLS relay"
```

---
### Task 5: The camera hub (one shared session)

**Files:**
- Modify: `src-tauri/src/camera/target.rs`: add the `TargetSource` trait.
- Create: `src-tauri/src/camera/session.rs`.
- Modify: `src-tauri/src/camera/mod.rs`: add `pub mod session;` and `pub use session::{CameraHub, CameraState, CameraStatus};`. Remove any `#[allow(dead_code)]` from Task 1.

**Interfaces:**
- Consumes: `Frame`, `CameraError` (`needs_retry`, `message`), `transport::{Transport, OffReason}`, `target::CameraTarget` (`key`), `jpeg6000::run`, `rtsps::run`, `h264::keyframe_jpeg`. In tests, `h264::test_stream` and `testcam::tiny_jpeg`.
- Produces (Tasks 6, 7 and 8 rely on these exact names):

```rust
// camera/target.rs
pub trait TargetSource: Send + Sync + 'static {
    /// Where the camera is now, or why there is none. May block (keychain).
    fn resolve(&self) -> Result<CameraTarget, OffReason>;
    /// Whether a running session for `target` still fits the printer: same
    /// printer and code, connected, and the same transport chosen. Cheap,
    /// no keychain.
    fn still_valid(&self, target: &CameraTarget) -> Result<(), OffReason>;
}

// camera/session.rs
pub type RunFuture = Pin<Box<dyn Future<Output = CameraError> + Send + 'static>>;
pub trait Connector: Send + Sync + 'static {
    fn run(&self, target: CameraTarget, tx: mpsc::Sender<Frame>) -> RunFuture;
}
pub struct LanConnector;                       // jpeg6000::run or rtsps::run by transport
pub trait CameraEvents: Send + Sync + 'static { fn state(&self, state: &CameraState); }
pub struct TauriCameraEvents(pub tauri::AppHandle);   // emits STATE_EVENT

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CameraState {
    pub transport: Option<Transport>,
    pub status: CameraStatus,
    /// Unix seconds of the last frame.
    pub last_frame_at: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraStatus {
    Idle,
    Connecting,
    Streaming,
    Off { reason: OffReason, message: String },
    Error { error: CameraError, message: String },
}
impl CameraState {
    pub fn off(reason: OffReason) -> Self;      // transport None, status Off with reason.message()
}

#[derive(Debug, Clone)]
pub struct Timing {
    pub linger: Duration,          // 10 s
    pub recheck: Duration,         // 5 s: re-resolve while Off, still_valid while streaming
    pub backoff_start: Duration,   // 1 s
    pub backoff_max: Duration,     // 30 s
    pub capture_timeout: Duration, // 15 s
    pub still_max_age: Duration,   // 5 s
    pub decode_every: Duration,    // 2 s
    pub state_every: Duration,     // 1 s: the most often last_frame_at alone is emitted
}
impl Default for Timing { /* the values above */ }

#[derive(Clone)]
pub struct CameraHub { /* Arc inside */ }
impl CameraHub {
    pub fn new(source: Arc<dyn TargetSource>, connector: Arc<dyn Connector>,
               events: Arc<dyn CameraEvents>, timing: Timing) -> Self;
    pub fn state(&self) -> CameraState;
    /// The state now; while no session runs, resolves the target (on a
    /// blocking thread) so the page can say why there is no camera.
    pub async fn probe(&self) -> CameraState;
    /// Adds a viewer. `sink` gets every frame it should see (the config,
    /// then video from a keyframe on, or each JPEG); it returns false once
    /// the webview is gone, and is then dropped.
    pub fn watch(&self, sink: Box<dyn FnMut(&Frame) -> bool + Send>) -> u64;
    pub fn unwatch(&self, id: u64);
    /// The latest picture if at most `still_max_age` old, else the next one
    /// (at most `capture_timeout`). `None` when there is none.
    pub async fn still(&self) -> Option<Vec<u8>>;
    /// A picture taken after this call, or why not (a sentence). A session
    /// stopped on an error is tried once more.
    pub async fn capture(&self) -> Result<Vec<u8>, String>;
    /// Clears a stopped error and connects again if anyone is watching.
    pub fn retry(&self) -> CameraState;
}
```

**Behaviour** (from the spec; each line has a test below):
1. **One session.** The first consumer (watcher, still or capture) starts one driver task. Later consumers share it, and the connector is called once.
2. **Driver loop:**
   - It sets `Connecting`, then resolves on `tauri::async_runtime::spawn_blocking`.
   - `Err(reason)` sets `Off { reason }` and waits `recheck` before resolving again.
   - `Ok(target)` sets `transport` and runs the connector, reading frames.
   - The first frame sets `Streaming` and resets the backoff.
   - Every `recheck` while running, it calls `still_valid(&target)`. An `Err(reason)` drops the run future (closing the connection), sets `Off`, and goes back to resolving.
3. **Run ends with an error:**
   - `needs_retry()` sets `Error { error, message }`. The driver then waits for `retry()`, a capture, or the consumers going to zero.
   - Otherwise it waits the backoff (`backoff_start`, doubling to `backoff_max`) and goes back to step 2.
4. **Frames:**
   - `last_frame_at` updates on every frame, and the state is emitted when the status changes. `last_frame_at` alone is emitted at most every `state_every`.
   - `Config` is cached.
   - `Jpeg` replaces the still cache.
   - A keyframe `Video` is decoded with `keyframe_jpeg` on a blocking thread when a still or capture is waiting, or when the cache is older than `decode_every`. A decode error is logged at debug and skipped.
5. **Sinks:**
   - A new sink first gets the cached `Config`, if any.
   - Video is skipped until a keyframe after the sink's config. After a new `Config`, each sink waits for a keyframe again.
   - JPEGs always go through.
   - A sink returning `false` is removed.
6. **Linger:** when consumers reach zero, a timer runs. After `linger` with still none (and no newer consumer in between), the driver is aborted (dropping the connection) and the state becomes `Idle`. A consumer arriving within the linger cancels it.
7. **`capture`:**
   - It counts as a consumer while it waits.
   - If the status is `Error`, it first does what `retry()` does.
   - It returns the first still cached after it was called.
   - It returns `Err(reason.message())` as soon as the status is `Off`, and `Err(message)` on a new `Error`.
   - After `capture_timeout` it returns `Err("No picture from the camera in time.")`.
8. **`still`:** a cache at most `still_max_age` old is returned at once. Otherwise it behaves like `capture(...).ok()`.
9. **Threading:** every spawn uses `tauri::async_runtime::spawn` or `spawn_blocking`, since commands may be sync. No `std::sync::Mutex` is held across an `.await`. Use `tokio::sync::watch` channels for the state and the still cache, so waiters can `select!` on both.

- [ ] **Step 1: Add `TargetSource` to `target.rs`** (the interface above), with a doc comment.

- [ ] **Step 2: Write the test doubles and failing tests** at the bottom of `session.rs`. The doubles and the first tests are below; write every test named after them in the same style.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::testcam::tiny_jpeg;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fast() -> Timing {
        Timing {
            linger: Duration::from_millis(150),
            recheck: Duration::from_millis(50),
            backoff_start: Duration::from_millis(20),
            backoff_max: Duration::from_millis(80),
            capture_timeout: Duration::from_millis(800),
            still_max_age: Duration::from_millis(300),
            decode_every: Duration::from_millis(0),
            state_every: Duration::from_millis(0),
        }
    }

    /// What one connector call does.
    #[derive(Clone)]
    enum Script {
        /// Sends these frames `every` apart, then holds the connection open.
        Frames(Vec<Frame>, Duration),
        /// Ends at once with this error.
        Fail(CameraError),
    }

    #[derive(Default)]
    struct FakeConnector {
        scripts: Mutex<Vec<Script>>,
        calls: AtomicUsize,
        /// Run futures still alive (dropped when the hub closes a session).
        alive: Arc<AtomicUsize>,
    }

    struct Alive(Arc<AtomicUsize>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl FakeConnector {
        fn with(scripts: Vec<Script>) -> Arc<Self> {
            Arc::new(FakeConnector { scripts: Mutex::new(scripts), ..Default::default() })
        }
    }

    impl Connector for FakeConnector {
        fn run(&self, _target: CameraTarget, tx: mpsc::Sender<Frame>) -> RunFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let script = {
                let mut s = self.scripts.lock().unwrap();
                if s.len() > 1 { s.remove(0) } else { s[0].clone() }
            };
            self.alive.fetch_add(1, Ordering::SeqCst);
            let alive = Alive(self.alive.clone());
            Box::pin(async move {
                let _alive = alive;
                match script {
                    Script::Fail(e) => e,
                    Script::Frames(frames, every) => {
                        for f in frames {
                            if tx.send(f).await.is_err() {
                                return CameraError::Other { message: "stopped".into() };
                            }
                            tokio::time::sleep(every).await;
                        }
                        std::future::pending::<CameraError>().await
                    }
                }
            })
        }
    }

    struct FakeSource {
        answers: Mutex<Vec<Result<(), OffReason>>>,
        valid: Mutex<Result<(), OffReason>>,
        target: CameraTarget,
    }

    impl FakeSource {
        fn ok(target: CameraTarget) -> Arc<Self> {
            Arc::new(FakeSource { answers: Mutex::new(vec![Ok(())]), valid: Mutex::new(Ok(())), target })
        }
    }

    impl TargetSource for FakeSource {
        fn resolve(&self) -> Result<CameraTarget, OffReason> {
            let mut a = self.answers.lock().unwrap();
            let next = if a.len() > 1 { a.remove(0) } else { a[0] };
            next.map(|()| self.target.clone())
        }
        fn still_valid(&self, _t: &CameraTarget) -> Result<(), OffReason> {
            *self.valid.lock().unwrap()
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<CameraState>>);
    impl CameraEvents for Events {
        fn state(&self, s: &CameraState) {
            self.0.lock().unwrap().push(s.clone());
        }
    }

    /// A target that is never dialled (the connector is fake).
    fn target() -> CameraTarget {
        crate::camera::testcam::offline_target(Transport::Jpeg)
    }

    async fn until(mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() { return; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition not met in 2 s");
    }

    fn sink(log: Arc<Mutex<Vec<Frame>>>) -> Box<dyn FnMut(&Frame) -> bool + Send> {
        Box::new(move |f| {
            log.lock().unwrap().push(f.clone());
            true
        })
    }

    fn jpeg_frames(n: usize) -> Vec<Frame> {
        (0..n).map(|_| Frame::Jpeg(tiny_jpeg().into())).collect()
    }

    #[tokio::test]
    async fn one_session_is_shared_by_every_viewer() {
        let conn = FakeConnector::with(vec![Script::Frames(jpeg_frames(50), Duration::from_millis(10))]);
        let hub = CameraHub::new(FakeSource::ok(target()), conn.clone(), Arc::new(Events::default()), fast());
        let (a, b) = (Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![])));
        hub.watch(sink(a.clone()));
        hub.watch(sink(b.clone()));
        until(|| a.lock().unwrap().len() >= 3 && b.lock().unwrap().len() >= 3).await;
        assert_eq!(conn.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(hub.state().status, CameraStatus::Streaming));
    }

    #[tokio::test]
    async fn a_viewer_gets_the_config_then_video_from_a_keyframe() {
        let config = Frame::Config { codec: "avc1.640028".into(), avcc: vec![1].into() };
        let delta = Frame::Video { key: false, data: vec![1].into() };
        let key = Frame::Video { key: true, data: vec![2].into() };
        let conn = FakeConnector::with(vec![Script::Frames(
            vec![config.clone(), delta.clone(), key.clone(), delta.clone()],
            Duration::from_millis(5),
        )]);
        // decode_every 0 would decode `key`; the bytes aren't H.264, so the
        // decode fails and is skipped: the frames still flow.
        let hub = CameraHub::new(FakeSource::ok(target()), conn, Arc::new(Events::default()), fast());
        let log = Arc::new(Mutex::new(vec![]));
        hub.watch(sink(log.clone()));
        until(|| log.lock().unwrap().len() >= 3).await;
        assert_eq!(*log.lock().unwrap(), vec![config, key, delta]);
    }

    #[tokio::test]
    async fn the_session_closes_after_the_linger_and_not_before() {
        let conn = FakeConnector::with(vec![Script::Frames(jpeg_frames(1), Duration::from_millis(1))]);
        let hub = CameraHub::new(FakeSource::ok(target()), conn.clone(), Arc::new(Events::default()), fast());
        let id = hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| conn.alive.load(Ordering::SeqCst) == 1).await;
        hub.unwatch(id);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(conn.alive.load(Ordering::SeqCst), 1, "still within the linger");
        until(|| conn.alive.load(Ordering::SeqCst) == 0).await;
        assert_eq!(hub.state().status, CameraStatus::Idle);
    }

    #[tokio::test]
    async fn capture_returns_a_picture_taken_after_it_was_asked() {
        let conn = FakeConnector::with(vec![Script::Frames(jpeg_frames(100), Duration::from_millis(20))]);
        let hub = CameraHub::new(FakeSource::ok(target()), conn, Arc::new(Events::default()), fast());
        let jpeg = hub.capture().await.unwrap();
        assert_eq!(jpeg, tiny_jpeg());
    }

    #[tokio::test]
    async fn capture_decodes_the_next_keyframe() {
        let (avcc, sample) = crate::camera::h264::test_stream(64, 48);
        let conn = FakeConnector::with(vec![Script::Frames(
            vec![
                Frame::Config { codec: "avc1.42c01e".into(), avcc: avcc.into() },
                Frame::Video { key: true, data: sample.into() },
            ],
            Duration::from_millis(5),
        )]);
        let hub = CameraHub::new(FakeSource::ok(target()), conn, Arc::new(Events::default()), fast());
        let jpeg = hub.capture().await.unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
    }
}
```

Add this to `testcam.rs` for `target()`:

```rust
/// A target nothing listens at: for tests whose connector is fake.
pub fn offline_target(transport: Transport) -> CameraTarget {
    let ca = TestCa::new("Offline CA");
    let verifier =
        PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, RejectionSlot::default())
            .unwrap();
    CameraTarget {
        host: "127.0.0.1".into(),
        port: 9,
        serial: SERIAL.into(),
        access_code: CODE.into(),
        tls: client_config(Arc::new(verifier)).unwrap(),
        transport,
        model: "P1S".into(),
    }
}
```

Write these as well, each with a one-line comment saying which behaviour number it covers:

| Test | Covers |
|---|---|
| `a_viewer_back_within_the_linger_keeps_the_session` | 6 |
| `a_stopping_error_waits_for_retry` | 3: `Fail(Auth)`; after 200 ms `calls == 1` and the status is `Error { error: Auth, .. }`; `retry()` then `calls == 2` |
| `other_errors_reconnect_with_backoff` | 3: `Fail(Other)` then `Frames`; `calls == 2` and `Streaming` within 2 s |
| `an_off_printer_is_reported_and_checked_again` | 2: source answers `[Err(LiveviewOff), Ok(())]`; the events include `Off { reason: LiveviewOff, .. }`, then `Streaming` |
| `a_printer_that_stops_fitting_ends_the_session` | 2: streaming, then set `valid` to `Err(NotConnected)`; the run future is dropped and the status becomes `Off { NotConnected }` |
| `capture_fails_with_the_reason_when_there_is_no_camera` | 7: source `Err(NoCamera)` gives `Err("This printer has no camera.")` |
| `capture_gives_up_after_its_timeout` | 7: `Frames(vec![], _)` gives `Err("No picture from the camera in time.")` |
| `capture_tries_a_stopped_session_once_more` | 7: scripts `[Fail(Busy), Frames(jpegs)]`; `watch`, wait for `Error`, then `capture()` is `Ok` |
| `a_fresh_still_is_served_without_waiting` | 8: after the first frame, `still()` returns within 50 ms |
| `a_sink_that_fails_is_dropped` | 5: a sink returning `false` is called once; with no other consumer the session lingers out |
| `the_state_payload_is_tagged` | `serde_json::to_value(CameraState::off(OffReason::NoCamera))` is `{"transport":null,"status":{"kind":"off","reason":"no_camera","message":"This printer has no camera."},"last_frame_at":null}` |

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::session`
Expected: compile errors, because `CameraHub` doesn't exist yet.

- [ ] **Step 4: Implement `session.rs`** to the behaviour list and the interface above. Suggested internals, which the implementer may adjust if the tests and the interface hold:

```rust
struct Inner {
    source: Arc<dyn TargetSource>,
    connector: Arc<dyn Connector>,
    events: Arc<dyn CameraEvents>,
    timing: Timing,
    shared: Mutex<Shared>,
    state_tx: watch::Sender<CameraState>,
    still_tx: watch::Sender<Option<Still>>,
    retry: tokio::sync::Notify,
}

struct Shared {
    sinks: HashMap<u64, Sink>,
    next_id: u64,
    waiters: usize,
    config: Option<(String, Arc<[u8]>)>,
    driver: Option<tauri::async_runtime::JoinHandle<()>>,
    /// Bumped whenever the consumer count reaches zero; a linger timer
    /// only closes the session if it is unchanged when it fires.
    idle_epoch: u64,
    last_state_emit: Option<Instant>,
    last_decode: Option<Instant>,
}

struct Sink {
    send: Box<dyn FnMut(&Frame) -> bool + Send>,
    /// The cached config has been sent and a keyframe since.
    keyed: bool,
    has_config: bool,
}

#[derive(Clone)]
struct Still {
    at: Instant,
    jpeg: Arc<[u8]>,
}
```

- `LanConnector::run` is `Box::pin(async move { match target.transport { Transport::Jpeg => jpeg6000::run(&target, tx).await, Transport::Rtsps => rtsps::run(&target, tx).await } })`.
- `TauriCameraEvents::state` emits `STATE_EVENT` with the state, via `tauri::Emitter`.
- Send each `CameraEvents::state` call after releasing the `shared` lock.

- [ ] **Step 5: Run the tests until they pass, then clippy**

Run: `cargo test --manifest-path src-tauri/Cargo.toml camera::`, then the clippy command.
Expected: PASS. Run the session tests three times in a row to check for flakiness: `for i in 1 2 3; do cargo test --manifest-path src-tauri/Cargo.toml camera::session -q || break; done`.

- [ ] **Step 6: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/camera/mod.rs src-tauri/src/camera/target.rs src-tauri/src/camera/session.rs src-tauri/src/camera/testcam.rs
git add src-tauri/src/camera/mod.rs src-tauri/src/camera/target.rs src-tauri/src/camera/session.rs src-tauri/src/camera/testcam.rs
git commit -m "feat(camera): share one camera session per printer, with stills and captures"
```

---

### Task 6: Camera commands and app wiring

**Files:**
- Modify: `src-tauri/src/printer/service.rs`: add `CameraFacts` and `PrinterService::camera_facts()`.
- Modify: `src-tauri/src/camera/target.rs`: add `resolve_from`, `ServiceTargets` and `camera_pref`.
- Create: `src-tauri/src/commands/camera.rs`.
- Modify: `src-tauri/src/commands/mod.rs` (`pub mod camera;`), and `src-tauri/src/lib.rs` (the five commands in `generate_handler!`, plus creating and managing the hub in `setup`).
- Create, via the test: `src/camera/wire.json`.

**Interfaces:**
- Consumes: Task 5's hub API; `printer::service::client_params`; `printer::settings::{PrinterConfig, get_access_code}`; `printer::client::ConnectionState`; `transport::choose`.
- Produces:
  - `printer::service::CameraFacts { config: Option<PrinterConfig>, connected: bool, has_state: bool, model: String, ipcam: Option<IpCam> }`.
  - `camera::target::{resolve_from, ServiceTargets, camera_pref}`.
  - The commands `camera_state`, `camera_watch(onFrame)`, `camera_unwatch(id)`, `camera_still`, `camera_retry`. Tauri arguments are camelCase from JS: `onFrame` and `id`.
  - `src/camera/wire.json`, which Task 8's frontend test reads.

- [ ] **Step 1: Write the failing `resolve_from` tests** in `camera/target.rs`:

```rust
    use crate::printer::settings::PrinterConfig;
    use crate::printer::state::{IpCam, LanLiveview};
    use crate::printer::service::CameraFacts;
    use crate::camera::transport::{OffReason, Transport};

    fn config() -> PrinterConfig {
        PrinterConfig {
            ip: "192.168.1.50".into(),
            serial: "0938AC123456789".into(),
            name: "Shop".into(),
            model: "H2D".into(),
            pinned_fingerprint: None,
            ca_verified: false,
        }
    }

    fn facts() -> CameraFacts {
        CameraFacts {
            config: Some(config()),
            connected: true,
            has_state: true,
            model: "H2D".into(),
            ipcam: Some(IpCam { present: Some(true), liveview: LanLiveview::Rtsp }),
        }
    }

    #[test]
    fn a_connected_printer_with_a_code_gives_its_camera() {
        let t = resolve_from(facts(), Some("12345678".into())).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.transport), ("192.168.1.50", 322, Transport::Rtsps));
        assert_eq!(t.serial, "0938AC123456789");
    }

    #[test]
    fn each_missing_piece_says_why() {
        let none = CameraFacts { config: None, ..facts() };
        assert_eq!(resolve_from(none, Some("1".into())).err(), Some(OffReason::NotConfigured));
        let offline = CameraFacts { connected: false, ..facts() };
        assert_eq!(resolve_from(offline, Some("1".into())).err(), Some(OffReason::NotConnected));
        let fresh = CameraFacts { has_state: false, ..facts() };
        assert_eq!(resolve_from(fresh, Some("1".into())).err(), Some(OffReason::Waiting));
        assert_eq!(resolve_from(facts(), None).err(), Some(OffReason::NotConfigured));
        let off = CameraFacts {
            ipcam: Some(IpCam { present: Some(true), liveview: LanLiveview::Off }),
            ..facts()
        };
        assert_eq!(resolve_from(off, Some("1".into())).err(), Some(OffReason::LiveviewOff));
    }
```

Match the `PrinterConfig` fields to `printer/settings.rs`, adding `..` defaults if it derives `Default`.

- [ ] **Step 2: Implement**

In `printer/service.rs`:

```rust
/// What the camera needs to know about the printer, read in one lock.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraFacts {
    pub config: Option<PrinterConfig>,
    pub connected: bool,
    /// A report has arrived since connecting.
    pub has_state: bool,
    /// From `get_version` when connected, else from settings.
    pub model: String,
    pub ipcam: Option<crate::printer::state::IpCam>,
}

impl PrinterService {
    pub fn camera_facts(&self) -> CameraFacts {
        let live = self.inner.live.lock().unwrap();
        CameraFacts {
            config: live.config.clone(),
            connected: live.connection == ConnectionState::Connected,
            has_state: live.state.is_some(),
            model: live
                .model
                .clone()
                .or_else(|| live.config.as_ref().map(|c| c.model.clone()))
                .unwrap_or_default()
                .trim()
                .to_string(),
            ipcam: live.state.as_ref().and_then(|s| s.ipcam.clone()),
        }
    }
}
```

In `camera/target.rs`:

```rust
/// The camera for `facts` with the keychain's `code`, or why there is none.
pub fn resolve_from(facts: CameraFacts, code: Option<String>) -> Result<CameraTarget, OffReason> {
    let config = facts.config.ok_or(OffReason::NotConfigured)?;
    if !facts.connected {
        return Err(OffReason::NotConnected);
    }
    if !facts.has_state {
        return Err(OffReason::Waiting);
    }
    let transport = choose(&facts.model, facts.ipcam.as_ref())?;
    let code = code.ok_or(OffReason::NotConfigured)?;
    let params = client_params(&config, code).map_err(|_| OffReason::NotConfigured)?;
    Ok(CameraTarget {
        host: params.host,
        port: transport.port(),
        serial: params.serial,
        access_code: params.access_code,
        tls: params.tls,
        transport,
        model: facts.model,
    })
}

/// The app's printer: the service's live state and the keychain's code.
pub struct ServiceTargets {
    service: PrinterService,
}

impl ServiceTargets {
    pub fn new(service: PrinterService) -> Self {
        Self { service }
    }
}

impl TargetSource for ServiceTargets {
    fn resolve(&self) -> Result<CameraTarget, OffReason> {
        let facts = self.service.camera_facts();
        let code = match facts.config.as_ref() {
            Some(c) => get_access_code(&c.serial).ok().flatten(),
            None => None,
        };
        resolve_from(facts, code)
    }

    fn still_valid(&self, target: &CameraTarget) -> Result<(), OffReason> {
        let facts = self.service.camera_facts();
        let config = facts.config.as_ref().ok_or(OffReason::NotConfigured)?;
        if config.ip != target.host || !config.serial.eq_ignore_ascii_case(&target.serial) {
            return Err(OffReason::NotConfigured);
        }
        if !facts.connected {
            return Err(OffReason::NotConnected);
        }
        match choose(&facts.model, facts.ipcam.as_ref()) {
            Ok(t) if t == target.transport => Ok(()),
            Ok(_) => Err(OffReason::Waiting),
            Err(reason) => Err(reason),
        }
    }
}

/// A camera preference: on unless stored as `"false"`.
pub fn camera_pref(app: &tauri::AppHandle, key: &str) -> bool {
    use tauri_plugin_store::StoreExt;
    app.store("preferences.json")
        .ok()
        .and_then(|s| s.get(key))
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true)
}
```

**Restarts:** `still_valid` returns `Waiting` when the transport changed, so the hub's driver resolves again and reconnects. A code change made in Settings restarts the MQTT client. The camera picks it up on its next resolve: after an error, or after the session lingers out.

In `commands/camera.rs`:

```rust
//! The camera commands. Live view sends frames over a Channel as raw bytes
//! (see `camera::wire`); stills come back as `data:` URLs.

use base64::Engine;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State};

use crate::camera::target::camera_pref;
use crate::camera::transport::OffReason;
use crate::camera::{wire, CameraHub, CameraState, LIVE_VIEW_PREF};

#[tauri::command]
pub async fn camera_state(app: AppHandle, hub: State<'_, CameraHub>) -> Result<CameraState, String> {
    if !camera_pref(&app, LIVE_VIEW_PREF) {
        return Ok(CameraState::off(OffReason::Disabled));
    }
    Ok(hub.probe().await)
}

#[tauri::command]
pub async fn camera_watch(
    app: AppHandle,
    hub: State<'_, CameraHub>,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<u64, String> {
    if !camera_pref(&app, LIVE_VIEW_PREF) {
        return Err(OffReason::Disabled.message().to_string());
    }
    Ok(hub.watch(Box::new(move |frame| {
        on_frame.send(InvokeResponseBody::Raw(wire::encode(frame))).is_ok()
    })))
}

#[tauri::command]
pub async fn camera_unwatch(hub: State<'_, CameraHub>, id: u64) -> Result<(), String> {
    hub.unwatch(id);
    Ok(())
}

#[tauri::command]
pub async fn camera_still(app: AppHandle, hub: State<'_, CameraHub>) -> Result<Option<String>, String> {
    if !camera_pref(&app, LIVE_VIEW_PREF) {
        return Ok(None);
    }
    Ok(hub.still().await.map(|jpeg| {
        format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        )
    }))
}

#[tauri::command]
pub async fn camera_retry(hub: State<'_, CameraHub>) -> Result<CameraState, String> {
    Ok(hub.retry())
}
```

**Channel closing:** `Channel::send` returns `Err` once the webview has dropped the channel, so a closed page releases its consumer even without `camera_unwatch`. If `send` keeps returning `Ok` after the webview drops the channel, `camera_unwatch` from the frontend's cleanup is the release path; Task 8 always calls it.

In `lib.rs`:
- Add the five commands after `commands::journal::journal_image,`.
- In `setup`, right after `let service = printer::service::PrinterService::new(...)` and before `app.manage(service)`, add:

```rust
                // The printer's camera: one shared session, opened on demand.
                let camera = camera::CameraHub::new(
                    Arc::new(camera::target::ServiceTargets::new(service.clone())),
                    Arc::new(camera::session::LanConnector),
                    Arc::new(camera::session::TauriCameraEvents(app.handle().clone())),
                    camera::session::Timing::default(),
                );
                app.manage(camera.clone());
```

Keep the `camera` binding: Task 7 uses it for journal snapshots.

- [ ] **Step 3: Write the wire fixture test** in `commands/camera.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use crate::camera::session::{CameraState, CameraStatus};
    use crate::camera::transport::{OffReason, Transport};
    use crate::camera::CameraError;

    // The frontend's mirrors (`src/camera/types.rs`) are tested against this
    // file. Run with `UPDATE_WIRE_FIXTURE=1` to rewrite it after a
    // deliberate wire change.
    #[test]
    fn the_wire_fixture_holds_what_the_real_types_serialize() {
        let state = |transport, status| CameraState { transport, status, last_frame_at: Some(1_761_600_000) };
        let reasons = [
            OffReason::NotConfigured, OffReason::NotConnected, OffReason::Waiting,
            OffReason::NoCamera, OffReason::LiveviewOff, OffReason::Unsupported, OffReason::Disabled,
        ];
        let errors = [
            CameraError::Auth, CameraError::Tls, CameraError::Refused, CameraError::Busy,
            CameraError::Other { message: "The camera stream ended.".into() },
        ];
        let fixture = json!({
            "idle": state(None, CameraStatus::Idle),
            "connecting": state(Some(Transport::Rtsps), CameraStatus::Connecting),
            "streaming_rtsps": state(Some(Transport::Rtsps), CameraStatus::Streaming),
            "streaming_jpeg": state(Some(Transport::Jpeg), CameraStatus::Streaming),
            "off": reasons.iter().map(|r| CameraState::off(*r)).collect::<Vec<_>>(),
            "error": errors.iter().map(|e| state(Some(Transport::Jpeg), CameraStatus::Error {
                error: e.clone(), message: e.message(),
            })).collect::<Vec<_>>(),
        });
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/camera/wire.json");
        if std::env::var_os("UPDATE_WIRE_FIXTURE").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
        }
        let on_disk: Value = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("src/camera/wire.json is missing"),
        )
        .unwrap();
        assert_eq!(on_disk, fixture, "the wire changed: rerun with UPDATE_WIRE_FIXTURE=1 and update the frontend mirrors");
    }
}
```

- [ ] **Step 4: Generate the fixture and run everything**

Run:
- `UPDATE_WIRE_FIXTURE=1 cargo test --manifest-path src-tauri/Cargo.toml commands::camera`
- `cargo test --manifest-path src-tauri/Cargo.toml camera`
- the full `cargo test --manifest-path src-tauri/Cargo.toml`
- the clippy command
- `cargo check --target wasm32-unknown-unknown`, which checks that the frontend is unaffected.

Expected: all PASS, and `src/camera/wire.json` exists.

- [ ] **Step 5: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/printer/service.rs src-tauri/src/camera/target.rs src-tauri/src/commands/camera.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git add src-tauri/src/printer/service.rs src-tauri/src/camera/target.rs src-tauri/src/commands/camera.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src/camera/wire.json
git commit -m "feat(camera): camera commands, the printer as the camera's source, and app wiring"
```

---

### Task 7: Journal snapshots at key moments

**Files:**
- Modify: `src-tauri/src/journal/watch.rs`: `JournalEvent::Paused { key }`, emitted by `active()`.
- Create: `src-tauri/src/journal/moments.rs`; add `pub mod moments;` to `journal/mod.rs`.
- Modify: `src-tauri/src/journal/types.rs`: `Photo.label: Option<String>`, `PrintEntry.snapshot_note: Option<String>`.
- Modify: `src-tauri/src/journal/store.rs`: schema v3, `claim_snapshot`, `add_snapshot`, `note_snapshot_miss`, and reading `label` and `snapshot_note`.
- Modify: `src-tauri/src/journal/recorder.rs`: the `Snapshots` trait, `Recorder::use_snapshots`, capture threads, and `Paused` in `apply` and `forget`.
- Modify: `src-tauri/src/camera/target.rs`: `HubSnapshots`.
- Modify: `src-tauri/src/lib.rs`: `recorder.use_snapshots(...)` before `service.attach_journal(...)`.
- Modify: any exhaustive `match` on `JournalEvent` elsewhere (search with `grep -rn "JournalEvent::Lost" src-tauri/src`).
- Regenerate: `src/calibration/wire.json`, since the calibration fixture serializes a journal `Photo` and `PrintEntry`. Run `UPDATE_WIRE_FIXTURE=1 cargo test --manifest-path src-tauri/Cargo.toml the_wire_fixture`. Then check that `src/calibration/types.rs`'s mirrors still deserialize, running `cargo test --bin bambumate calibration`. Add `#[serde(default)]` fields there only if a mirror needs them.

**Interfaces:**
- Consumes: `CameraHub::capture` (Task 5); `camera_pref` and `SNAPSHOTS_PREF` (Task 6); `journal::photos::prepare_photo(&[u8]) -> Result<Vec<u8>, &'static str>`; `journal::types::Outcome::{Finished, Failed}`.
- Produces:

```rust
// journal/moments.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Moment { FirstLayer, Half, Finished, Failed, Paused }
impl Moment {
    pub fn key(self) -> &'static str;     // "first_layer" | "half" | "finished" | "failed" | "paused"
    pub fn label(self) -> &'static str;   // "FIRST LAYER" | "50 %" | "FINISHED" | "FAILED" | "PAUSED"
}
/// The moments `event` reaches, with the job key they belong to.
pub fn due(event: &JournalEvent) -> Vec<(&str, Moment)>;

// journal/recorder.rs
pub trait Snapshots: Send + Sync + 'static {
    /// Whether the user wants journal snapshots now.
    fn enabled(&self) -> bool;
    /// One JPEG from the printer's camera; may block up to ~15 s. `Err` is
    /// a sentence for the entry's note.
    fn capture(&self) -> Result<Vec<u8>, String>;
}
impl Recorder { pub fn use_snapshots(&self, snapshots: Arc<dyn Snapshots>); }

// journal/store.rs
impl JournalStore {
    /// Records that `moment` (a `Moment::key`) is being taken for entry
    /// `id`: false if it was already, or the entry has a calibration
    /// purpose. `GONE` if the entry is gone.
    pub fn claim_snapshot(&self, id: i64, moment: &str) -> Result<bool, String>;
    /// `add_photo` with a label.
    pub fn add_snapshot(&self, id: i64, jpeg: &[u8], label: &str, now: i64) -> Result<Photo, String>;
    /// The reason the entry's last snapshot wasn't taken.
    pub fn note_snapshot_miss(&self, id: i64, reason: &str, now: i64) -> Result<(), String>;
}

// camera/target.rs
pub struct HubSnapshots { hub: CameraHub, app: tauri::AppHandle }
impl HubSnapshots { pub fn new(hub: CameraHub, app: tauri::AppHandle) -> Self; }
impl crate::journal::recorder::Snapshots for HubSnapshots { /* enabled: camera_pref(SNAPSHOTS_PREF); capture: tauri::async_runtime::block_on(hub.capture()) */ }
```

- [ ] **Step 1: Write the failing watcher test.** Copy the shape of the nearest existing test in `journal/watch.rs` that drives `observe` through RUNNING into PAUSE, and use its report helpers:

```rust
    #[test]
    fn a_pause_with_an_error_is_reported_and_a_plain_pause_is_not() {
        // Drive a job to RUNNING (confirmed), then:
        // 1. a PAUSE report with print_error None → no Paused event;
        // 2. a PAUSE report with print_error Some(0x0300_8001) → exactly one
        //    JournalEvent::Paused { key } with the job's key;
        // 3. the same PAUSE again → Paused again (the recorder dedupes).
    }
```

Fill in the body with the module's own helpers (for example a `report(state, percent, ...)` builder and `observe` calls). The comments above are the assertions. The test must not be left as comments.

- [ ] **Step 2: Implement `Paused`**
- Add the variant to `JournalEvent`, after `TrayUsed`:

```rust
    /// The confirmed job is paused with `print_error` set: a moment the
    /// journal may photograph. Sent with every such report; the recorder
    /// takes it once.
    Paused { key: String },
```

- In `active()`, in the branch where the same job continues (`if same && !restarted(...)`), compute the next job with `follow(...)` as now. Then, before returning, push the event when `state == "PAUSE" && report.print_error.is_some()`: `events.push(JournalEvent::Paused { key: job_key_after_follow.clone() })`.
- In `recorder.rs`, `forget` and `apply` treat `Paused` as a no-op: `apply` returns `Ok(false)`.
- Run `cargo test --manifest-path src-tauri/Cargo.toml journal::watch`. Expected: PASS.

- [ ] **Step 3: Write `journal/moments.rs` with its tests**

```rust
//! The moments a print is photographed at, read off the watcher's events.
//! A moment whose window was missed (the app joined at 70 %) is skipped,
//! not taken late. Taking each once is the store's job (`claim_snapshot`).

use super::types::Outcome;
use super::watch::JournalEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Moment {
    FirstLayer,
    Half,
    Finished,
    Failed,
    Paused,
}

impl Moment {
    pub fn key(self) -> &'static str {
        match self {
            Moment::FirstLayer => "first_layer",
            Moment::Half => "half",
            Moment::Finished => "finished",
            Moment::Failed => "failed",
            Moment::Paused => "paused",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Moment::FirstLayer => "FIRST LAYER",
            Moment::Half => "50 %",
            Moment::Finished => "FINISHED",
            Moment::Failed => "FAILED",
            Moment::Paused => "PAUSED",
        }
    }
}

pub fn due(event: &JournalEvent) -> Vec<(&str, Moment)> {
    match event {
        JournalEvent::Progress { key, percent, layer } => {
            let mut out = Vec::new();
            if matches!(layer, Some(2..=3)) {
                out.push((key.as_str(), Moment::FirstLayer));
            }
            if (50..=55).contains(percent) {
                out.push((key.as_str(), Moment::Half));
            }
            out
        }
        JournalEvent::Ended { key, outcome: Outcome::Finished, .. } => vec![(key.as_str(), Moment::Finished)],
        JournalEvent::Ended { key, outcome: Outcome::Failed, .. } => vec![(key.as_str(), Moment::Failed)],
        JournalEvent::Paused { key } => vec![(key.as_str(), Moment::Paused)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(percent: u32, layer: Option<u32>) -> JournalEvent {
        JournalEvent::Progress { key: "k".into(), percent, layer }
    }

    #[test]
    fn the_first_layer_and_half_way_have_windows() {
        assert_eq!(due(&progress(1, Some(1))), vec![]);
        assert_eq!(due(&progress(2, Some(2))), vec![("k", Moment::FirstLayer)]);
        assert_eq!(due(&progress(3, Some(3))), vec![("k", Moment::FirstLayer)]);
        assert_eq!(due(&progress(4, Some(4))), vec![]);
        assert_eq!(due(&progress(4, None)), vec![]);
        assert_eq!(due(&progress(49, Some(80))), vec![]);
        assert_eq!(due(&progress(50, Some(80))), vec![("k", Moment::Half)]);
        assert_eq!(due(&progress(55, Some(90))), vec![("k", Moment::Half)]);
        assert_eq!(due(&progress(56, Some(91))), vec![]);
        assert_eq!(
            due(&progress(52, Some(2))),
            vec![("k", Moment::FirstLayer), ("k", Moment::Half)]
        );
    }

    #[test]
    fn ends_and_error_pauses_are_moments() {
        let ended = |outcome| JournalEvent::Ended {
            key: "k".into(), outcome, ended_at: 0, error_code: None, hms: vec![],
        };
        assert_eq!(due(&ended(Outcome::Finished)), vec![("k", Moment::Finished)]);
        assert_eq!(due(&ended(Outcome::Failed)), vec![("k", Moment::Failed)]);
        assert_eq!(due(&ended(Outcome::Unknown)), vec![]);
        assert_eq!(due(&JournalEvent::Paused { key: "k".into() }), vec![("k", Moment::Paused)]);
        assert_eq!(due(&JournalEvent::Lost { key: "k".into() }), vec![]);
    }

    #[test]
    fn labels_are_the_page_text() {
        let all = [Moment::FirstLayer, Moment::Half, Moment::Finished, Moment::Failed, Moment::Paused];
        let labels: Vec<_> = all.iter().map(|m| m.label()).collect();
        assert_eq!(labels, ["FIRST LAYER", "50 %", "FINISHED", "FAILED", "PAUSED"]);
    }
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::moments`. Expected: PASS.

- [ ] **Step 4: Write the failing store tests** in `journal/store.rs`. Use the module's existing helpers (`open`, `new_print`, or whatever its tests use to create an entry):
- `snapshots_are_claimed_once_per_moment`: `claim_snapshot(id, "half")` gives true, then false; `"finished"` gives true.
- `a_calibration_print_is_photographed_like_any_other`: after the existing purpose-setting call, `claim_snapshot` still gives true (user ruling, 2026-10-27).
- `a_snapshot_keeps_its_label_and_a_miss_its_reason`: after `add_snapshot(id, &jpeg, "50 %", now)` and `note_snapshot_miss(id, "This printer has no camera.", now)`, `get(id)` shows `photos[0].label == Some("50 %")` and `snapshot_note == Some("This printer has no camera.")`. A plain `add_photo` photo has `label == None`.
- `claim_snapshot` on a deleted entry gives `Err(GONE)`.
- `a_v2_journal_moves_to_v3_with_its_rows`: write a v2 database with the existing pattern for the v1 to v2 migration test (search `migrate_to_v2` tests), open it, and assert `user_version == 3`, both new columns present, and old rows readable with `label` and `snapshot_note` `None`. Also check that a v3 file opened again stays v3, and that a newer version is refused with `NEWER_SCHEMA` (the existing test likely covers this through `SCHEMA_VERSION + 1`, so it keeps passing).

- [ ] **Step 5: Implement the store changes**
- `SCHEMA_VERSION = 3`.
- Add `migrate_to_v3`, a copy of `migrate_to_v2`'s pattern that adds `print_photos.label TEXT` and `prints.snapshots_json TEXT`, each behind its own `pragma_table_info` column check. Call it after `migrate_to_v2` wherever `open` calls the migrations.
- Add `snapshots_json` to `COLUMNS`. In the row reader, parse it as:

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SnapshotLog {
    taken: Vec<String>,
    missed: Option<String>,
}
```

  Then set `PrintEntry.snapshot_note = log.missed`. Unreadable JSON is treated as empty.
- The photos query selects `label` too.
- `claim_snapshot` runs an immediate transaction: read `purpose_json` and `snapshots_json` (no row gives `GONE`). A purpose that is not null gives false. A key already in `taken` gives false. Otherwise push the key and write it back, giving true. It doesn't touch `updated_at`.
- `add_snapshot` shares a private `insert_photo(id, jpeg, label: Option<&str>, now)` with `add_photo`, which becomes `insert_photo(id, jpeg, None, now)`. Both return `Photo { id, file_name, added_at, label }`.
- `note_snapshot_miss` sets `missed` and calls `touch(id, now)`.
- In `journal/types.rs`, add `#[serde(default)] pub label: Option<String>` to `Photo` and `#[serde(default)] pub snapshot_note: Option<String>` to `PrintEntry`. Fix every struct literal.

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::`. Expected: PASS.

- [ ] **Step 6: Write the failing recorder tests** in `journal/recorder.rs`, using its existing harness (`start_with`, a temp `JournalPaths`, a `NoSlices`, and a recording `JournalNotify`):

```rust
    struct FakeShots {
        on: bool,
        answer: Result<Vec<u8>, String>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl Snapshots for FakeShots {
        fn enabled(&self) -> bool { self.on }
        fn capture(&self) -> Result<Vec<u8>, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.answer.clone()
        }
    }
```

| Test | What it checks |
|---|---|
| `half_way_is_photographed_once_with_its_label` | Start a print. Record `Progress { percent: 50 }` twice and `Progress { percent: 51 }`. Within 2 s, the entry has exactly one photo labelled `50 %`, and `calls == 1` |
| `the_end_is_photographed_after_the_entry_closes` | `Ended { Finished }` gives a `FINISHED` photo on the now-ended entry |
| `a_failed_capture_leaves_its_reason` | `answer: Err("This printer has no camera.")` gives no photo and that `snapshot_note` |
| `snapshots_off_take_and_note_nothing` | `on: false` gives `calls == 0`, no photo and no note |
| `an_unreadable_picture_is_a_miss_not_a_photo` | `answer: Ok(b"not a jpeg")` gives no photo; the note is the `prepare_photo` error sentence |

Poll the store in a loop: 10 ms sleeps, up to 2 s.

- [ ] **Step 7: Implement the recorder changes**
- `Recorder` gets `snapshots: Arc<Mutex<Option<Arc<dyn Snapshots>>>>`, shared with `Writer`. `use_snapshots` sets it. Both are created in `start_with`.
- `Writer` gets `claimed: HashSet<(i64, Moment)>`.
- In `Writer::write`, for each event, before `apply`, collect `due(event)` mapped through `self.keys.get(key)` to `(id, moment)`. Only entries open before this event count. A `Started` in the same batch already put its key in `keys`, and `Ended` removes it in `apply`, so read the id first.
- After the batch's applies, for each `(id, moment)`:
  1. Skip it if `claimed` has it, or there are no snapshots, or `!enabled()`.
  2. Otherwise insert it into `claimed`, then call `store.claim_snapshot(id, moment.key())`.
  3. On `Ok(true)`, spawn a thread named `journal-snapshot` that:
     - runs `let shot = snaps.capture()`;
     - prepares the photo with `prepare_photo(&jpeg).map_err(str::to_string)`;
     - opens its own `JournalStore::open(&paths)`;
     - calls `add_snapshot(id, &jpeg, moment.label(), now())`, or on `Err(reason)` `note_snapshot_miss(id, &reason, now())`;
     - calls `notify.changed()`.
  4. Any error inside the thread is logged at `warn` (without the JPEG) and dropped.
  5. `Ok(false)` and `Err(GONE)` are skipped quietly. Other errors are logged at `warn`.
- The writer clones `paths` and `notify` for the thread (both are `Clone`/`Arc`; make `JournalPaths` `Clone` if it isn't).

- [ ] **Step 8: Wire it up**
- `camera/target.rs`: implement `HubSnapshots` as in the interface.
- `lib.rs`: inside `if let Some(paths) = journal_paths { ... }`, after `Recorder::start(...)` and before `service.attach_journal(recorder, watch)`, add:

```rust
                    recorder.use_snapshots(Arc::new(camera::target::HubSnapshots::new(
                        camera.clone(),
                        app.handle().clone(),
                    )));
```

  This needs the `camera` hub created before this block; move Task 6's hub creation above it if needed.

- [ ] **Step 9: Run everything**

Run:
- `UPDATE_WIRE_FIXTURE=1 cargo test --manifest-path src-tauri/Cargo.toml the_wire_fixture`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- the clippy command
- `cargo test --bin bambumate`
- `cargo check --target wasm32-unknown-unknown`

Expected: all PASS.

- [ ] **Step 10: Format and commit**

```bash
rustfmt --edition 2021 src-tauri/src/journal/watch.rs src-tauri/src/journal/moments.rs src-tauri/src/journal/mod.rs src-tauri/src/journal/types.rs src-tauri/src/journal/store.rs src-tauri/src/journal/recorder.rs src-tauri/src/camera/target.rs src-tauri/src/lib.rs
git add src-tauri/src/journal/watch.rs src-tauri/src/journal/moments.rs src-tauri/src/journal/mod.rs src-tauri/src/journal/types.rs src-tauri/src/journal/store.rs src-tauri/src/journal/recorder.rs src-tauri/src/camera/target.rs src-tauri/src/lib.rs src/calibration/wire.json
git commit -m "feat(journal): photograph each print at its first layer, half way and end"
```

Also `git add` any other file changed for an exhaustive match, named individually.

---
### Task 8: Frontend camera module and the Printer → Live panel

**Files:**
- Create: `src/camera/mod.rs`, `src/camera/types.rs`, `src/camera/shim.rs`, `src/camera/bridge.rs`, `src/camera/view.rs`
- Create: `style/camera.css`; link it in `index.html` after `style/calibrate.css` with `<link data-trunk rel="css" href="style/camera.css" />`
- Modify: `src/main.rs` (`mod camera;`), `src/pages/printer.rs` (the panel beside the hero)
- Modify: `tests/webkit/fixtures.mjs` (camera fixtures), `tests/webkit/app-flows.mjs` (mock `Channel`, mock `VideoDecoder`, and flows)

**Interfaces:**
- Consumes:
  - The backend commands from Task 6, with JS arguments `onFrame` and `id`.
  - The event `camera://state`.
  - `src/camera/wire.json` from Task 6.
  - The channel byte layout from the Global Constraints.
  - `crate::agent::bridge::listen_scoped`.
  - `crate::printer::PrinterShared` (`view: RwSignal<PrinterView>`).
- Produces:
  - `camera::types::{CameraState, CameraStatus, OffReason, CameraErrorKind, Transport, Parsed, Kind, parse, Shown, shown, LIVEVIEW_STEPS}`.
  - `camera::bridge::{state, still, retry, unwatch, Watch}`.
  - `camera::view::{CameraPanel, CameraThumb}`. Task 9 places `CameraThumb`, which is written here.

- [ ] **Step 1: Write the failing `types.rs` tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("wire.json")).unwrap()
    }

    #[test]
    fn every_backend_state_deserializes() {
        let f = fixture();
        for key in ["idle", "connecting", "streaming_rtsps", "streaming_jpeg"] {
            let s: CameraState = serde_json::from_value(f[key].clone()).unwrap();
            assert_eq!(s.last_frame_at, Some(1_761_600_000), "{key}");
        }
        let offs: Vec<CameraState> = serde_json::from_value(f["off"].clone()).unwrap();
        assert_eq!(offs.len(), 7);
        assert!(offs.iter().all(|s| matches!(s.status, CameraStatus::Off { .. })));
        let errors: Vec<CameraState> = serde_json::from_value(f["error"].clone()).unwrap();
        assert_eq!(errors.len(), 5);
    }

    #[test]
    fn each_state_reads_as_the_spec_says() {
        let f = fixture();
        let offs: Vec<CameraState> = serde_json::from_value(f["off"].clone()).unwrap();
        let texts: Vec<Shown> = offs.iter().map(shown).collect();
        assert_eq!(texts[0], Shown::Note("No printer is set up.".into()));
        assert_eq!(texts[1], Shown::Note("The printer isn't connected.".into()));
        assert_eq!(texts[2], Shown::Note("Waiting for the printer's camera details…".into()));
        assert_eq!(texts[3], Shown::Note("This printer has no camera.".into()));
        assert_eq!(texts[4], Shown::LiveviewOff);
        assert_eq!(texts[5], Shown::Note("BambuMate can't show this printer's camera yet.".into()));
        assert_eq!(texts[6], Shown::Note("Live view is off in Settings.".into()));
        let errors: Vec<CameraState> = serde_json::from_value(f["error"].clone()).unwrap();
        assert_eq!(
            shown(&errors[3]),
            Shown::Error("The camera is in use by another app. Close the live view in Bambu Studio or Handy, then retry.".into())
        );
        let idle: CameraState = serde_json::from_value(f["idle"].clone()).unwrap();
        assert_eq!(shown(&idle), Shown::Connecting);
        let live: CameraState = serde_json::from_value(f["streaming_jpeg"].clone()).unwrap();
        assert_eq!(shown(&live), Shown::Live);
    }

    #[test]
    fn channel_messages_parse_like_the_backend_writes_them() {
        let mut config = vec![1, 0, 0, 11];
        config.extend_from_slice(b"avc1.640028");
        config.extend_from_slice(&[1, 2, 3]);
        assert_eq!(
            parse(&config),
            Some(Parsed { kind: Kind::Config, key: false, payload: 15, codec: Some("avc1.640028".into()) })
        );
        assert_eq!(parse(&[2, 1, 9, 8]), Some(Parsed { kind: Kind::Video, key: true, payload: 2, codec: None }));
        assert_eq!(parse(&[2, 0, 7]), Some(Parsed { kind: Kind::Video, key: false, payload: 2, codec: None }));
        assert_eq!(parse(&[3, 0, 0xFF, 0xD8]), Some(Parsed { kind: Kind::Jpeg, key: false, payload: 2, codec: None }));
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[9, 0, 1]), None);
        assert_eq!(parse(&[1, 0, 0, 200, b'a']), None, "codec longer than the message");
        assert_eq!(parse(&[2, 0]), None, "no payload");
    }
}
```

- [ ] **Step 2: Implement `types.rs`**

```rust
//! Mirrors of the backend's camera state (`camera::session`), the page's
//! reading of it, and the Channel message header. Tested against
//! `wire.json`, which the backend writes.

use serde::Deserialize;

/// Under the "Turn on LAN Mode Liveview" title.
pub const LIVEVIEW_STEPS: &str =
    "On the printer screen: Settings → General (or LAN Only) → LAN Mode Liveview.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Rtsps,
    Jpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffReason {
    NotConfigured,
    NotConnected,
    Waiting,
    NoCamera,
    LiveviewOff,
    Unsupported,
    Disabled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraErrorKind {
    Auth,
    Tls,
    Refused,
    Busy,
    Other { message: String },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraStatus {
    Idle,
    Connecting,
    Streaming,
    Off { reason: OffReason, message: String },
    Error { error: CameraErrorKind, message: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CameraState {
    pub transport: Option<Transport>,
    pub status: CameraStatus,
    pub last_frame_at: Option<i64>,
}

impl Default for CameraState {
    fn default() -> Self {
        CameraState { transport: None, status: CameraStatus::Idle, last_frame_at: None }
    }
}

/// What the panel shows for a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// `[CONNECTING…]`.
    Connecting,
    /// The picture.
    Live,
    /// The title and [`LIVEVIEW_STEPS`].
    LiveviewOff,
    /// A plain sentence.
    Note(String),
    /// `[ERROR: …]` with Retry.
    Error(String),
}

pub fn shown(s: &CameraState) -> Shown {
    match &s.status {
        CameraStatus::Idle | CameraStatus::Connecting => Shown::Connecting,
        CameraStatus::Streaming => Shown::Live,
        CameraStatus::Off { reason: OffReason::LiveviewOff, .. } => Shown::LiveviewOff,
        CameraStatus::Off { message, .. } => Shown::Note(message.clone()),
        CameraStatus::Error { message, .. } => Shown::Error(message.clone()),
    }
}

impl CameraState {
    /// No picture can come until something changes on the printer or in
    /// Settings: the panel stops watching.
    pub fn is_off(&self) -> bool {
        matches!(self.status, CameraStatus::Off { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Config,
    Video,
    Jpeg,
}

/// A Channel message's header: what it is, and where its payload starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub kind: Kind,
    pub key: bool,
    pub payload: usize,
    pub codec: Option<String>,
}

/// Longest message accepted (the backend's `MAX_MESSAGE` plus the header).
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024 + 300;

/// Reads the header of `b` (only its first bytes are needed). `None` for
/// anything unknown, cut short or too large.
pub fn parse(b: &[u8]) -> Option<Parsed> {
    let (&tag, rest) = b.split_first()?;
    let (&flag, _) = rest.split_first()?;
    match tag {
        1 => {
            let len = usize::from(u16::from_be_bytes([*b.get(2)?, *b.get(3)?]));
            let codec = std::str::from_utf8(b.get(4..4 + len)?).ok()?.to_string();
            if b.len() <= 4 + len || !codec.is_ascii() {
                return None;
            }
            Some(Parsed { kind: Kind::Config, key: false, payload: 4 + len, codec: Some(codec) })
        }
        2 | 3 if b.len() > 2 => Some(Parsed {
            kind: if tag == 2 { Kind::Video } else { Kind::Jpeg },
            key: tag == 2 && flag == 1,
            payload: 2,
            codec: None,
        }),
        _ => None,
    }
}
```

**Header copy:** in wasm, `parse` gets a copy of only the first `min(len, 4 + 256)` bytes; the payload stays in JS as a `subarray`. So `parse` must not require the whole message. The `b.len() <= 4 + len` check only works on the copied prefix when the copy is longer than the header; copy `min(len, 4 + 256 + 1)` bytes so a config with any payload passes. The message's total size is checked separately against `MAX_MESSAGE`.

Run: `cargo test --bin bambumate camera::types`. Expected: PASS.

- [ ] **Step 3: Write `shim.rs`**

```rust
//! The browser side of the camera: a Tauri Channel whose messages arrive
//! as bytes, WebCodecs H.264 decoding onto a canvas, JPEG object URLs, and
//! a visibility watch (on screen and window shown). Kept in JS because
//! these APIs are awkward through web-sys; everything they receive is
//! treated as untrusted bytes.

use js_sys::{Function, Promise, Uint8Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
function bytes(m) {
  if (m instanceof ArrayBuffer) return new Uint8Array(m);
  if (ArrayBuffer.isView(m)) return new Uint8Array(m.buffer, m.byteOffset, m.byteLength);
  if (Array.isArray(m)) return Uint8Array.from(m);
  return null;
}
export function cameraHasDecoder() {
  return typeof VideoDecoder === "function" && typeof EncodedVideoChunk === "function";
}
export async function cameraWatch(onBytes) {
  const tauri = window.__TAURI__ && window.__TAURI__.core;
  if (!tauri || typeof tauri.Channel !== "function") throw new Error("no channel");
  const channel = new tauri.Channel();
  channel.onmessage = (m) => { const b = bytes(m); if (b) onBytes(b); };
  const id = await tauri.invoke("camera_watch", { onFrame: channel });
  return { id, channel };
}
export class CameraDecoder {
  constructor(canvas, onError) {
    this.canvas = canvas; this.onError = onError; this.dec = null; this.ts = 0; this.needKey = true;
  }
  async configure(codec, description) {
    this.close();
    const config = { codec, description: description.slice(), optimizeForLatency: true };
    let ok = false;
    try { ok = !!(await VideoDecoder.isConfigSupported(config)).supported; } catch (_) { ok = false; }
    if (!ok) return false;
    this.dec = new VideoDecoder({
      output: (frame) => {
        try {
          if (this.canvas.width !== frame.displayWidth) this.canvas.width = frame.displayWidth;
          if (this.canvas.height !== frame.displayHeight) this.canvas.height = frame.displayHeight;
          this.canvas.getContext("2d").drawImage(frame, 0, 0);
        } finally { frame.close(); }
      },
      error: (e) => { this.close(); this.onError(String(e && e.message || e)); },
    });
    this.dec.configure(config);
    this.needKey = true;
    return true;
  }
  decode(data, key) {
    const dec = this.dec;
    if (!dec || dec.state !== "configured") return;
    if (this.needKey && !key) return;
    if (!key && dec.decodeQueueSize > 30) { this.needKey = true; return; }
    this.needKey = false;
    this.ts += 33333;
    try {
      dec.decode(new EncodedVideoChunk({ type: key ? "key" : "delta", timestamp: this.ts, data }));
    } catch (e) { this.close(); this.onError(String(e && e.message || e)); }
  }
  close() { const d = this.dec; this.dec = null; if (d && d.state !== "closed") { try { d.close(); } catch (_) {} } }
}
export function cameraJpegUrl(data) { return URL.createObjectURL(new Blob([data], { type: "image/jpeg" })); }
export function cameraRevoke(url) { try { URL.revokeObjectURL(url); } catch (_) {} }
export function cameraVisibility(el, onChange) {
  let onScreen = false;
  const report = () => onChange(onScreen && document.visibilityState === "visible");
  const io = new IntersectionObserver((entries) => {
    onScreen = entries.some((e) => e.isIntersecting); report();
  });
  io.observe(el);
  document.addEventListener("visibilitychange", report);
  return () => { io.disconnect(); document.removeEventListener("visibilitychange", report); };
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = cameraHasDecoder)]
    pub fn has_decoder() -> bool;
    /// Resolves to `{ id, channel }`; rejects when the backend refuses.
    #[wasm_bindgen(js_name = cameraWatch, catch)]
    pub fn watch(on_bytes: &Closure<dyn FnMut(Uint8Array)>) -> Result<Promise, JsValue>;

    pub type CameraDecoder;
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: &web_sys::HtmlCanvasElement, on_error: &Closure<dyn FnMut(String)>) -> CameraDecoder;
    /// Resolves to whether the browser can decode `codec`.
    #[wasm_bindgen(method)]
    pub fn configure(this: &CameraDecoder, codec: &str, description: &Uint8Array) -> Promise;
    #[wasm_bindgen(method)]
    pub fn decode(this: &CameraDecoder, data: &Uint8Array, key: bool);
    #[wasm_bindgen(method)]
    pub fn close(this: &CameraDecoder);

    #[wasm_bindgen(js_name = cameraJpegUrl)]
    pub fn jpeg_url(data: &Uint8Array) -> String;
    #[wasm_bindgen(js_name = cameraRevoke)]
    pub fn revoke(url: &str);
    /// Calls `on_change(visible)`; the returned function stops watching.
    #[wasm_bindgen(js_name = cameraVisibility)]
    pub fn visibility(el: &web_sys::Element, on_change: &Closure<dyn FnMut(bool)>) -> Function;
}
```

`Function` is used for the returned remover. If `inline_js` fails under Trunk, move the JS to `src/camera/shim.js` and use `#[wasm_bindgen(module = "/src/camera/shim.js")]`. Trunk copies wasm-bindgen snippets either way.

- [ ] **Step 4: Write `bridge.rs`**

```rust
//! Invoke wrappers for the `camera_*` commands, and the live watch.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::shim;
use super::types::CameraState;

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
struct IdArgs {
    id: u64,
}

pub async fn state() -> Result<CameraState, String> {
    call("camera_state", &Empty {}).await
}

/// A `data:` URL of the latest picture, `None` when there is none.
pub async fn still() -> Result<Option<String>, String> {
    call("camera_still", &Empty {}).await
}

pub async fn retry() -> Result<CameraState, String> {
    call("camera_retry", &Empty {}).await
}

pub async fn unwatch(id: u64) {
    let _ = call::<_, ()>("camera_unwatch", &IdArgs { id }).await;
}

/// A live watch: the backend's id and the closure receiving its messages
/// (kept alive as long as the watch).
pub struct Watch {
    pub id: u64,
    _on_bytes: Closure<dyn FnMut(js_sys::Uint8Array)>,
}

/// Starts watching; `on_bytes` gets each message.
pub async fn watch(on_bytes: impl FnMut(js_sys::Uint8Array) + 'static) -> Result<Watch, String> {
    let closure = Closure::<dyn FnMut(js_sys::Uint8Array)>::new(on_bytes);
    let promise = shim::watch(&closure).map_err(|_| "The camera couldn't be opened.".to_string())?;
    let answer = JsFuture::from(promise)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "The camera couldn't be opened.".into()))?;
    let id = js_sys::Reflect::get(&answer, &"id".into())
        .ok()
        .and_then(|v| v.as_f64())
        .ok_or("The camera couldn't be opened.")? as u64;
    Ok(Watch { id, _on_bytes: closure })
}
```

**Closure lifetime:** a JS message may arrive after the `Watch` is dropped. If a dropped `Closure` panics when called, keep the closure alive the way `listen_scoped` does (`closure.forget()` plus a "gone" flag checked inside), and document why.

- [ ] **Step 5: Write `view.rs`**, the `CameraPanel`. Requirements, each covered by a flow in Step 7:
1. **State:** on mount, `bridge::state()` fills `RwSignal<CameraState>`, and `listen_scoped("camera://state", …)` keeps it current, using `try_set`.
2. **Visibility:** `shim::visibility(root, …)` sets `visible: RwSignal<bool>`. The remover runs in `on_cleanup`.
3. **Mode** is derived from `(visible, state, has_decoder())`:
   - Not visible, or `state.is_off()`: nothing runs.
   - Transport `Jpeg`, or `Rtsps` with `has_decoder()`: a live watch (`bridge::watch`).
   - `Rtsps` without `has_decoder()`, or after a decoder error or an unsupported config: stills. Call `bridge::still()` at once and then every 2 s; show the `data:` URL in the `<img>`, with the note `STILL · HH:MM:SS` taken from `last_frame_at`, or the fetch time if it is absent.
   - Leaving a mode stops it: `bridge::unwatch(id)`, `decoder.close()`, revoke the object URL, and clear the interval.
4. **Messages:**
   - `parse` runs on a copy of the first `min(len, 261)` bytes, from `Uint8Array::subarray(0, n).to_vec()`. A message over `MAX_MESSAGE` is ignored.
   - `Config`: `decoder.configure(codec, payload)`. A `false` result switches to stills.
   - `Video`: `decoder.decode(payload_subarray, key)`.
   - `Jpeg`: `shim::jpeg_url(payload)`, set it as the `<img src>`, and revoke the previous URL.
5. **Markup:** an `<img>` for JPEG and stills, or a `<canvas>` for H.264, inside `<div class="cam-frame">`. The caption is `LIVE · {MODEL}` on the left and the last frame time (`HH:MM:SS`, local) on the right. The model comes from `PrinterShared.view` (`printer.model`, uppercased, through `untrusted_text` with a 32-character cap).
6. **Overlays** inside the frame, per `shown(&state)`: `Connecting` gives `[CONNECTING…]`, `LiveviewOff` gives the title plus `LIVEVIEW_STEPS`, `Note(s)` gives the sentence, and `Error(m)` gives `[ERROR: m]` plus a `Retry` button that calls `bridge::retry()` and sets the state. `Live` shows no overlay. A decoder error shows nothing extra: the panel switches to stills.
7. **Expand:**
   - The frame is a `<button class="cam-frame" aria-expanded=…>` (labelled `Expand camera` / `Collapse camera`) that toggles `expanded: RwSignal<bool>`, a prop passed in by the page.
   - A `keydown` listener on `window`, added on mount and removed on cleanup, collapses on `Escape` while expanded.
8. **Async safety:** every async callback uses `try_set`/`try_get_untracked`, because the owner may be gone.

`CameraThumb` (placed by Task 9, built here so both share the code):
- An `<a href="/printer" class="cam-thumb" aria-label="Open the live camera">` with an `<img>`.
- It calls `bridge::still()` on mount and then every 8 s, and listens to `camera://state`.
- It renders nothing until a still arrives, and again whenever the state `is_off()` or a still answer is `None`.

- [ ] **Step 6: Place the panel on the page**
- In `PrinterPage` (`src/pages/printer.rs`), add `let expanded = RwSignal::new(false);`.
- Replace `<Hero view=view />` with the block below. The status line format is `RUNNING · 42% · Layer 80/200`, from the same fields the hero shows; reuse `format_remaining` etc. where helpful, plus a small `status_line(&PrinterView) -> String` with a unit test.

```rust
                        <div class="pr-top" class:pr-top-expanded=move || expanded.get()>
                            <Show when=move || !expanded.get() fallback=move || view! {
                                <p class="pr-status-line nd-mono">{move || view.with(status_line)}</p>
                            }>
                                <Hero view=view />
                            </Show>
                            <CameraPanel expanded=expanded />
                        </div>
```

- In `style/camera.css`, using `--nd-*` tokens only:

```css
.pr-top { display: grid; gap: var(--nd-space-lg); grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); align-items: start; }
.pr-top-expanded { grid-template-columns: minmax(0, 1fr); }
@media (max-width: 900px) { .pr-top { grid-template-columns: minmax(0, 1fr); } }
.pr-status-line { margin: 0; color: var(--nd-text-secondary); letter-spacing: 0.06em; }
.cam-panel { display: grid; gap: var(--nd-space-sm); min-width: 0; }
.cam-frame { position: relative; display: block; width: 100%; aspect-ratio: 16 / 9; padding: 0; border: 1px solid var(--nd-border); border-radius: 12px; background: var(--nd-surface); overflow: hidden; cursor: zoom-in; }
.pr-top-expanded .cam-frame { cursor: zoom-out; }
.cam-frame img, .cam-frame canvas { width: 100%; height: 100%; object-fit: contain; display: block; }
.cam-overlay { position: absolute; inset: 0; display: grid; place-content: center; gap: var(--nd-space-sm); padding: var(--nd-space-lg); text-align: center; color: var(--nd-text-secondary); }
.cam-overlay .cam-title { color: var(--nd-text-primary); }
.cam-caption { display: flex; justify-content: space-between; gap: var(--nd-space-md); font-family: var(--nd-font-mono); font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--nd-text-secondary); }
.cam-thumb { display: block; width: 160px; aspect-ratio: 16 / 9; border: 1px solid var(--nd-border); border-radius: 8px; overflow: hidden; background: var(--nd-surface); }
.cam-thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
```

Check `style/tokens.css` for the real token names (`--nd-space-*`, `--nd-font-mono`, `--nd-text-*`) and use those that exist. No literal colours. If the repo has a focus-ring rule, `.cam-frame:focus-visible` follows it.

- [ ] **Step 7: Add the fixtures and flows**

In `tests/webkit/fixtures.mjs`, export camera states that match `src/camera/wire.json`:

```js
export const CAMERA_LIVE_JPEG = { transport: "jpeg", status: { kind: "streaming" }, last_frame_at: 1761600000 };
export const CAMERA_LIVE_RTSPS = { transport: "rtsps", status: { kind: "streaming" }, last_frame_at: 1761600000 };
export const cameraOff = (reason, message) => ({ transport: null, status: { kind: "off", reason, message }, last_frame_at: null });
export const CAMERA_BUSY = {
  transport: "jpeg",
  status: { kind: "error", error: { kind: "busy" }, message: "The camera is in use by another app. Close the live view in Bambu Studio or Handy, then retry." },
  last_frame_at: null,
};
// 1×1 white JPEG.
export const JPEG_1X1_BASE64 = "/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA=";
```

The default `FIXTURES` gains the following, so no other flow opens a stream:

```js
camera_state: cameraOff("liveview_off", "Turn on LAN Mode Liveview"),
camera_watch: 1,
camera_unwatch: null,
camera_still: null,
camera_retry: cameraOff("liveview_off", "Turn on LAN Mode Liveview"),
```

**The 1×1 JPEG:** if the base64 isn't a decodable JPEG in both engines, build one in Node at fixture load with a minimal JPEG encoder, or replace it with any valid tiny JPEG's base64 (checked with `file`). The flows need only an `<img>` whose `naturalWidth > 0`.

In `installTauriMock` (`app-flows.mjs`), add a fake `Channel`, keep every instance, and expose it:

```js
  const channels = (window.__cameraChannels = []);
  class Channel {
    constructor() { this.onmessage = () => {}; channels.push(this); }
    toJSON() { return "__CHANNEL__"; }
  }
  window.__TAURI__ = { core: { invoke, Channel }, event: { listen } };
```

Add a helper that sends a message to the newest channel:

```js
const cameraSend = (page, bytes) =>
  page.evaluate((b) => window.__cameraChannels.at(-1).onmessage(new Uint8Array(b).buffer), Array.from(bytes));
```

Add a `VideoDecoder` stand-in for the H.264 flow, installed with `addInitScript` on a fresh page. It records calls and draws nothing:

```js
() => {
  window.__decoded = [];
  window.EncodedVideoChunk = class { constructor(i) { Object.assign(this, i); } };
  window.VideoDecoder = class {
    static async isConfigSupported(c) { window.__decoderConfig = c.codec; return { supported: true }; }
    constructor(cb) { this.cb = cb; this.state = "unconfigured"; this.decodeQueueSize = 0; }
    configure() { this.state = "configured"; }
    decode(chunk) { window.__decoded.push(chunk.type); }
    close() { this.state = "closed"; }
  };
}
```

Flows (each a `step(...)` in the Printer section, after the existing Printer → Live steps):

| Flow | What it checks |
|---|---|
| `the camera panel sits beside the status and says how to turn on liveview` | `.cam-panel` is visible next to `.pr-hero` (both bounding boxes in one row at 1280px); `Turn on LAN Mode Liveview` and the steps text are visible; no `camera_watch` call |
| `a P1 camera streams JPEGs into the panel` | Set `camera_state` to `CAMERA_LIVE_JPEG` and re-enter the page. `camera_watch` is called with `onFrame`. `cameraSend` `[3, 0, ...jpegBytes]`. The panel's `img` gets a `blob:` src with `naturalWidth > 0`, and the caption reads `LIVE · ` plus the fixture model, uppercased |
| `an H2 camera decodes H.264 with WebCodecs` | With the decoder stand-in and `CAMERA_LIVE_RTSPS`: send a config (tag 1, codec `avc1.42c01e`, payload `[1, 66, 192, 30]`), then a delta (tag 2, flag 0), then a key (tag 2, flag 1). `__decoderConfig == "avc1.42c01e"` and `__decoded` is `["key"]`, since the delta before the first key was dropped. A `canvas` is in the frame |
| `without WebCodecs an H2 camera shows refreshed stills` | Fresh page, `delete window.VideoDecoder` in an init script, `CAMERA_LIVE_RTSPS`, and `camera_still` answering `"data:image/jpeg;base64," + JPEG_1X1_BASE64`. No `camera_watch`; `camera_still` is called again within 3 s; the note matches `/^STILL · \d\d:\d\d:\d\d$/` |
| `a busy camera says so and Retry asks again` | `camera_state: CAMERA_BUSY`. The text `[ERROR: The camera is in use by another app. …]` is shown; clicking `Retry` calls `camera_retry` |
| `camera states follow the camera event` | `window.__emit("camera://state", cameraOff("no_camera", "This printer has no camera."))` shows `This printer has no camera.` |
| `the camera expands and Esc collapses it` | Click `.cam-frame`: `aria-expanded="true"`, `.pr-hero` is hidden and `.pr-status-line` is shown. `Escape` reverses both |
| `leaving Printer → Live releases the camera` | Navigate to Home after a watch: `camera_unwatch` is called with `{ id: 1 }` |

Run `trunk build`, then `cd tests/webkit && node app-flows.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../..`. Expected: every flow is OK in both WebKit and Chromium, with no `pageerror` or `console.error`. If `layout.mjs` checks the Printer page at 375px, the panel must not overflow there.

- [ ] **Step 8: Run the Rust checks**

Run: `cargo test --bin bambumate` and `cargo check --target wasm32-unknown-unknown` (CI runs no clippy on the frontend).

- [ ] **Step 9: Format and commit**

```bash
rustfmt --edition 2021 src/camera/mod.rs src/camera/types.rs src/camera/shim.rs src/camera/bridge.rs src/camera/view.rs src/pages/printer.rs src/main.rs
git add src/camera/mod.rs src/camera/types.rs src/camera/shim.rs src/camera/bridge.rs src/camera/view.rs src/pages/printer.rs src/main.rs style/camera.css index.html tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "feat(camera): live camera panel beside the printer status"
```

---

### Task 9: Home thumbnail, journal snapshot labels, Settings switches, About credit

**Files:**
- Modify: `src/pages/home.rs`: `<CameraThumb />` in the `HomeMode::Printing` hero, after `SegmentedBar`.
- Modify: `src/journal/types.rs`: `Photo.label: Option<String>` and `PrintEntry.snapshot_note: Option<String>`, both under the structs' existing `#[serde(default)]`.
- Modify: `src/pages/journal_detail.rs`: the label on `PhotoThumb` and in `photo_alt`, and the miss note under the strip.
- Modify: `src/pages/settings.rs`: a `Camera` section after `<PrinterSettings />`.
- Modify: `src/pages/about.rs`: the openh264 credit.
- Modify: `style/camera.css`, `style/journal.css` (label and note styles).
- Modify: `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs`.

**Interfaces:**
- Consumes: `camera::view::CameraThumb` (Task 8); the journal JSON fields `photos[].label` and `snapshot_note` (Task 7); preference keys `camera_live_view` and `camera_snapshots`; `commands::{get_preference, set_preference}`.
- Produces: no new interfaces.

- [ ] **Step 1: Write the failing `photo_alt` test** in `journal_detail.rs`, next to the existing `photo_alt` tests. Use their date and offset values:

```rust
    #[test]
    fn a_snapshot_is_named_by_its_moment() {
        // 2026-10-21 09:00 UTC.
        assert_eq!(
            photo_alt(2, 3, Some("50 %"), 1_792_573_200, 0),
            "50 % snapshot, photo 2 of 3, added 21 Oct 2026"
        );
        assert_eq!(
            photo_alt(2, 3, None, 1_792_573_200, 0),
            "Photo 2 of 3, added 21 Oct 2026"
        );
    }
```

Update the existing `photo_alt` test's two calls to pass `None` as the third argument; their expected text doesn't change. Change `photo_alt(n, total, added_at, offset)` to `photo_alt(n, total, label: Option<&str>, added_at, offset)`. With a label it reads `"{label} snapshot, photo {n} of {total}, added {date}"`, and without one it is unchanged.

- [ ] **Step 2: Implement the journal changes**
- Pass `label` into `PhotoThumb`. The `photos` memo becomes `(id, added_at, label)`.
- Render `<span class="jr-photo-label nd-mono" aria-hidden="true">{label}</span>` over the thumbnail when there is one; the alt text carries it for screen readers.
- Under the strip, inside the Photos section after `jr-photo-actions`, render `<p class="jr-snapshot-note">{format!("No camera snapshot: {reason}")}</p>` when `entry.snapshot_note` is `Some`. The reason goes through the page's `untrusted_text`-style cleaning if the journal page has one; otherwise plain text with a 200-character cap.
- In `style/journal.css`:

```css
.jr-photo-label { position: absolute; left: 6px; bottom: 6px; padding: 2px 6px; border-radius: 4px; background: var(--nd-surface); color: var(--nd-text-primary); font-size: 10px; letter-spacing: 0.08em; }
.jr-snapshot-note { margin: 0; color: var(--nd-text-secondary); font-size: 13px; }
```

The thumbnail's container needs `position: relative`; add it to the existing photo-thumb class if it lacks it.

- [ ] **Step 3: Home**
- In the `HomeMode::Printing` arm of `home.rs`, add `<CameraThumb />` right after `<SegmentedBar … />`, inside the same fragment.
- In `style/home.css`, if needed, give it `margin-top: var(--nd-space-md)` through a `.hm-hero .cam-thumb` rule.

- [ ] **Step 4: Settings**
- Add a `Camera` section after `<PrinterSettings />` in `settings.rs`, with two toggles in the setup wizard's `feature-toggle` / `toggle-label` / `toggle-input` markup:

```rust
            <section class="settings-section settings-camera">
                <h2 class="nd-label settings-section-title">"Camera"</h2>
                <p class="section-description">"The printer's camera, over your local network only."</p>
                <CameraSwitch key="camera_live_view" label="Live view"
                    note="Shows the camera on Printer → Live and Home." />
                <CameraSwitch key="camera_snapshots" label="Journal snapshots"
                    note="Photographs each print after the first layer, half way and at the end." />
            </section>
```

- `CameraSwitch` is a small component in `settings.rs`:
  - It reads `get_preference(key)` on mount; on unless the value is `"false"`.
  - A change saves `"true"` or `"false"` with `set_preference`.
  - On failure it reverts the box and shows `<span class="status-text status-warning">"Couldn't save the setting."</span>`.
  - It uses `try_set` after awaits.
  - The input has `id` `camera-live-view` or `camera-snapshots`, and the label's `for` points at it.

- [ ] **Step 5: About**
- In `about.rs`, add one line where third-party credits or licences are listed (or under the version block, if there is no such list): `"Video decoding: OpenH264 (BSD-2-Clause), from Cisco Systems."`.

- [ ] **Step 6: Add the flows**

| Flow | What it checks |
|---|---|
| `Home shows a camera thumbnail while printing` | With the existing printing Home fixture and `camera_still` answering a JPEG data URL: `.cam-thumb img` is visible and its `href` is `/printer` |
| `Home hides the thumbnail when there is no picture` | `camera_still: null`: no `.cam-thumb` |
| `journal snapshots carry their moment` | `journal_get` answers `JOURNAL_FINISHED` with `photos: [{ id: 7, file_name: "5f0c.jpg", added_at: …, label: "50 %" }]` and `snapshot_note: "This printer has no camera."`: `.jr-photo-label` reads `50 %`, the image alt starts `50 % snapshot`, and `No camera snapshot: This printer has no camera.` is visible |
| `Settings saves the camera switches` | Unticking `Live view` sends `set_preference { key: "camera_live_view", value: "false" }`; `Journal snapshots` likewise |
| `About credits OpenH264` | The text is visible on `/settings/about` |

Run `trunk build`, then `cd tests/webkit && node app-flows.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../..`. Expected: all OK in both engines.

- [ ] **Step 7: Run the Rust checks, format and commit**

Run: `cargo test --bin bambumate` and `cargo check --target wasm32-unknown-unknown`.

```bash
rustfmt --edition 2021 src/pages/home.rs src/pages/journal_detail.rs src/journal/types.rs src/pages/settings.rs src/pages/about.rs
git add src/pages/home.rs src/pages/journal_detail.rs src/journal/types.rs src/pages/settings.rs src/pages/about.rs style/camera.css style/journal.css style/home.css tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "feat(camera): Home thumbnail, labelled journal snapshots, and camera settings"
```

Stage only the files that actually changed. For example, leave out `style/home.css` if it wasn't touched.

---

## Self-review notes (controller)

- **Spec coverage:**

  | Spec part | Task |
  |---|---|
  | Transports | 1, 2, 4 |
  | Shared session, linger, still cache, errors, retry, stall, backoff | 5 |
  | Commands, preferences, read-only | 6 |
  | Journal moments, v3, once per entry, misses | 7 |
  | Panel, states, expand/Esc, visibility, WebCodecs, still fallback | 8 |
  | Home, journal labels, Settings, About credit | 9 |

  The hardware checks stay on the user (listed in the PR).
- **Out of scope**, as in the spec: timelapse, recording, cloud viewing, and camera or light control.
