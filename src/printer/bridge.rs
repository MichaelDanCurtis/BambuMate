//! Invoke wrappers for the `printer_*` commands. Tauri expects camelCase
//! argument names.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{DiscoveryReport, PrinterConfigView, PrinterView, TestOutcome};

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

pub async fn discover() -> Result<DiscoveryReport, String> {
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
