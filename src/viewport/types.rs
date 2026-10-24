//! Serde mirrors of the backend's `ModelView`, and the `model_view` call.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    Overhang,
    SmallBedContact,
    TallThin,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Risk {
    pub kind: RiskKind,
    pub position: [f32; 3],
    pub value: f32,
    pub message: String,
}

/// `dots`, `dot_normals` and `edges` are base64 little-endian `f32` buffers;
/// see `decode`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ModelView {
    pub name: String,
    pub triangle_count: usize,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub dots: String,
    pub dot_normals: String,
    pub edges: String,
    pub risks: Vec<Risk>,
}

#[derive(Serialize)]
struct ViewArgs<'a> {
    path: &'a str,
}

/// Read a model file through the backend. Failures arrive as plain text.
pub async fn load_view(path: &str) -> Result<ModelView, String> {
    call("model_view", &ViewArgs { path }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backend_view_deserializes() {
        let json = r#"{"name":"cube.stl","triangle_count":12,
            "bounds_min":[0,0,0],"bounds_max":[1,2,3],
            "dots":"","dot_normals":"","edges":"",
            "risks":[{"kind":"small_bed_contact","position":[1,2,3],"value":4.5,"message":"may"},
                     {"kind":"overhang","position":[0,0,0],"value":1,"message":"m"},
                     {"kind":"tall_thin","position":[0,0,0],"value":6,"message":"m"}]}"#;
        let v: ModelView = serde_json::from_str(json).unwrap();
        assert_eq!(v.triangle_count, 12);
        assert_eq!(v.bounds_max, [1.0, 2.0, 3.0]);
        let kinds: Vec<_> = v.risks.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                RiskKind::SmallBedContact,
                RiskKind::Overhang,
                RiskKind::TallThin
            ]
        );
    }
}
