//! The model viewport (spec 2026-10-22 §2): a WebGL2 dot or wireframe view
//! of a model file, with its geometry risks pinned on it and listed below.
//!
//! `gl.rs` draws; this file owns loading, interaction and pin projection.
//! Nothing animates: a frame is drawn only when the camera, mode, model,
//! size or theme changes.

pub mod camera;
pub mod decode;
pub mod gl;
pub mod types;

use std::sync::{Arc, Weak};

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlCanvasElement;

use crate::components::patterns::{StateKind, StateText};
use camera::Orbit;
use gl::{Colors, GlProblem, Mode, Renderer};
use types::{ModelView, Risk, RiskKind};

/// Radians of turn per pixel dragged.
const DRAG_TURN: f32 = 0.01;
/// Radians of turn per arrow key press.
const KEY_TURN: f32 = 0.1;
const KEY_ZOOM_IN: f32 = 0.9;
const KEY_ZOOM_OUT: f32 = 1.1;
/// Zoom is `exp(pixels * WHEEL_ZOOM)`.
const WHEEL_ZOOM: f32 = 0.001;
/// Pixels per wheel line and page, for wheels that report those.
const WHEEL_LINE_PX: f64 = 16.0;
const WHEEL_PAGE_PX: f64 = 800.0;
/// One wheel event zooms by at most `exp(1)`.
const WHEEL_MAX_PX: f64 = 1000.0;

/// What the canvas area says when there is no renderer.
pub fn problem_message(problem: GlProblem) -> &'static str {
    match problem {
        GlProblem::NoWebGl => "3D preview needs WebGL2",
        GlProblem::SetupFailed => "3D preview couldn't start",
    }
}

pub fn kind_label(kind: RiskKind) -> &'static str {
    match kind {
        RiskKind::Overhang => "Overhang",
        RiskKind::SmallBedContact => "Small bed contact",
        RiskKind::TallThin => "Tall and thin",
    }
}

/// The list number for risk `index`, counting from `01`.
pub fn risk_number(index: usize) -> String {
    format!("{:02}", index + 1)
}

fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Millimetres: one decimal under 100 mm (no trailing `.0`), whole above.
/// Negative or non-finite sizes print as 0.
fn format_mm(v: f32) -> String {
    let v = if v.is_finite() && v > 0.0 { v } else { 0.0 };
    if v >= 100.0 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.1}");
        s.strip_suffix(".0").map(str::to_owned).unwrap_or(s)
    }
}

/// `"{N} triangles · {W} × {D} × {H} mm"`.
pub fn meta_line(triangles: usize, min: [f32; 3], max: [f32; 3]) -> String {
    let noun = if triangles == 1 {
        "triangle"
    } else {
        "triangles"
    };
    let [w, d, h] = [0, 1, 2].map(|i| format_mm(max[i] - min[i]));
    format!("{} {noun} · {w} × {d} × {h} mm", group_thousands(triangles))
}

pub fn canvas_label(name: &str) -> String {
    format!("3D view of {name}. Drag or use arrow keys to rotate, + and − to zoom, 0 to reset.")
}

/// What a key does to the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyAction {
    /// Yaw and pitch deltas in radians.
    Rotate(f32, f32),
    Zoom(f32),
    Refit,
}

pub fn key_action(key: &str) -> Option<KeyAction> {
    Some(match key {
        "ArrowLeft" => KeyAction::Rotate(KEY_TURN, 0.0),
        "ArrowRight" => KeyAction::Rotate(-KEY_TURN, 0.0),
        "ArrowUp" => KeyAction::Rotate(0.0, KEY_TURN),
        "ArrowDown" => KeyAction::Rotate(0.0, -KEY_TURN),
        "+" | "=" => KeyAction::Zoom(KEY_ZOOM_IN),
        "-" => KeyAction::Zoom(KEY_ZOOM_OUT),
        "0" => KeyAction::Refit,
        _ => return None,
    })
}

/// The zoom factor for one wheel event. `delta_mode` is the DOM's: 0 pixels,
/// 1 lines, 2 pages.
pub fn wheel_factor(delta_y: f64, delta_mode: u32) -> f32 {
    let px = match delta_mode {
        1 => delta_y * WHEEL_LINE_PX,
        2 => delta_y * WHEEL_PAGE_PX,
        _ => delta_y,
    };
    if !px.is_finite() {
        return 1.0;
    }
    (px.clamp(-WHEEL_MAX_PX, WHEEL_MAX_PX) as f32 * WHEEL_ZOOM).exp()
}

