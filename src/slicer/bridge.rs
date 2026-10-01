//! Invoke wrappers for the `slicer_*` commands.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{JobView, PresetLists, SlicerSettings, SlicerSettingsView, SlicerStatus};

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
struct NoArgs {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobArgs {
    job_id: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThumbArgs {
    job_id: u64,
    plate: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PresetArgs {
    printer: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SliceArgs {
    model_path: String,
    printer: String,
    process: String,
    filament: String,
    bed_type: Option<String>,
}

#[derive(Serialize)]
struct SettingsArgs {
    settings: SlicerSettings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageArgs {
    file_name: String,
    data_base64: String,
}

pub async fn status() -> Result<SlicerStatus, String> {
    call("slicer_status", &NoArgs {}).await
}

pub async fn presets(printer: Option<String>) -> Result<PresetLists, String> {
    call("slicer_presets", &PresetArgs { printer }).await
}

pub async fn get_settings() -> Result<SlicerSettingsView, String> {
    call("slicer_get_settings", &NoArgs {}).await
}

pub async fn set_settings(settings: SlicerSettings) -> Result<SlicerSettingsView, String> {
    call("slicer_set_settings", &SettingsArgs { settings }).await
}

pub async fn slice(
    model_path: String,
    printer: String,
    process: String,
    filament: String,
    bed_type: Option<String>,
) -> Result<JobView, String> {
    call(
        "slicer_slice",
        &SliceArgs {
            model_path,
            printer,
            process,
            filament,
            bed_type,
        },
    )
    .await
}

pub async fn cancel(job_id: u64) -> Result<bool, String> {
    call("slicer_cancel", &JobArgs { job_id }).await
}

pub async fn jobs() -> Result<Vec<JobView>, String> {
    call("slicer_jobs", &NoArgs {}).await
}

pub async fn thumbnail(job_id: u64, plate: u32) -> Result<Option<String>, String> {
    call("slicer_thumbnail", &ThumbArgs { job_id, plate }).await
}

pub async fn open_in_bambu_studio(job_id: u64) -> Result<serde_json::Value, String> {
    call("slicer_open_in_bambu_studio", &JobArgs { job_id }).await
}

pub async fn pick_model() -> Result<Option<String>, String> {
    call("slicer_pick_model", &NoArgs {}).await
}

pub async fn stage_model(file_name: String, data_base64: String) -> Result<String, String> {
    call(
        "slicer_stage_model",
        &StageArgs {
            file_name,
            data_base64,
        },
    )
    .await
}
