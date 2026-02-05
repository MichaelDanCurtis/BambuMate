use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

// -- Arg structs for serialization --

#[derive(Serialize)]
struct SetApiKeyArgs {
    service: String,
    key: String,
}

#[derive(Serialize)]
struct GetApiKeyArgs {
    service: String,
}

#[derive(Serialize)]
struct DeleteApiKeyArgs {
    service: String,
}

#[derive(Serialize)]
struct GetPreferenceArgs {
    key: String,
}

#[derive(Serialize)]
struct SetPreferenceArgs {
    key: String,
    value: String,
}

// -- Health report matching backend struct --

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthReport {
    pub bambu_studio_installed: bool,
    pub bambu_studio_path: Option<String>,
    pub profile_dir_accessible: bool,
    pub profile_dir_path: Option<String>,
    pub claude_api_key_set: bool,
    pub openai_api_key_set: bool,
    pub kimi_api_key_set: bool,
    pub openrouter_api_key_set: bool,
}

// -- Typed invoke helpers --

pub async fn set_api_key(service: &str, key: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&SetApiKeyArgs {
        service: service.to_string(),
        key: key.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("set_api_key", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

pub async fn get_api_key(service: &str) -> Result<Option<String>, String> {
    let args = serde_wasm_bindgen::to_value(&GetApiKeyArgs {
        service: service.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("get_api_key", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn delete_api_key(service: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&DeleteApiKeyArgs {
        service: service.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("delete_api_key", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

pub async fn run_health_check() -> Result<HealthReport, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("run_health_check", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn get_preference(key: &str) -> Result<Option<String>, String> {
    let args = serde_wasm_bindgen::to_value(&GetPreferenceArgs {
        key: key.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("get_preference", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn set_preference(key: &str, value: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&SetPreferenceArgs {
        key: key.to_string(),
        value: value.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("set_preference", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}
