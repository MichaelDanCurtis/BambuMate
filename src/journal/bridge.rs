//! Invoke wrappers for the `journal_*` commands. Tauri expects camelCase
//! argument names.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{EntryUpdate, JournalFilter, MonthSummary, PrintEntry};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    call_with(cmd, args).await
}

async fn call_with<R: DeserializeOwned>(cmd: &str, args: JsValue) -> Result<R, String> {
    let out = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{cmd} failed")))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct NoArgs {}

#[derive(Serialize)]
struct ListArgs<'a> {
    filter: &'a JournalFilter,
}

#[derive(Serialize)]
struct IdArgs {
    id: i64,
}

#[derive(Serialize)]
struct UpdateArgs<'a> {
    id: i64,
    update: &'a EntryUpdate,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddPhotoArgs {
    id: i64,
    data_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhotoArgs {
    id: i64,
    photo_id: Option<i64>,
}

pub async fn list(filter: &JournalFilter) -> Result<Vec<PrintEntry>, String> {
    call("journal_list", &ListArgs { filter }).await
}

pub async fn get(id: i64) -> Result<Option<PrintEntry>, String> {
    call("journal_get", &IdArgs { id }).await
}

pub async fn update(id: i64, update: &EntryUpdate) -> Result<PrintEntry, String> {
    call("journal_update", &UpdateArgs { id, update }).await
}

/// The base64 is copied into the JS argument and dropped before the
/// await, so up to 33 MB isn't held in wasm memory for the whole round trip.
pub async fn add_photo(id: i64, data_base64: String) -> Result<PrintEntry, String> {
    let args = serde_wasm_bindgen::to_value(&AddPhotoArgs { id, data_base64 })
        .map_err(|e| e.to_string())?;
    call_with("journal_add_photo", args).await
}

pub async fn remove_photo(id: i64, photo_id: i64) -> Result<PrintEntry, String> {
    call(
        "journal_remove_photo",
        &PhotoArgs {
            id,
            photo_id: Some(photo_id),
        },
    )
    .await
}

pub async fn delete(id: i64) -> Result<(), String> {
    call("journal_delete", &IdArgs { id }).await
}

pub async fn month_summary() -> Result<MonthSummary, String> {
    call("journal_month_summary", &NoArgs {}).await
}

pub async fn last() -> Result<Option<PrintEntry>, String> {
    call("journal_last", &NoArgs {}).await
}

/// A photo, or the plate thumbnail when `photo_id` is `None`, as a `data:`
/// URL. Any `Err` means the file is missing or unreadable: the page shows
/// "Photo unavailable".
pub async fn image(id: i64, photo_id: Option<i64>) -> Result<String, String> {
    call("journal_image", &PhotoArgs { id, photo_id }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::types::{EntryUpdate, VerdictChoice};

    // Tauri maps camelCase argument names to the command's snake_case
    // parameters; a wrong name is silently read as missing.
    #[test]
    fn argument_names_are_the_ones_the_commands_take() {
        assert_eq!(
            json(&AddPhotoArgs {
                id: 3,
                data_base64: "QUJD".into()
            }),
            r#"{"id":3,"dataBase64":"QUJD"}"#
        );
        assert_eq!(
            json(&PhotoArgs {
                id: 3,
                photo_id: Some(7)
            }),
            r#"{"id":3,"photoId":7}"#
        );
        let update = EntryUpdate {
            verdict: Some(VerdictChoice::Good),
            ..Default::default()
        };
        assert_eq!(
            json(&UpdateArgs {
                id: 3,
                update: &update
            }),
            r#"{"id":3,"update":{"verdict":"good"}}"#
        );
        assert_eq!(json(&IdArgs { id: 3 }), r#"{"id":3}"#);
        assert_eq!(json(&NoArgs {}), "{}");
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap()
    }
}
