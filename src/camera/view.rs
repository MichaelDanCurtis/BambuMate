//! The camera panel beside the status on Printer → Live, and the small
//! thumbnail that links to it.
//!
//! The panel runs one of three modes, derived from whether it is on screen,
//! the backend's state, and whether the browser has WebCodecs: nothing, a
//! live watch (JPEGs, or H.264 decoded onto a canvas), or a still fetched
//! every 2 s. Leaving a mode stops everything it started.
//!
//! The backend only sends `camera://state` while a session runs, so the
//! panel keeps a watch open while the camera is off for a reason the
//! printer can end, and asks for the state again when no session runs.
//!
//! The Channel, the decoder and the timers call back after awaits or from
//! JS, possibly once the panel is gone, so they touch signals only through
//! `try_*`, and a stopped [`Session`] ignores whatever still arrives.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use js_sys::{Function, Uint8Array};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::bridge::{self, Watch};
use super::shim::{self, CameraDecoder};
use super::types::{
    parse, shown, CameraState, CameraStatus, Kind, Shown, Transport, HEADER_COPY, LIVEVIEW_STEPS,
    MAX_MESSAGE,
};
use crate::agent::bridge::listen_scoped;
use crate::printer::types::untrusted_text;
use crate::printer::PrinterShared;

/// How often the panel asks for a still when it can't show live video.
const STILL_EVERY: Duration = Duration::from_secs(2);

/// How often the thumbnail asks for a still.
const THUMB_EVERY: Duration = Duration::from_secs(8);

/// How long after a failed watch the panel tries again, at first and at
/// most.
const WATCH_AGAIN_FIRST: Duration = Duration::from_secs(2);
const WATCH_AGAIN_MAX: Duration = Duration::from_secs(30);

/// Most characters shown of the model the printer reports.
const MODEL_MAX: usize = 32;

/// The title over [`LIVEVIEW_STEPS`].
const LIVEVIEW_TITLE: &str = "Turn on LAN Mode Liveview";

/// What the panel runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Off screen, the camera is off for good, or no session runs (the
    /// panel asks for the state instead).
    Nothing,
    /// A watch on the backend's Channel, for this transport. With none
    /// (the camera is off for now) it only keeps the session open, so the
    /// session checks the printer again and says when that changes; a
    /// transport arriving starts a new watch.
    Live(Option<Transport>),
    /// A still fetched every [`STILL_EVERY`].
    Stills,
}

/// `fallback` is set once the browser couldn't decode the H.264 stream.
fn mode_for(visible: bool, state: &CameraState, has_decoder: bool, fallback: bool) -> Mode {
    if !visible {
        return Mode::Nothing;
    }
    if let CameraStatus::Off { reason, .. } = state.status {
        return if reason.may_change() {
            Mode::Live(None)
        } else {
            Mode::Nothing
        };
    }
    match state.transport {
        Some(Transport::Jpeg) => Mode::Live(Some(Transport::Jpeg)),
        Some(Transport::Rtsps) if has_decoder && !fallback => Mode::Live(Some(Transport::Rtsps)),
        Some(Transport::Rtsps) => Mode::Stills,
        None if state.status == CameraStatus::Idle => Mode::Nothing,
        None => Mode::Live(None),
    }
}

