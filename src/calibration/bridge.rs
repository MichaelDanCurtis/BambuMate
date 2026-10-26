//! Invoke wrappers for the `calibration_*` commands. Tauri expects
//! camelCase argument names. A `None` goes out as `undefined`, which Tauri
//! reads as a missing optional argument.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{
    Defaults, LinkCandidates, PreviewView, Reading, RunSummary, RunView, SaveAnswer, Setup,
    StepKey, SuggestAnswer, TestKind,
};

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
struct StartArgs<'a> {
    slot: Option<&'a str>,
    preset: Option<&'a str>,
    run: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariantArgs<'a> {
    run_id: i64,
    variant: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestArgs {
    run_id: i64,
    test: TestKind,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewArgs<'a> {
    run_id: i64,
    test: TestKind,
    setup: &'a Setup,
    reading: Option<&'a Reading>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveArgs<'a> {
    run_id: i64,
    test: TestKind,
    setup: &'a Setup,
    reading: &'a Reading,
    expected_old: &'a str,
    photo: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordArgs<'a> {
    run_id: i64,
    setup: &'a Setup,
    reading: &'a Reading,
    photo: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultArgs {
    result_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkipArgs {
    run_id: i64,
    step: StepKey,
    skipped: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SuggestArgs<'a> {
    run_id: i64,
    test: TestKind,
    setup: &'a Setup,
    photo_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageArgs<'a> {
    file_name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkArgs {
    result_id: i64,
    journal_id: i64,
}

pub async fn runs() -> Result<Vec<RunSummary>, String> {
    call("calibration_runs", &NoArgs {}).await
}

pub async fn start(
    slot: Option<&str>,
    preset: Option<&str>,
    run: Option<i64>,
) -> Result<RunView, String> {
    call("calibration_start", &StartArgs { slot, preset, run }).await
}

pub async fn set_variant(run_id: i64, variant: &str) -> Result<RunView, String> {
    call("calibration_set_variant", &VariantArgs { run_id, variant }).await
}

pub async fn defaults(run_id: i64, test: TestKind) -> Result<Defaults, String> {
    call("calibration_defaults", &TestArgs { run_id, test }).await
}

pub async fn preview(
    run_id: i64,
    test: TestKind,
    setup: &Setup,
    reading: Option<&Reading>,
) -> Result<PreviewView, String> {
    let args = PreviewArgs {
        run_id,
        test,
        setup,
        reading,
    };
    call("calibration_preview", &args).await
}

pub async fn save(
    run_id: i64,
    test: TestKind,
    setup: &Setup,
    reading: &Reading,
    expected_old: &str,
    photo: Option<&str>,
) -> Result<SaveAnswer, String> {
    let args = SaveArgs {
        run_id,
        test,
        setup,
        reading,
        expected_old,
        photo,
    };
    call("calibration_save", &args).await
}

pub async fn record(
    run_id: i64,
    setup: &Setup,
    reading: &Reading,
    photo: Option<&str>,
) -> Result<RunView, String> {
    let args = RecordArgs {
        run_id,
        setup,
        reading,
        photo,
    };
    call("calibration_record", &args).await
}

pub async fn undo(result_id: i64) -> Result<RunView, String> {
    call("calibration_undo", &ResultArgs { result_id }).await
}

pub async fn skip(run_id: i64, step: StepKey, skipped: bool) -> Result<RunView, String> {
    call(
        "calibration_skip",
        &SkipArgs {
            run_id,
            step,
            skipped,
        },
    )
    .await
}

/// The base64 is moved into the JS argument and dropped before the await.
pub async fn suggest(
    run_id: i64,
    test: TestKind,
    setup: &Setup,
    photo_base64: String,
) -> Result<SuggestAnswer, String> {
    let args = SuggestArgs {
        run_id,
        test,
        setup,
        photo_base64,
    };
    call("calibration_suggest", &args).await
}

pub async fn image(file_name: &str) -> Result<String, String> {
    call("calibration_image", &ImageArgs { file_name }).await
}

pub async fn link_candidates(result_id: i64) -> Result<LinkCandidates, String> {
    call("calibration_link_candidates", &ResultArgs { result_id }).await
}

pub async fn link_print(result_id: i64, journal_id: i64) -> Result<RunView, String> {
    call(
        "calibration_link_print",
        &LinkArgs {
            result_id,
            journal_id,
        },
    )
    .await
}
