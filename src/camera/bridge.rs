//! Invoke wrappers for the `camera_*` commands, and the live watch.

use std::cell::Cell;
use std::rc::Rc;

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

const NOT_OPENED: &str = "The camera couldn't be opened.";

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

/// A live watch: the backend's id and its Channel. Dropping it stops the
/// messages reaching the callback; [`unwatch`] tells the backend.
///
/// The callback's `Closure` is leaked rather than kept here: the Channel
/// may still deliver a message already in flight after the watch is
/// dropped, and calling a dropped `Closure` throws. A shared flag, set on
/// drop, turns those late calls into no-ops.
pub struct Watch {
    pub id: u64,
    channel: JsValue,
    gone: Rc<Cell<bool>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.gone.set(true);
        shim::release(&self.channel);
    }
}

/// Starts watching; `on_bytes` gets each message, from the first (which
/// may arrive before this resolves) until the `Watch` is dropped.
pub async fn watch(
    mut on_bytes: impl FnMut(js_sys::Uint8Array) + 'static,
) -> Result<Watch, String> {
    let gone = Rc::new(Cell::new(false));
    let alive = gone.clone();
    let closure = Closure::<dyn FnMut(js_sys::Uint8Array)>::new(move |b| {
        if !alive.get() {
            on_bytes(b);
        }
    });
    let started = shim::watch(&closure);
    closure.forget();
    let promise = started.map_err(|_| NOT_OPENED.to_string())?;
    let answer = match JsFuture::from(promise).await {
        Ok(a) => a,
        Err(e) => {
            gone.set(true);
            return Err(e
                .as_string()
                .or_else(|| {
                    js_sys::Reflect::get(&e, &"message".into())
                        .ok()
                        .and_then(|m| m.as_string())
                })
                .unwrap_or_else(|| NOT_OPENED.into()));
        }
    };
    let get = |k: &str| js_sys::Reflect::get(&answer, &k.into()).ok();
    let Some(id) = get("id").and_then(|v| v.as_f64()) else {
        gone.set(true);
        return Err(NOT_OPENED.into());
    };
    Ok(Watch {
        id: id as u64,
        channel: get("channel").unwrap_or(JsValue::UNDEFINED),
        gone,
    })
}
