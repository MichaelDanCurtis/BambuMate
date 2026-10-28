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
export function cameraRelease(channel) {
  if (channel && typeof channel === "object") channel.onmessage = () => {};
}
// Chunks from a keyframe on are held while `configure` waits for the
// browser, then fed: the stream's first keyframe comes right behind its
// config. `token` changes on every configure and close, so a configure
// overtaken while it waited makes no decoder.
const HOLD_MAX = 120;
export class CameraDecoder {
  constructor(canvas, onError) {
    this.canvas = canvas; this.onError = onError; this.dec = null; this.ts = 0; this.needKey = true;
    this.token = 0; this.held = null;
  }
  async configure(codec, description) {
    this.close();
    const token = ++this.token;
    this.held = [];
    const config = { codec, description: description.slice(), optimizeForLatency: true };
    let ok = false;
    try { ok = !!(await VideoDecoder.isConfigSupported(config)).supported; } catch (_) { ok = false; }
    if (token !== this.token) return ok;
    const held = this.held; this.held = null;
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
    for (const [data, key] of held) this.decode(data, key);
    return true;
  }
  decode(data, key) {
    if (this.held) {
      if (key) this.held = [];
      if ((key || this.held.length) && this.held.length < HOLD_MAX) this.held.push([data, key]);
      return;
    }
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
  close() {
    this.token++; this.held = null;
    const d = this.dec; this.dec = null;
    if (d && d.state !== "closed") { try { d.close(); } catch (_) {} }
  }
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
    /// Lets go of the channel's callback: later messages reach nothing.
    #[wasm_bindgen(js_name = cameraRelease)]
    pub fn release(channel: &JsValue);

    #[derive(Clone)]
    pub type CameraDecoder;
    #[wasm_bindgen(constructor)]
    pub fn new(
        canvas: &web_sys::HtmlCanvasElement,
        on_error: &Closure<dyn FnMut(String)>,
    ) -> CameraDecoder;
    /// Resolves to whether the browser can decode `codec`. Chunks given to
    /// `decode` meanwhile are held from a keyframe on, then fed.
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