/// The pointer dragging the view and its last position.
pub type Drag = Option<(i32, i32, i32)>;

/// A `pointermove` during `drag`: the drag afterwards, and how far to turn.
/// `buttons` is the DOM's bitmask. A move with the primary button up ends
/// the drag, so a release that never reached us as a `pointerup` (a system
/// gesture, a switch away) can't leave the model spinning under the cursor.
pub fn drag_move(drag: Drag, id: i32, buttons: u16, x: i32, y: i32) -> (Drag, Option<(i32, i32)>) {
    match drag {
        Some((d, ..)) if d != id => (drag, None),
        Some(_) if buttons & 1 == 0 => (None, None),
        Some((_, ox, oy)) => (Some((id, x, y)), Some((x - ox, y - oy))),
        None => (None, None),
    }
}

/// A projected pin, kept only if it lands on the `w` × `h` canvas.
pub fn visible_pin(p: Option<(f32, f32)>, w: f32, h: f32) -> Option<(f32, f32)> {
    p.filter(|&(x, y)| (0.0..=w).contains(&x) && (0.0..=h).contains(&y))
}

/// Inline style for pin `index` of `count`. Lower numbers stack on top, so
/// pin 1 stays clickable where pins overlap.
pub fn pin_style(at: Option<(f32, f32)>, count: usize, index: usize) -> String {
    match at {
        Some((x, y)) => format!(
            "left:{x:.1}px;top:{y:.1}px;z-index:{}",
            count.saturating_sub(index)
        ),
        None => "display:none".into(),
    }
}

/// A view decoded and ready to draw.
struct Loaded {
    name: String,
    meta: String,
    min: [f32; 3],
    max: [f32; 3],
    dots: Vec<f32>,
    edges: Vec<f32>,
    risks: Vec<Risk>,
}

impl Loaded {
    fn from_view(v: ModelView) -> Result<Loaded, String> {
        Ok(Loaded {
            meta: meta_line(v.triangle_count, v.bounds_min, v.bounds_max),
            dots: decode::decode_triples(&v.dots)?,
            edges: decode::decode_segments(&v.edges)?,
            name: v.name,
            min: v.bounds_min,
            max: v.bounds_max,
            risks: v.risks,
        })
    }
}

#[derive(Clone)]
enum Load {
    Idle,
    Loading,
    Ready(Arc<Loaded>),
    Failed(String),
}

/// The renderer and the model whose geometry it holds. The `Weak` keeps the
/// old allocation's address from being reused, so comparing pointers is safe.
struct Gpu {
    renderer: Renderer,
    shows: Weak<Loaded>,
}

/// Pin positions in CSS pixels relative to the stage; `None` is hidden.
type Pins = Vec<Option<(f32, f32)>>;

enum Frame {
    Failed(GlProblem),
    Drawn(Pins),
}

/// Draw `model` to `canvas`, making a renderer for the canvas or uploading
/// the model first if needed. Returns where the pins go.
fn render(
    slot: &mut Option<Gpu>,
    canvas: &HtmlCanvasElement,
    model: &Arc<Loaded>,
    orbit: &Orbit,
    mode: Mode,
) -> Frame {
    if slot.as_ref().is_none_or(|g| g.renderer.canvas() != canvas) {
        // A new canvas: let the old context go before taking another.
        *slot = None;
        let renderer = match Renderer::new(canvas) {
            Ok(r) => r,
            Err(problem) => return Frame::Failed(problem),
        };
        *slot = Some(Gpu {
            renderer,
            shows: Weak::new(),
        });
    }
    let Some(gpu) = slot.as_mut() else {
        return Frame::Failed(GlProblem::SetupFailed);
    };
    if gpu.shows.as_ptr() != Arc::as_ptr(model) {
        gpu.renderer.set_geometry(&model.dots, &model.edges);
        gpu.shows = Arc::downgrade(model);
    }

    let (w, h) = (canvas.client_width() as f32, canvas.client_height() as f32);
    if w <= 0.0 || h <= 0.0 {
        // Not laid out (detached or hidden): nothing to draw or pin.
        return Frame::Drawn(vec![None; model.risks.len()]);
    }
    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0) as f32;
    let colors = Colors {
        model: gl::css_rgb("--nd-text-primary"),
        background: gl::css_rgb("--nd-black"),
    };
    gpu.renderer
        .draw(&orbit.view_proj(w / h), mode, colors, dpr);

    // The canvas's border offsets its content box inside the stage.
    let (dx, dy) = (canvas.client_left() as f32, canvas.client_top() as f32);
    Frame::Drawn(
        model
            .risks
            .iter()
            .map(|r| {
                visible_pin(orbit.project(r.position, w, h), w, h).map(|(x, y)| (x + dx, y + dy))
            })
            .collect(),
    )
}