/// `HH:MM:SS` of `unix_secs`, at `offset_minutes` east of UTC.
fn clock_text(unix_secs: i64, offset_minutes: i32) -> String {
    let secs = (unix_secs + i64::from(offset_minutes) * 60).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

/// [`clock_text`] in the local time zone.
fn local_clock(unix_secs: i64) -> String {
    let at = js_sys::Date::new(&JsValue::from_f64(unix_secs as f64 * 1000.0));
    clock_text(unix_secs, -(at.get_timezone_offset() as i32))
}

fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// What a running mode holds, and what stopping it releases.
#[derive(Default)]
struct Running {
    stopped: bool,
    watch: Option<Watch>,
    decoder: Option<CameraDecoder>,
    /// The object URL of the JPEG shown.
    url: Option<String>,
    interval: Option<IntervalHandle>,
    /// A still request is out: the next tick doesn't send another.
    asking: bool,
}

type Session = Rc<RefCell<Running>>;

fn stop(session: &Session) {
    let mut s = session.borrow_mut();
    s.stopped = true;
    if let Some(watch) = s.watch.take() {
        let id = watch.id;
        drop(watch);
        spawn_local(bridge::unwatch(id));
    }
    if let Some(decoder) = s.decoder.take() {
        decoder.close();
    }
    if let Some(url) = s.url.take() {
        shim::revoke(&url);
    }
    if let Some(interval) = s.interval.take() {
        interval.clear();
    }
}

/// The signals a running mode writes.
#[derive(Clone, Copy)]
struct Outputs {
    picture: RwSignal<Option<String>>,
    still_at: RwSignal<Option<i64>>,
    fallback: RwSignal<bool>,
    canvas: NodeRef<html::Canvas>,
}

/// Watches the backend's Channel. A watch is how the panel hears about
/// changes, so a failed one asks for the state (which may say why, and
/// change the mode) and tries again after `wait`, doubling, while the mode
/// holds.
fn start_live(session: Session, out: Outputs, asker: Asker, wait: Duration) {
    let receiving = session.clone();
    let on_bytes = move |b: Uint8Array| on_message(&receiving, &b, out);
    spawn_local(async move {
        match bridge::watch(on_bytes).await {
            Ok(watch) => {
                let late = {
                    let mut s = session.borrow_mut();
                    if s.stopped {
                        Some(watch)
                    } else {
                        s.watch = Some(watch);
                        None
                    }
                };
                // Stopped while the backend was answering.
                if let Some(watch) = late {
                    let id = watch.id;
                    drop(watch);
                    bridge::unwatch(id).await;
                }
            }
            Err(e) => {
                web_sys::console::warn_1(&format!("camera: {e}").into());
                if session.borrow().stopped {
                    return;
                }
                asker.ask();
                set_timeout(
                    move || {
                        if !session.borrow().stopped {
                            start_live(session, out, asker, (wait * 2).min(WATCH_AGAIN_MAX));
                        }
                    },
                    wait,
                );
            }
        }
    });
}

/// One Channel message. Only the header is copied into wasm; the payload
/// stays in JS as a view.
fn on_message(session: &Session, b: &Uint8Array, out: Outputs) {
    let len = b.length() as usize;
    if len > MAX_MESSAGE || session.borrow().stopped {
        return;
    }
    let head = b.subarray(0, len.min(HEADER_COPY) as u32).to_vec();
    let Some(parsed) = parse(&head) else {
        return;
    };
    let payload = b.subarray(parsed.payload as u32, len as u32);
    match parsed.kind {
        Kind::Config => {
            let Some(decoder) = decoder_for(session, out) else {
                return;
            };
            let codec = parsed.codec.unwrap_or_default();
            let supported = decoder.configure(&codec, &payload);
            spawn_local(async move {
                let ok = JsFuture::from(supported)
                    .await
                    .ok()
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !ok {
                    out.fallback.try_set(true);
                }
            });
        }
        Kind::Video => {
            if let Some(decoder) = session.borrow().decoder.as_ref() {
                decoder.decode(&payload, parsed.key);
            }
        }
        Kind::Jpeg => {
            let url = shim::jpeg_url(&payload);
            let old = session.borrow_mut().url.replace(url.clone());
            out.picture.try_set(Some(url));
            if let Some(old) = old {
                shim::revoke(&old);
            }
        }
    }
}

/// The session's decoder, made on the first config, drawing onto the
/// canvas. A decoder error switches the panel to stills.
fn decoder_for(session: &Session, out: Outputs) -> Option<CameraDecoder> {
    let mut s = session.borrow_mut();
    if s.decoder.is_none() {
        let canvas = out.canvas.try_get_untracked().flatten()?;
        let fallback = out.fallback;
        let on_error = Closure::<dyn FnMut(String)>::new(move |e: String| {
            web_sys::console::warn_1(&format!("camera: decoding failed: {e}").into());
            fallback.try_set(true);
        });
        s.decoder = Some(CameraDecoder::new(&canvas, &on_error));
        // The decoder may report an error after the session stopped; the
        // callback only reaches a signal through `try_set`.
        on_error.forget();
    }
    s.decoder.clone()
}

fn start_stills(session: Session, out: Outputs) {
    let ask = {
        let session = session.clone();
        move || {
            {
                let mut s = session.borrow_mut();
                if s.stopped || s.asking {
                    return;
                }
                s.asking = true;
            }
            let session = session.clone();
            spawn_local(async move {
                let answer = bridge::still().await;
                let mut s = session.borrow_mut();
                s.asking = false;
                if s.stopped {
                    return;
                }
                if let Ok(Some(url)) = answer {
                    out.picture.try_set(Some(url));
                    out.still_at.try_set(Some(now_secs()));
                }
            });
        }
    };
    ask();
    if let Ok(handle) = set_interval_with_handle(ask, STILL_EVERY) {
        session.borrow_mut().interval = Some(handle);
    }
}

/// Asks the backend for the camera's state. An answer that comes after an
/// event is older than it, and is dropped.
#[derive(Clone, Copy)]
struct Asker {
    state: RwSignal<CameraState>,
    /// `camera://state` events seen.
    events: StoredValue<u64>,
    /// An answer or an event has set `state`.
    known: RwSignal<bool>,
}

impl Asker {
    fn ask(self) {
        let Some(seen) = self.events.try_get_value() else {
            return;
        };
        spawn_local(async move {
            match bridge::state().await {
                Ok(s) if self.events.try_get_value() == Some(seen) => {
                    self.state.try_set(s);
                    self.known.try_set(true);
                }
                Ok(_) => {}
                Err(e) => web_sys::console::warn_1(&format!("camera: {e}").into()),
            }
        });
    }
}

/// Keeps `state` current: the backend's answer on mount, then each
/// `camera://state` event. The returned [`Asker`] asks again.
fn follow_state(state: RwSignal<CameraState>) -> Asker {
    let asker = Asker {
        state,
        events: StoredValue::new(0),
        known: RwSignal::new(false),
    };
    listen_scoped("camera://state", move |s: CameraState| {
        asker.events.try_update_value(|n| *n += 1);
        asker.state.try_set(s);
        asker.known.try_set(true);
    });
    asker.ask();
    asker
}

/// Printer → Live's camera. `expanded` widens it over the status, which
/// the page then shows as one line.
#[component]
pub fn CameraPanel(expanded: RwSignal<bool>) -> impl IntoView {
    let printer = expect_context::<PrinterShared>().view;
    let state = RwSignal::new(CameraState::default());
    let asker = follow_state(state);

    let visible = RwSignal::new(false);
    let fallback = RwSignal::new(false);
    let picture = RwSignal::new(Option::<String>::None);
    let still_at = RwSignal::new(Option::<i64>::None);
    let root = NodeRef::<html::Div>::new();
    let canvas = NodeRef::<html::Canvas>::new();
    let out = Outputs {
        picture,
        still_at,
        fallback,
        canvas,
    };

    // On screen, with the window shown.
    let watching = StoredValue::new_local(None::<(Function, Closure<dyn FnMut(bool)>)>);
    Effect::new(move |_| {
        let Some(el) = root.get() else {
            return;
        };
        if watching.with_value(Option::is_some) {
            return;
        }
        let on_change = Closure::<dyn FnMut(bool)>::new(move |v: bool| {
            visible.try_set(v);
        });
        let remove = shim::visibility(el.unchecked_ref(), &on_change);
        watching.set_value(Some((remove, on_change)));
    });

    let has_decoder = shim::has_decoder();
    let mode =
        Memo::new(move |_| state.with(|s| mode_for(visible.get(), s, has_decoder, fallback.get())));
    let uses_canvas = Memo::new(move |_| mode.get() == Mode::Live(Some(Transport::Rtsps)));

    // No session runs (the hub closed it), so no event will come: ask.
    let no_session = Memo::new(move |_| {
        visible.get()
            && asker.known.get()
            && state.with(|s| s.status == CameraStatus::Idle && s.transport.is_none())
    });
    Effect::new(move |was: Option<bool>| {
        let now = no_session.get();
        if now && was == Some(false) {
            asker.ask();
        }
        now
    });

    let current = StoredValue::new_local(None::<Session>);
    Effect::new(move |_| {
        let m = mode.get();
        if let Some(Some(old)) = current.try_update_value(Option::take) {
            stop(&old);
        }
        picture.set(None);
        still_at.set(None);
        let session = Session::default();
        match m {
            Mode::Nothing => return,
            Mode::Live(_) => start_live(session.clone(), out, asker, WATCH_AGAIN_FIRST),
            Mode::Stills => start_stills(session.clone(), out),
        }
        current.set_value(Some(session));
    });

    let keys = window_event_listener(leptos::ev::keydown, move |e| {
        if e.key() == "Escape" && expanded.try_get_untracked() == Some(true) {
            expanded.try_set(false);
        }
    });
    on_cleanup(move || {
        keys.remove();
        if let Some(Some(session)) = current.try_update_value(Option::take) {
            stop(&session);
        }
        if let Some(Some((remove, on_change))) = watching.try_update_value(Option::take) {
            let _ = remove.call0(&JsValue::NULL);
            drop(on_change);
        }
    });

    let model = Memo::new(move |_| {
        printer.with(|v| {
            v.printer
                .as_ref()
                .map(|p| untrusted_text(&p.model.to_uppercase(), MODEL_MAX))
                .unwrap_or_default()
        })
    });
    let shown_now = Memo::new(move |_| state.with(shown));
    let off = Memo::new(move |_| state.with(CameraState::is_off));
    let last_frame = Memo::new(move |_| state.with(|s| s.last_frame_at));
    let retrying = RwSignal::new(false);
    let retry = move |_| {
        retrying.set(true);
        spawn_local(async move {
            match bridge::retry().await {
                Ok(s) => {
                    state.try_set(s);
                }
                Err(e) => {
                    web_sys::console::warn_1(&format!("camera: {e}").into());
                    // Don't leave the old error up with nothing behind it.
                    asker.ask();
                }
            }
            retrying.try_set(false);
        });
    };

    view! {
        <div class="cam-panel" node_ref=root>
            <div class="cam-stage">
                <button
                    type="button"
                    class="cam-frame"
                    aria-expanded=move || if expanded.get() { "true" } else { "false" }
                    aria-label=move || if expanded.get() { "Collapse camera" } else { "Expand camera" }
                    on:click=move |_| expanded.update(|e| *e = !*e)
                >
                    <Show
                        when=move || uses_canvas.get()
                        fallback=move || picture.get().map(|src| view! { <img src=src alt="" /> })
                    >
                        <canvas node_ref=canvas></canvas>
                    </Show>
                </button>
                {move || match shown_now.get() {
                    Shown::Live => None,
                    Shown::Connecting => Some(view! {
                        <div class="cam-overlay"><p class="nd-mono">"[CONNECTING…]"</p></div>
                    }.into_any()),
                    Shown::LiveviewOff => Some(view! {
                        <div class="cam-overlay">
                            <p class="cam-title">{LIVEVIEW_TITLE}</p>
                            <p>{LIVEVIEW_STEPS}</p>
                        </div>
                    }.into_any()),
                    Shown::Note(text) => Some(view! {
                        <div class="cam-overlay"><p>{text}</p></div>
                    }.into_any()),
                    Shown::Error(message) => Some(view! {
                        <div class="cam-overlay">
                            <p class="nd-mono">{format!("[ERROR: {message}]")}</p>
                            <button
                                type="button"
                                class="btn btn-secondary cam-retry"
                                disabled=move || retrying.get()
                                on:click=retry
                            >"Retry"</button>
                        </div>
                    }.into_any()),
                }}
            </div>
            <Show when=move || !off.get()>
                <p class="cam-caption">
                    <span class="cam-live">{move || format!("LIVE · {}", model.get())}</span>
                    {move || if mode.get() == Mode::Stills {
                        still_at.get().map(|at| view! {
                            <span class="cam-still-note">{format!("STILL · {}", local_clock(last_frame.get().unwrap_or(at)))}</span>
                        }.into_any())
                    } else {
                        last_frame.get().map(|at| view! {
                            <span class="cam-time">{local_clock(at)}</span>
                        }.into_any())
                    }}
                </p>
            </Show>
        </div>
    }
}

/// A small still that opens Printer → Live. Shows nothing while there is
/// no picture or the camera is off.
#[component]
pub fn CameraThumb() -> impl IntoView {
    let src = RwSignal::new(Option::<String>::None);
    let off = RwSignal::new(false);
    // A still request is out: the next tick doesn't send another.
    let asking = RwSignal::new(false);
    listen_scoped("camera://state", move |s: CameraState| {
        off.try_set(s.is_off());
    });
    // The state as it stands now; later changes arrive as events.
    spawn_local(async move {
        if let Ok(state) = bridge::state().await {
            off.try_set(state.is_off());
        }
    });
    let ask = move || {
        if asking.get_untracked() {
            return;
        }
        asking.set(true);
        spawn_local(async move {
            let url = bridge::still().await.ok().flatten();
            src.try_set(url);
            asking.try_set(false);
        });
    };
    ask();
    if let Ok(handle) = set_interval_with_handle(ask, THUMB_EVERY) {
        on_cleanup(move || handle.clear());
    }
    move || {
        if off.get() {
            return None;
        }
        src.get().map(|s| {
            view! {
                <a href="/printer" class="cam-thumb" aria-label="Open the live camera">
                    <img src=s alt="" />
                </a>
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::types::{CameraStatus, OffReason};

    fn with(transport: Option<Transport>, status: CameraStatus) -> CameraState {
        CameraState {
            transport,
            status,
            last_frame_at: None,
        }
    }

    fn off(reason: OffReason) -> CameraState {
        with(
            None,
            CameraStatus::Off {
                reason,
                message: String::new(),
            },
        )
    }

    #[test]
    fn the_mode_follows_visibility_state_and_decoder() {
        let jpeg = with(Some(Transport::Jpeg), CameraStatus::Streaming);
        let rtsps = with(Some(Transport::Rtsps), CameraStatus::Connecting);
        let live_jpeg = Mode::Live(Some(Transport::Jpeg));
        let live_rtsps = Mode::Live(Some(Transport::Rtsps));
        assert_eq!(mode_for(true, &jpeg, false, false), live_jpeg);
        assert_eq!(mode_for(false, &jpeg, true, false), Mode::Nothing);
        assert_eq!(mode_for(true, &rtsps, true, false), live_rtsps);
        assert_eq!(mode_for(true, &rtsps, false, false), Mode::Stills);
        assert_eq!(mode_for(true, &rtsps, true, true), Mode::Stills);
        let busy = with(
            Some(Transport::Jpeg),
            CameraStatus::Error {
                error: crate::camera::types::CameraErrorKind::Busy,
                message: String::new(),
            },
        );
        assert_eq!(mode_for(true, &busy, true, false), live_jpeg);
    }

    // The backend only sends events while a session runs: an off camera
    // keeps one open, unless nothing on the printer can change it.
    #[test]
    fn an_off_camera_keeps_a_watch_while_the_printer_can_change_it() {
        for reason in [
            OffReason::LiveviewOff,
            OffReason::NotConnected,
            OffReason::Waiting,
            OffReason::NotConfigured,
        ] {
            assert_eq!(mode_for(true, &off(reason), true, false), Mode::Live(None));
            assert_eq!(mode_for(false, &off(reason), true, false), Mode::Nothing);
        }
        for reason in [
            OffReason::NoCamera,
            OffReason::Unsupported,
            OffReason::Disabled,
        ] {
            assert_eq!(mode_for(true, &off(reason), true, false), Mode::Nothing);
        }
        // No session: the panel asks for the state rather than watching.
        assert_eq!(
            mode_for(true, &with(None, CameraStatus::Idle), true, false),
            Mode::Nothing
        );
        assert_eq!(
            mode_for(true, &with(None, CameraStatus::Connecting), true, false),
            Mode::Live(None)
        );
    }

    #[test]
    fn the_clock_reads_hours_minutes_and_seconds() {
        // 2025-10-27 21:20:00 UTC.
        assert_eq!(clock_text(1_761_600_000, 0), "21:20:00");
        assert_eq!(clock_text(1_761_600_000, 60), "22:20:00");
        assert_eq!(clock_text(1_761_600_005, -22 * 60), "23:20:05");
    }
}
