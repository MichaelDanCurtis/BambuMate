//! Production ToolHost: adapts bm_* tools onto BambuMate's existing commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::tools::ToolHost;
use super::types::{AppState, UiCommand};
use crate::commands::profile::GenerateResult;
use crate::profile::BambuPaths;
use crate::scraper::types::FilamentSpecs;

pub struct TauriToolHost {
    app: AppHandle,
    state: Mutex<AppState>,
    staged: Mutex<HashMap<String, GenerateResult>>,
}

pub(crate) fn merge_specs(partial: Value) -> Result<FilamentSpecs, String> {
    let Value::Object(fields) = partial else {
        return Err("'specs' must be an object".into());
    };
    let mut base = serde_json::to_value(FilamentSpecs::default()).map_err(|e| e.to_string())?;
    if let Value::Object(b) = &mut base {
        for (k, v) in fields {
            b.insert(k, v);
        }
    }
    serde_json::from_value(base).map_err(|e| format!("invalid specs: {e}"))
}

fn to_json<T: serde::Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

impl TauriToolHost {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            state: Mutex::new(AppState::default()),
            staged: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_app_state(&self, s: AppState) {
        *self.state.lock().unwrap() = s;
    }

    pub fn set_photo(&self, path: String) {
        self.state.lock().unwrap().photo_path = Some(path);
    }
}

#[async_trait]
impl ToolHost for TauriToolHost {
    fn user_filament_dir(&self) -> Result<PathBuf, String> {
        BambuPaths::detect()
            .map_err(|e| format!("Bambu Studio not found: {e}"))?
            .user_filament_dir()
            .ok_or_else(|| "Bambu Studio user filament folder not found".to_string())
    }

    fn system_filament_dir(&self) -> Option<PathBuf> {
        BambuPaths::detect().ok().map(|p| p.system_filament_dir())
    }

    fn app_state(&self) -> AppState {
        self.state.lock().unwrap().clone()
    }

    fn emit_ui(&self, cmd: UiCommand) {
        let _ = self.app.emit("agent://ui", cmd);
    }

    fn bambu_studio_running(&self) -> bool {
        crate::profile::is_bambu_studio_running()
    }

    async fn search_filament(&self, name: &str) -> Result<Value, String> {
        to_json(crate::commands::scraper::search_filament(self.app.clone(), name.to_string()).await)
    }

    async fn catalog_search(&self, query: &str, limit: usize) -> Result<Value, String> {
        to_json(
            crate::commands::scraper::search_catalog(
                self.app.clone(),
                query.to_string(),
                Some(limit),
            )
            .await,
        )
    }

    async fn generate_profile(
        &self,
        specs: Value,
        target_printer: Option<String>,
        base: Option<String>,
    ) -> Result<Value, String> {
        let specs = merge_specs(specs)?;
        let result = crate::commands::profile::generate_profile_from_specs(
            specs,
            target_printer,
            base,
            None,
        )
        .await?;
        let staged_id = uuid::Uuid::new_v4().to_string();
        let summary = json!({
            "staged_id": staged_id,
            "profile_name": result.profile_name,
            "filename": result.filename,
            "base_profile_used": result.base_profile_used,
            "diffs": result.diffs,
            "warnings": result.warnings,
            "bambu_studio_running": result.bambu_studio_running,
        });
        self.staged.lock().unwrap().insert(staged_id, result);
        Ok(summary)
    }

    async fn install_staged(&self, staged_id: &str, force: bool) -> Result<Value, String> {
        let staged = self
            .staged
            .lock()
            .unwrap()
            .remove(staged_id)
            .ok_or_else(|| {
                format!("no staged profile '{staged_id}'; call bm_generate_profile first")
            })?;
        to_json(
            crate::commands::profile::install_generated_profile(
                staged.profile_json,
                staged.metadata_info,
                staged.filename,
                force,
            )
            .await,
        )
    }

    async fn run_analysis(
        &self,
        photo_path: &str,
        profile_path: Option<String>,
    ) -> Result<Value, String> {
        let bytes =
            std::fs::read(photo_path).map_err(|e| format!("cannot read {photo_path}: {e}"))?;
        let request = crate::commands::analyzer::AnalyzeRequest {
            image_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            profile_path,
            material_type: None,
        };
        to_json(crate::commands::analyzer::analyze_print(self.app.clone(), request).await)
    }

    async fn history(&self, profile_path: &str) -> Result<Value, String> {
        to_json(
            crate::commands::history::list_history_sessions(
                self.app.clone(),
                profile_path.to_string(),
            )
            .await,
        )
    }

    async fn launch_bambu_studio(&self, profile_path: Option<String>) -> Result<Value, String> {
        to_json(
            crate::commands::launcher::launch_bambu_studio(self.app.clone(), None, profile_path)
                .await,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_specs_fills_missing_fields_from_defaults() {
        let specs =
            merge_specs(json!({"brand":"Polymaker","material":"PLA","nozzle_temp_min":190}))
                .unwrap();
        assert_eq!(specs.brand, "Polymaker");
        assert_eq!(specs.nozzle_temp_min, Some(190));
        assert_eq!(specs.bed_temp_max, None);
    }

    #[test]
    fn merge_specs_rejects_non_objects() {
        assert!(merge_specs(json!("PLA")).is_err());
    }
}