/// Page listeners that ask for a redraw: theme changes on `<html>` and
/// window resizes. Dropping this unhooks both.
struct PageHooks {
    observer: Option<web_sys::MutationObserver>,
    _on_theme: Closure<dyn FnMut()>,
    on_resize: Closure<dyn FnMut()>,
}

impl PageHooks {
    fn new(redraw: RwSignal<u32>) -> PageHooks {
        // Either can fire after unmount, before the drop below runs.
        let bump = move || {
            redraw.try_update(|n| *n = n.wrapping_add(1));
        };
        let on_theme = Closure::<dyn FnMut()>::new(bump);
        let on_resize = Closure::<dyn FnMut()>::new(bump);
        let window = web_sys::window();
        let observer = window
            .as_ref()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
            .and_then(|root| {
                let observer =
                    web_sys::MutationObserver::new(on_theme.as_ref().unchecked_ref()).ok()?;
                let init = web_sys::MutationObserverInit::new();
                init.set_attributes(true);
                init.set_attribute_filter(&js_sys::Array::of1(&"data-theme".into()));
                observer.observe_with_options(&root, &init).ok()?;
                Some(observer)
            });
        if let Some(w) = &window {
            let _ =
                w.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        }
        PageHooks {
            observer,
            _on_theme: on_theme,
            on_resize,
        }
    }
}

impl Drop for PageHooks {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            observer.disconnect();
        }
        if let Some(w) = web_sys::window() {
            let _ = w.remove_event_listener_with_callback(
                "resize",
                self.on_resize.as_ref().unchecked_ref(),
            );
        }
    }
}

/// The signals a loaded view's markup reads and writes. All `Copy`.
#[derive(Clone, Copy)]
struct Ctx {
    orbit: RwSignal<Orbit>,
    mode: RwSignal<Mode>,
    selected: RwSignal<Option<usize>>,
    pins: RwSignal<Pins>,
    gl_failed: RwSignal<Option<GlProblem>>,
    canvas: NodeRef<html::Canvas>,
    /// The pointer dragging the view and its last position.
    drag: StoredValue<Drag>,
}

/// A 3D view of the model at `path`, with its geometry risks. Renders nothing
/// while `path` is `None`. `hide_name` drops the file name from the header for
/// pages that already show it.
#[component]
pub fn ModelViewport(
    #[prop(into)] path: Signal<Option<String>>,
    #[prop(optional)] hide_name: bool,
) -> impl IntoView {
    let state = RwSignal::new(Load::Idle);
    let redraw = RwSignal::new(0u32);
    let cx = Ctx {
        orbit: RwSignal::new(Orbit::fit([0.0; 3], [1.0; 3])),
        mode: RwSignal::new(Mode::Dots),
        selected: RwSignal::new(None),
        pins: RwSignal::new(Vec::new()),
        gl_failed: RwSignal::new(None),
        canvas: NodeRef::new(),
        drag: StoredValue::new(None),
    };
    let gpu = StoredValue::new_local(None::<Gpu>);
    // Bumped per path, so a slow load for an old path cannot land late.
    let generation = StoredValue::new(0u64);

    let hooks = StoredValue::new_local(Some(PageHooks::new(redraw)));
    on_cleanup(move || {
        hooks.try_update_value(|h| h.take());
    });

    Effect::new(move |_| {
        let path = path.get();
        let this = generation.get_value().wrapping_add(1);
        generation.set_value(this);
        let Some(path) = path else {
            state.set(Load::Idle);
            return;
        };
        state.set(Load::Loading);
        spawn_local(async move {
            let result = types::load_view(&path).await.and_then(Loaded::from_view);
            // A newer path (or an unmount) wins.
            if generation.try_get_value() != Some(this) {
                return;
            }
            match result {
                Ok(model) => {
                    cx.orbit.try_set(Orbit::fit(model.min, model.max));
                    cx.selected.try_set(None);
                    cx.pins.try_set(Vec::new());
                    state.try_set(Load::Ready(Arc::new(model)));
                }
                Err(e) => {
                    state.try_set(Load::Failed(e));
                }
            }
        });
    });

    // Draw whenever the canvas, model, camera, mode, size or theme changes.
    Effect::new(move |_| {
        let canvas = cx.canvas.get();
        let model = state.with(|s| match s {
            Load::Ready(m) => Some(Arc::clone(m)),
            _ => None,
        });
        let orbit = cx.orbit.get();
        let mode = cx.mode.get();
        redraw.track();
        let (Some(canvas), Some(model)) = (canvas, model) else {
            return;
        };
        if cx.gl_failed.get_untracked().is_some() {
            return;
        }
        match gpu.try_update_value(|slot| render(slot, &canvas, &model, &orbit, mode)) {
            Some(Frame::Drawn(pins)) => {
                cx.pins.try_set(pins);
            }
            Some(Frame::Failed(problem)) => {
                cx.gl_failed.try_set(Some(problem));
            }
            None => {}
        }
    });

    move || match state.get() {
        Load::Idle => ().into_any(),
        Load::Loading => view! { <StateText kind=StateKind::Loading message="Model" /> }.into_any(),
        Load::Failed(e) => view! { <StateText kind=StateKind::Error message=e /> }.into_any(),
        Load::Ready(model) => loaded_view(model, cx, hide_name).into_any(),
    }
}

