# LAN Camera: Live View and Journal Snapshots

**Status:** Designed 2026-10-27, section by section, with the user's approval at each step.
**Sub-project:** Phase 3 (print intelligence), fifth and last feature. The earlier ones were the filament advisor (#34), the print journal (#35), AMS detection (#36) and calibration mode (#37).
**Branch:** `claude/lan-camera`, from `main`.

## Goal

Show the printer's camera inside BambuMate, over the local network only. Also let the print journal photograph each print at a few key moments, without the user doing anything.

## Decisions

| Topic | Decision |
|---|---|
| Scope | **Live view + journal snapshots.** No timelapse, recording or cloud viewing. |
| Decoding | **WebCodecs + openh264.** Live H.264 is decoded in the webview with WebCodecs. The backend decodes single keyframes with `openh264` (compiled in, `source` feature) for snapshots and for the still fallback. |
| Placement | **Panel beside status** on Printer → Live, plus a small thumbnail in Home's printing hero. |
| Snapshots | **Key moments:** after the first layer, at 50 %, and at the end (finished, failed, or paused with an error). |

## What the printers offer

These facts come from public protocol notes (OpenBambuAPI, ha-bambulab) and from reading Bambu Studio's source. Studio is AGPL: it was read for facts, and none of its code is copied.

- **H2, X1 and P2S families:**
  - The stream is RTSPS on port 322 at `rtsps://bblp:<access code>@<printer ip>:322/streaming/live/1`.
  - The server is live555, with Digest auth. The video is H.264 over RTP, interleaved on the TCP connection.
- **P1 and A1 families:**
  - The stream is a TLS connection on port 6000.
  - The client sends an 80-byte auth packet: little-endian u32s `0x40`, `0x3000`, `0`, `0`, then `bblp` and the access code, each NUL-padded to 32 bytes.
  - Each frame is a 16-byte header (u32 LE size, 0, 1, 0) followed by exactly that many JPEG bytes. Frames arrive every 1–2 s.
  - A wrong access code makes the printer close the connection without sending anything.
- **MQTT `print.ipcam`:**
  - `ipcam_dev` is `"0"` when there is no camera.
  - `rtsp_url` reads `"disable"` when LAN liveview is off, and an `rtsps://…` URL when it is on. An empty value means unknown.
  - The host inside `rtsp_url` can be wrong. BambuMate always dials the printer IP it already uses.
- **Certificates:** the printer certificate's CN is the serial, and it has no IP SAN. Some firmware never answers a TLS 1.3 hello. The existing `printer::tls` verifier (CA bundle, serial check, fingerprint pin) covers both points.
- **Session limits:**
  - Printers allow only a few camera sessions. P1S stalls a third stream.
  - Studio reports "The device cannot handle more conversations" when the cap is hit.
- **RTCP risk:**
  - live555 servers expect RTCP receiver reports. One X1C report says the stream went silent after about 10 s without them.
  - `retina` doesn't send receiver reports.
  - This must be checked on the user's H2 first. If it bites, the fallback is a small custom RTSP client that sends them.
- **WebCodecs:**
  - `VideoDecoder` exists in Safari 16.4+ (macOS 13.3+) and in Chromium (WebView2).
  - BambuMate supports macOS 10.15. On older macOS the live view falls back to stills.

## Design

### 1. Backend (`src-tauri/src/camera/`)

**`transport.rs`**
- `choose(model, ipcam) -> Result<Transport, OffReason>` decides in this order:
  1. `ipcam_dev == "0"` → `NoCamera`;
  2. `rtsp_url == "disable"` → `LiveviewOff`;
  3. `rtsp_url` starts with `rtsp` → `Rtsps`;
  4. a P1 or A1 model → `Jpeg`;
  5. an X1, H2 or P2S model → `Rtsps`;
  6. no `ipcam` yet → `Waiting`; otherwise `Unsupported`.
- The service adds `NotConfigured` and `NotConnected`, and the commands add `Disabled` (live view switched off).
- The printer state keeps only a classification of `rtsp_url` (off, RTSP, or unset), never the URL itself.
- A pure function with table tests.

**`jpeg6000.rs`**
- Opens TCP to port 6000 and wraps it in TLS with `printer::tls::client_config`, using the serial as the server name.
- Writes the 80-byte auth packet, then loops: read the 16-byte header, then read the JPEG.
- Limits:
  - A frame larger than 4 MiB, or a header with the wrong constant words, ends the session with an error.
  - A JPEG must start `FF D8` and end `FF D9`, or it is dropped.
- The parser is a pure `FrameReader` over bytes, so it can be tested without a socket.

**`rtsps.rs`**
- Binds a one-shot loopback listener on `127.0.0.1:0`.
- Relays that socket to the printer's port 322 through TLS, with the same verifier.
- Runs `retina` against `rtsp://bblp:<code>@127.0.0.1:<port>/streaming/live/1` with Digest auth and TCP transport.
- Output:
  - A `Config` message with the RFC 6381 codec string and the `avcC` bytes, sent first.
  - Each access unit as AVC length-prefixed data, with a keyframe flag.
- Frames before the first keyframe are dropped.
- If no frame arrives for 10 s, it reconnects, backing off from 1 s to 30 s.

**`session.rs`**
- `CameraHub` keeps at most one session per printer, shared by every consumer.
- Consumers:
  - each live viewer, through `camera_watch`;
  - each snapshot job.
- The session closes 10 s after the last consumer leaves.
- Errors map to the UI states in section 2:
  - **Auth:** port 6000 closed before any frame.
  - **Tls:** the certificate check failed.
  - **Refused:** port 322 refused or closed at once. This is liveview off, or another viewer holding the camera.
  - **Busy:** connected, but no frame within 10 s.
  - Any other error retries with backoff.
- Auth, Tls, Refused and Busy stop the session until Retry. A journal snapshot always makes one fresh attempt; a still never does.
- A panic in the session's driver stops it with an error until Retry.
- State changes are only sent while a session runs, so the live panel keeps a watch open while the camera is off for a reason the printer can end (liveview off, not connected, waiting, not configured).
- The latest JPEG (from 6000, or a decoded keyframe from RTSPS) is cached for the Home thumbnail and for the still fallback.

**Snapshots (`CameraHub::capture` in `session.rs`)**
- `capture() -> Result<Jpeg, String>` joins the session as a consumer, with a 15 s timeout.
- On port 6000 it takes the next frame.
- On RTSPS it takes the next keyframe and decodes it with `openh264`: AVC to Annex B with SPS/PPS prepended, then YUV to RGB, then a JPEG at quality 85 with the longest side capped at 1920 px.
- The YUV-to-JPEG step is a pure function.

**Commands (`commands/camera.rs`)**
- BambuMate has one printer, so the commands take no printer id.
- `camera_state() -> CameraState`: the transport, the current state, and the time of the last frame.
- `camera_watch(on_frame: Channel) -> id` sends frames as raw bytes.
  - Each message starts with a one-byte tag: `1` config, `2` access unit, `3` JPEG.
  - For config and access units the tag is followed by a u8 keyframe flag, then the payload.
  - `camera_unwatch(id)` releases the consumer. A channel the page has let go of does not: in Tauri 2.10 `Channel::send` succeeds while the webview exists, so only a webview that is gone ends a watch by itself.
- `camera_still() -> Option<data URL>` returns the latest JPEG if it is at most 5 s old. Otherwise it waits up to 15 s for the next one. It serves Home and the fallback. On a session stopped by an error it returns nothing at once, without connecting again.
- `camera_retry() -> CameraState`.
- State changes go out as `camera://state` events. With live view off in Settings, they always say `Disabled`.

**Settings**
- `camera_live_view` and `camera_snapshots` are stored as preferences through the existing `get_preference`/`set_preference`. Both are on by default.
- With live view off, no session opens for viewers.
- With snapshots off, the journal never asks for one.

**Read-only**
- MQTT stays read-only (`pushall`, `get_version`).
- The camera connections only do the stream handshake. They never send commands, toggle liveview, or touch the chamber light.

### 2. Interface

**Printer → Live**
- A camera panel sits beside the status hero, and drops below it on narrow screens.
- The frame is a flat 16:9 block on `--nd-surface`, with no shadow and no radius above the existing card radius.
- A mono caption reads `LIVE · <MODEL>`, with the last frame's time on the right.
- Clicking the frame expands it to full width, and Esc collapses it. While expanded, the status shows as one mono line.
- Streaming runs only while the panel is on screen (`IntersectionObserver`) and the window is visible. Otherwise the panel unwatches.

**States (text, no illustrations)**
- `[CONNECTING…]`
- **Liveview off:** "Turn on LAN Mode Liveview", with the steps: on the printer screen, Settings → General (or LAN Only) → LAN Mode Liveview.
- "The camera is in use by another app. Close the live view in Bambu Studio or Handy, then retry."
- "This printer has no camera."
- "The printer isn't connected."
- `[ERROR: <message>]` with Retry.
  - A wrong access code reads "Check the access code in Settings".
  - A TLS failure reads "Couldn't verify the printer's camera".
- **Showing the latest still:** when WebCodecs is missing or a decoder error occurs, the panel shows the cached JPEG, refreshed about every 2 s, with a mono note `STILL · <time>`.

**Decoding (`src/camera/`)**
- `src/camera/types.rs` holds the mirrors, `bridge.rs` the commands and channel, and `view.rs` the `<CameraView>` component.
- A small JS shim does the WebCodecs work:
  - It feature-detects `VideoDecoder`, then calls `isConfigSupported({codec, description})`.
  - It configures the decoder and feeds `EncodedVideoChunk`s, keyframe first.
  - It draws each `VideoFrame` onto a canvas, then closes the frame.
- Port-6000 JPEGs go into an `<img>` through object URLs. Each previous URL is revoked.
- The decoder, the channel and any object URL are released when the component is disposed.
- Every byte from the printer is untrusted:
  - message sizes are capped, at 4 MiB each;
  - an unknown tag is ignored;
  - any decoder error falls back to the still view.

**Home**
- While printing and liveview is available, the printing hero shows a small thumbnail of the latest still. It refreshes every 8 s, inside the session's 10 s linger, so the session stays open.
- It links to Printer → Live.
- It is hidden when not printing, when the transport is `Off`, or when live view is turned off in Settings.

**Journal**
- Automatic snapshots appear in the entry's photo strip with a mono label: `FIRST LAYER`, `50 %`, `FINISHED`, `FAILED` or `PAUSED`.
- They can be opened, deleted or sent to Analyze, like any other photo.
- A missed snapshot leaves one quiet line on the entry: "No camera snapshot: <reason>".

**Settings**
- A Camera row with two switches: "Live view" and "Journal snapshots".

### 3. Data, errors and testing

**Journal snapshots**
- The recorder already sees layer, progress and state. A pure rule, `snapshot_moments(prev, now, report) -> Vec<Moment>`, fires:
  - `FirstLayer` when the job's layer is 2 or 3;
  - `Half` when its progress is 50–55 %;
  - `Finished` on FINISH;
  - `Failed` on FAILED;
  - `Paused` on PAUSE while `print_error` is non-zero. HMS codes alone don't count: many are warnings.
- A moment whose window was missed (for example, the app joined at 70 %) is skipped, not taken late.
- Each moment is taken at most once per entry.
  - Moments taken are stored with the entry, so a restart mid-print doesn't repeat them.
  - Calibration prints are photographed like any other print (user ruling, 2026-10-27). They are linked to a step only after they end, and the finished photo shows the test pattern.
- Captures run off the recorder's writer thread, so a slow camera never delays journal writes.
- The JPEG goes through the existing `JournalStore::add_photo` path with its label.
- **Schema v3:**
  - `print_photos` gets a nullable `label TEXT`.
  - `prints` gets a `snapshots_json TEXT` holding the moments taken and the last miss reason.
  - The migration `migrate_to_v3` follows the v2 pattern: in a transaction, with a column check.

**Errors**
- **Snapshot failures** (timeout, no camera, liveview off, in use) never affect the print or the journal entry. Only the reason is recorded.
- With snapshots switched off, nothing is taken and nothing is noted.
- **Live view errors** show as the states above. Retry rebuilds the session.
- **Logs** never contain the access code. URLs are logged with the password removed.

**Testing**
- **Unit tests:**
  - **Port-6000 framing:** the auth packet layout; header parsing; split reads; oversize frames; wrong constant words; truncated frames; bad JPEG markers.
  - **Transport choice:** model × `ipcam` cases.
  - **Session hub:** consumer counting and the close timer, using `tokio::time::pause`.
  - **Snapshot rules:** each moment once; restart resume; pause with and without an error.
  - **openh264 keyframe to JPEG:** on a small recorded H.264 keyframe fixture, generated once by a test helper and checked in under `testdata/`.
  - **Journal v3 migration** from v2 and v0 files.
  - **Channel message encoding.**
- **Fake printer:** an in-process TLS server using a test certificate. It speaks the port-6000 protocol, so the JPEG session can be tested end to end, including a wrong code (connection closed) and the session cap (closed at once).
- **Browser flows** on fixtures, in `tests/webkit/app-flows.mjs`:
  - every camera state's text;
  - expand and collapse;
  - the still fallback when `VideoDecoder` is missing;
  - the Home thumbnail appearing and hiding;
  - journal snapshot labels and the miss note;
  - the Settings switches.
- **Tests never reach a real printer** and never spend AI tokens.
- **Hardware checks on the user's H2**, in order:
  1. The RTSPS stream runs past 30 s without RTCP receiver reports.
  2. It runs alongside Bambu Studio or Handy.
  3. Snapshots during a real print.

## Out of scope

- Timelapse and recording video.
- Cloud or remote viewing (TUTK or Agora).
- Controlling the camera or the chamber light, or turning liveview on from the app.
- Live-view audio, zoom or pan.

## Known caveats

- **openh264 patents:** compiling openh264 from source doesn't carry Cisco's patent licence, which covers only Cisco's separately downloaded binary. The user chose this route. The About page credits openh264 (BSD-2-Clause).
- **Cloud mode:** whether the H2 serves LAN liveview while in cloud mode isn't documented. The design reads `rtsp_url` and reports what it finds.
