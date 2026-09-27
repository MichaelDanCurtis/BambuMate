//! Invoke wrappers and event listeners for the agent panel.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{AgentModel, AgentSettings, AppState, AuthMode, Provider, Readiness};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn tauri_listen(
        event: &str,
        handler: &Closure<dyn FnMut(JsValue)>,
    ) -> Result<JsValue, JsValue>;
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let out = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{cmd} failed")))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct P {
    provider: Provider,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartArgs {
    provider: Provider,
    model: Option<String>,
    effort: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendArgs {
    session_id: String,
    text: String,
    images: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionArgs {
    session_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnswerArgs {
    ask_id: String,
    answers: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RewindArgs {
    session_id: String,
    seq: u32,
}

#[derive(Serialize)]
struct StateArgs<'a> {
    state: &'a AppState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageArgs {
    filename: String,
    data_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsArgs {
    full_access: bool,
    claude_auth_mode: AuthMode,
}

#[derive(Serialize)]
struct Empty {}

pub async fn readiness(provider: Provider) -> Result<Readiness, String> {
    call("agent_readiness", &P { provider }).await
}
pub async fn models(provider: Provider) -> Result<Vec<AgentModel>, String> {
    call("agent_models", &P { provider }).await
}
pub async fn login(provider: Provider) -> Result<Option<String>, String> {
    call("agent_login", &P { provider }).await
}
pub async fn start(
    provider: Provider,
    model: Option<String>,
    effort: Option<String>,
) -> Result<String, String> {
    call(
        "agent_start",
        &StartArgs {
            provider,
            model,
            effort,
        },
    )
    .await
}
pub async fn send(session_id: String, text: String, images: Vec<String>) -> Result<u32, String> {
    call(
        "agent_send",
        &SendArgs {
            session_id,
            text,
            images,
        },
    )
    .await
}
pub async fn interrupt(session_id: String) -> Result<(), String> {
    call("agent_interrupt", &SessionArgs { session_id }).await
}
pub async fn answer(ask_id: String, answers: Vec<String>) -> Result<(), String> {
    call("agent_answer", &AnswerArgs { ask_id, answers }).await
}
pub async fn rewind(session_id: String, seq: u32) -> Result<bool, String> {
    call("agent_rewind", &RewindArgs { session_id, seq }).await
}
pub async fn set_app_state(state: &AppState) {
    let _: Result<(), String> = call("agent_set_app_state", &StateArgs { state }).await;
}
pub async fn stage_image(filename: String, data_base64: String) -> Result<String, String> {
    call(
        "agent_stage_image",
        &StageArgs {
            filename,
            data_base64,
        },
    )
    .await
}
pub async fn get_settings() -> Result<AgentSettings, String> {
    call("agent_get_settings", &Empty {}).await
}
pub async fn set_settings(
    full_access: bool,
    claude_auth_mode: AuthMode,
) -> Result<AgentSettings, String> {
    call(
        "agent_set_settings",
        &SettingsArgs {
            full_access,
            claude_auth_mode,
        },
    )
    .await
}

/// Subscribe to a Tauri event for the lifetime of the app.
pub fn listen<T: DeserializeOwned + 'static>(event: &'static str, mut on: impl FnMut(T) + 'static) {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |msg: JsValue| {
        let payload =
            js_sys::Reflect::get(&msg, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value::<T>(payload) {
            Ok(v) => on(v),
            Err(e) => web_sys::console::warn_1(&format!("{event}: {e}").into()),
        }
    });
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = tauri_listen(event, &closure).await {
            web_sys::console::warn_1(&e);
        }
        closure.forget();
    });
}