fn loaded_view(model: Arc<Loaded>, cx: Ctx, hide_name: bool) -> impl IntoView {
    let (min, max) = (model.min, model.max);
    let refit = move || {
        cx.orbit.try_set(Orbit::fit(min, max));
    };
    let turn = move |dyaw: f32, dpitch: f32| {
        cx.orbit.try_update(|o| o.rotate(dyaw, dpitch));
    };
    let zoom = move |factor: f32| {
        cx.orbit.try_update(|o| o.zoom(factor));
    };

    let on_pointerdown = move |e: web_sys::PointerEvent| {
        if e.button() != 0 {
            return;
        }
        if let Some(el) = e
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.set_pointer_capture(e.pointer_id());
        }
        cx.drag
            .try_set_value(Some((e.pointer_id(), e.client_x(), e.client_y())));
    };
    let on_pointermove = move |e: web_sys::PointerEvent| {
        let Some(drag) = cx.drag.try_get_value() else {
            return;
        };
        let (next, delta) = drag_move(
            drag,
            e.pointer_id(),
            e.buttons(),
            e.client_x(),
            e.client_y(),
        );
        if next != drag {
            cx.drag.try_set_value(next);
        }
        if let Some((dx, dy)) = delta {
            // Dragging right turns the model right; dragging down tips it toward you.
            turn(-(dx as f32) * DRAG_TURN, dy as f32 * DRAG_TURN);
        }
    };
    let on_pointerup = move |e: web_sys::PointerEvent| {
        if matches!(cx.drag.try_get_value(), Some(Some((id, ..))) if id == e.pointer_id()) {
            cx.drag.try_set_value(None);
        }
    };
    let on_wheel = move |e: web_sys::WheelEvent| {
        e.prevent_default();
        zoom(wheel_factor(e.delta_y(), e.delta_mode()));
    };
    let on_keydown = move |e: web_sys::KeyboardEvent| {
        // Leave the app's and the system's shortcuts alone.
        if e.ctrl_key() || e.meta_key() || e.alt_key() {
            return;
        }
        let Some(action) = key_action(&e.key()) else {
            return;
        };
        e.prevent_default();
        match action {
            KeyAction::Rotate(dyaw, dpitch) => turn(dyaw, dpitch),
            KeyAction::Zoom(factor) => zoom(factor),
            KeyAction::Refit => refit(),
        }
    };

    let label = canvas_label(&model.name);
    let count = model.risks.len();
    let pin_model = Arc::clone(&model);
    let stage = move || {
        if let Some(problem) = cx.gl_failed.get() {
            return view! { <StateText kind=StateKind::Error message=problem_message(problem) /> }
                .into_any();
        }
        let pins = pin_model
            .risks
            .iter()
            .enumerate()
            .map(|(i, risk)| {
                view! {
                    <button type="button" class="vp-pin"
                        aria-label=format!("Risk {}: {}", i + 1, risk.message)
                        aria-pressed=move || if cx.selected.get() == Some(i) { "true" } else { "false" }
                        style=move || pin_style(cx.pins.with(|p| p.get(i).copied().flatten()), count, i)
                        on:click=move |_| {
                            cx.selected.try_update(|s| *s = if *s == Some(i) { None } else { Some(i) });
                        }>
                        {i + 1}
                    </button>
                }
            })
            .collect_view();
        view! {
            <canvas class="vp-canvas" tabindex="0" role="img" aria-label=label.clone()
                data-yaw=move || format!("{:.3}", cx.orbit.get().yaw)
                node_ref=cx.canvas
                on:pointerdown=on_pointerdown
                on:pointermove=on_pointermove
                on:pointerup=on_pointerup
                on:pointercancel=on_pointerup
                on:lostpointercapture=on_pointerup
                on:wheel=on_wheel
                on:dblclick=move |_| refit()
                on:keydown=on_keydown>
            </canvas>
            {pins}
        }
        .into_any()
    };

    let risks = if model.risks.is_empty() {
        view! { <StateText kind=StateKind::Empty message="No risks found in this orientation" /> }
            .into_any()
    } else {
        let rows = model
            .risks
            .iter()
            .enumerate()
            .map(|(i, risk)| {
                view! {
                    <li class="vp-risk" data-selected=move || (cx.selected.get() == Some(i)).then_some("")>
                        <span class="vp-risk-num nd-mono">{risk_number(i)}</span>
                        <span class="nd-label">{kind_label(risk.kind)}</span>
                        <span class="vp-risk-msg">{risk.message.clone()}</span>
                    </li>
                }
            })
            .collect_view();
        view! { <ol class="vp-risks">{rows}</ol> }.into_any()
    };

    let mode_button = move |mode: Mode, text: &'static str| {
        view! {
            <button type="button" class="vp-mode nd-tab"
                aria-pressed=move || if cx.mode.get() == mode { "true" } else { "false" }
                on:click=move |_| {
                    cx.mode.try_set(mode);
                }>
                {text}
            </button>
        }
    };

    view! {
        <section class="vp" data-mode=move || cx.mode.get().key()>
            <header class="vp-head">
                {(!hide_name).then(|| view! {
                    <span class="nd-label vp-name">{model.name.clone()}</span>
                })}
                <span class="nd-label vp-meta">{model.meta.clone()}</span>
                <div class="vp-modes" role="group" aria-label="View">
                    {mode_button(Mode::Dots, "Dots")}
                    {mode_button(Mode::Wireframe, "Wireframe")}
                </div>
            </header>
            <div class="vp-stage">{stage}</div>
            <p class="nd-label vp-note">"As oriented in the file — Bambu Studio may rotate it when slicing."</p>
            {risks}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_context_and_a_failed_setup_read_differently() {
        assert_eq!(
            problem_message(GlProblem::NoWebGl),
            "3D preview needs WebGL2"
        );
        assert_eq!(
            problem_message(GlProblem::SetupFailed),
            "3D preview couldn't start"
        );
    }

    #[test]
    fn kinds_read_as_plain_words() {
        assert_eq!(kind_label(RiskKind::Overhang), "Overhang");
        assert_eq!(kind_label(RiskKind::SmallBedContact), "Small bed contact");
        assert_eq!(kind_label(RiskKind::TallThin), "Tall and thin");
    }

    #[test]
    fn risk_numbers_are_two_digits_from_one() {
        assert_eq!(risk_number(0), "01");
        assert_eq!(risk_number(8), "09");
        assert_eq!(risk_number(9), "10");
        assert_eq!(risk_number(99), "100");
    }

    #[test]
    fn the_meta_line_gives_count_and_size() {
        assert_eq!(
            meta_line(12, [0., 0., 0.], [10., 10., 10.]),
            "12 triangles · 10 × 10 × 10 mm"
        );
        // Under 100 mm keeps one decimal (dropping a trailing .0); 100 mm and
        // over rounds to whole millimetres.
        assert_eq!(
            meta_line(1, [-1., 2., 0.], [11.34, 7.26, 150.4]),
            "1 triangle · 12.3 × 5.3 × 150 mm"
        );
        assert_eq!(
            meta_line(1_234_567, [0.; 3], [0.04, 256., 99.]),
            "1,234,567 triangles · 0 × 256 × 99 mm"
        );
    }

    #[test]
    fn a_broken_size_does_not_print_nan() {
        let line = meta_line(0, [f32::NAN, 0., 5.], [1., f32::INFINITY, 1.]);
        assert_eq!(line, "0 triangles · 0 × 0 × 0 mm");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(5_000_000), "5,000,000");
    }

    #[test]
    fn the_canvas_label_names_the_model_and_the_controls() {
        assert_eq!(
            canvas_label("cube.stl"),
            "3D view of cube.stl. Drag or use arrow keys to rotate, + and − to zoom, 0 to reset."
        );
    }

    #[test]
    fn keys_map_to_camera_moves() {
        assert_eq!(key_action("ArrowLeft"), Some(KeyAction::Rotate(0.1, 0.0)));
        assert_eq!(key_action("ArrowRight"), Some(KeyAction::Rotate(-0.1, 0.0)));
        assert_eq!(key_action("ArrowUp"), Some(KeyAction::Rotate(0.0, 0.1)));
        assert_eq!(key_action("ArrowDown"), Some(KeyAction::Rotate(0.0, -0.1)));
        assert_eq!(key_action("+"), Some(KeyAction::Zoom(0.9)));
        assert_eq!(key_action("="), Some(KeyAction::Zoom(0.9)));
        assert_eq!(key_action("-"), Some(KeyAction::Zoom(1.1)));
        assert_eq!(key_action("0"), Some(KeyAction::Refit));
        assert_eq!(key_action("a"), None);
        assert_eq!(key_action("Tab"), None);
    }

    #[test]
    fn the_wheel_zooms_exponentially() {
        assert_eq!(wheel_factor(0.0, 0), 1.0);
        assert!((wheel_factor(100.0, 0) - 0.1f32.exp()).abs() < 1e-6);
        assert!(wheel_factor(-100.0, 0) < 1.0);
        // Line and page deltas scale to roughly pixels.
        assert!((wheel_factor(3.0, 1) - 0.048f32.exp()).abs() < 1e-6);
        assert!((wheel_factor(1.0, 2) - 0.8f32.exp()).abs() < 1e-6);
        assert_eq!(wheel_factor(f64::NAN, 0), 1.0);
        // One huge event is capped rather than ignored.
        assert!((wheel_factor(1e12, 0) - 1f32.exp()).abs() < 1e-5);
    }

    #[test]
    fn a_drag_turns_by_the_distance_moved() {
        let drag = Some((1, 100, 100));
        assert_eq!(
            drag_move(drag, 1, 1, 140, 90),
            (Some((1, 140, 90)), Some((40, -10)))
        );
        // Another pointer leaves the drag alone.
        assert_eq!(drag_move(drag, 2, 1, 0, 0), (drag, None));
        // No drag, no turn.
        assert_eq!(drag_move(None, 1, 1, 0, 0), (None, None));
    }

    #[test]
    fn a_move_with_the_button_up_ends_the_drag() {
        let drag = Some((1, 100, 100));
        // The release was missed: buttons is 0 (or only the right button).
        assert_eq!(drag_move(drag, 1, 0, 140, 90), (None, None));
        assert_eq!(drag_move(drag, 1, 2, 140, 90), (None, None));
        // Primary plus another button still drags.
        assert_eq!(
            drag_move(drag, 1, 3, 101, 100),
            (Some((1, 101, 100)), Some((1, 0)))
        );
    }

    #[test]
    fn pins_off_the_canvas_are_culled() {
        assert_eq!(visible_pin(Some((10., 20.)), 400., 300.), Some((10., 20.)));
        assert_eq!(visible_pin(Some((0., 300.)), 400., 300.), Some((0., 300.)));
        assert_eq!(visible_pin(Some((-1., 20.)), 400., 300.), None);
        assert_eq!(visible_pin(Some((10., 301.)), 400., 300.), None);
        assert_eq!(visible_pin(Some((401., 0.)), 400., 300.), None);
        assert_eq!(visible_pin(None, 400., 300.), None);
    }

    #[test]
    fn pin_styles_place_or_hide_and_stack_earlier_pins_on_top() {
        assert_eq!(
            pin_style(Some((10.26, 20.0)), 2, 0),
            "left:10.3px;top:20.0px;z-index:2"
        );
        assert_eq!(
            pin_style(Some((1.0, 2.0)), 2, 1),
            "left:1.0px;top:2.0px;z-index:1"
        );
        assert_eq!(pin_style(None, 2, 0), "display:none");
    }
}
