//! Invoke wrappers for the advisor commands.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{Advice, Need};

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
struct InterpretArgs<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdviseArgs<'a> {
    needs: &'a [Need],
    model_path: Option<&'a str>,
}

#[derive(Serialize)]
struct BrandsArgs<'a> {
    brands: &'a [String],
}

#[derive(Serialize)]
struct NoArgs {}

pub async fn advise_interpret(text: &str) -> Result<Vec<Need>, String> {
    call("advise_interpret", &InterpretArgs { text }).await
}

pub async fn advise(needs: &[Need], model_path: Option<&str>) -> Result<Advice, String> {
    call("advise", &AdviseArgs { needs, model_path }).await
}

pub async fn get_preferred_brands() -> Result<Vec<String>, String> {
    call("get_preferred_brands", &NoArgs {}).await
}

pub async fn set_preferred_brands(brands: &[String]) -> Result<Vec<String>, String> {
    call("set_preferred_brands", &BrandsArgs { brands }).await
}
