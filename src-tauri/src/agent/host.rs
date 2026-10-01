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

/// The frontend pushes `route` on every change but leaves fields it does not
/// own (e.g. `photo_path`, set by `agent_stage_image`) as `None`. A `None`
/// therefore means "unchanged", not "cleared".
pub(crate) fn merge_app_state(current: &AppState, incoming: AppState) -> AppState {
    AppState {
        route: incoming.route,
        selected_profile: incoming
            .selected_profile
            .or_else(|| current.selected_profile.clone()),
        selected_filament: incoming
            .selected_filament
            .or_else(|| current.selected_filament.clone()),
        photo_path: incoming.photo_path.or_else(|| current.photo_path.clone()),
        last_analysis_session: incoming
            .last_analysis_session
            .or(current.last_analysis_session),
    }
}

/// Runs `install` on a clone of the staged entry and removes the entry only
/// if it succeeds, so a failed install can be retried with the same id. The
/// lock is never held across the `.await`.
pub(crate) async fn take_staged_on_success<T, R, F, Fut>(
    staged: &Mutex<HashMap<String, T>>,
    staged_id: &str,
    install: F,
) -> Result<R, String>
where
    T: Clone,
    F: FnOnce(T) -> Fut,
    Fut: std::future::Future<Output = Result<R, String>>,
{
    let entry = staged
        .lock()
        .unwrap()
        .get(staged_id)
        .cloned()
        .ok_or_else(|| {
            format!("no staged profile '{staged_id}'; call bm_generate_profile first")
        })?;
    let out = install(entry).await?;
    staged.lock().unwrap().remove(staged_id);
    Ok(out)
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

    /// Merges a frontend push into the current state; see `merge_app_state`.
    pub fn set_app_state(&self, s: AppState) {
        let mut state = self.state.lock().unwrap();
        *state = merge_app_state(&state, s);
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
        to_json(
            take_staged_on_success(&self.staged, staged_id, |staged: GenerateResult| {
                crate::commands::profile::install_generated_profile(
                    staged.profile_json,
                    staged.metadata_info,
                    staged.filename,
                    force,
                )
            })
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

    async fn slice(
        &self,
        req: super::tools::slicer::SliceToolRequest,
    ) -> Result<Vec<crate::slicer::jobs::JobView>, String> {
        use tauri::Manager;
        let svc = self
            .app
            .try_state::<crate::slicer::jobs::SlicerService>()
            .ok_or("Slicing isn't available.")?
            .inner()
            .clone();
        let defaults = crate::commands::slicer::effective_settings(&self.app);
        let filaments = std::iter::once(req.filament.clone())
            .chain(req.compare_filaments.iter().cloned().map(Some));
        // Cancels this call's unfinished jobs if it ends early: a refused
        // enqueue part-way through, or the agent turn being stopped (which
        // drops this future).
        let cancel_svc = svc.clone();
        let mut guard = PendingJobs::new(move |id| {
            cancel_svc.cancel(id);
        });
        for filament in filaments {
            let choice = defaults.choice(req.printer.clone(), req.process.clone(), filament)?;
            let view = svc
                .enqueue(crate::slicer::jobs::JobRequest {
                    source_path: req.model_path.clone(),
                    choice,
                    origin: crate::slicer::jobs::JobOrigin::Agent,
                })
                .map_err(|e| e.to_string())?;
            guard.ids.push(view.id);
        }
        self.emit_ui(UiCommand::Navigate {
            route: "/slice".into(),
            profile_path: None,
        });
        // Each job has its own 5-minute timeout; this bounds the wait for
        // jobs queued behind others. bm_slice_result picks up the rest.
        let deadline = tokio::time::Instant::now() + SLICE_WAIT;
        let mut views = Vec::new();
        for id in guard.ids.clone() {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if let Some(v) = svc.wait(id, left).await {
                views.push(v);
            }
        }
        // The caller has its answer: jobs still waiting past the bound keep
        // going, so bm_slice_result can pick them up.
        guard.ids.clear();
        Ok(views)
    }

    fn slice_job(&self, job_id: u64) -> Option<crate::slicer::jobs::JobView> {
        use tauri::Manager;
        self.app
            .try_state::<crate::slicer::jobs::SlicerService>()
            .and_then(|s| s.job(job_id))
    }
}

/// How long `bm_slice` waits for its jobs before handing back whatever state
/// they are in.
const SLICE_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// Jobs `bm_slice` queued and hasn't answered for yet. Dropping it cancels
/// them, so a refused enqueue part-way through, or a stopped agent turn
/// (which drops the waiting future), doesn't leave work nobody asked to keep.
struct PendingJobs<F: Fn(u64)> {
    cancel: F,
    ids: Vec<u64>,
}

impl<F: Fn(u64)> PendingJobs<F> {
    fn new(cancel: F) -> Self {
        Self {
            cancel,
            ids: Vec::new(),
        }
    }
}

impl<F: Fn(u64)> Drop for PendingJobs<F> {
    fn drop(&mut self) {
        for id in &self.ids {
            (self.cancel)(*id);
        }
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

    fn with_photo(p: &str) -> AppState {
        AppState {
            route: "/analysis".into(),
            selected_profile: Some("a.json".into()),
            photo_path: Some(p.into()),
            ..Default::default()
        }
    }

    #[test]
    fn merge_app_state_keeps_host_owned_fields_when_push_omits_them() {
        let pushed = AppState {
            route: "/profiles".into(),
            ..Default::default()
        };
        let merged = merge_app_state(&with_photo("/tmp/p.jpg"), pushed);
        assert_eq!(merged.route, "/profiles");
        assert_eq!(merged.photo_path.as_deref(), Some("/tmp/p.jpg"));
        assert_eq!(merged.selected_profile.as_deref(), Some("a.json"));
    }

    #[test]
    fn merge_app_state_replaces_fields_the_push_sets() {
        let pushed = AppState {
            route: "/analysis".into(),
            photo_path: Some("/tmp/new.jpg".into()),
            last_analysis_session: Some(7),
            ..Default::default()
        };
        let merged = merge_app_state(&with_photo("/tmp/old.jpg"), pushed);
        assert_eq!(merged.photo_path.as_deref(), Some("/tmp/new.jpg"));
        assert_eq!(merged.last_analysis_session, Some(7));
    }

    fn staged_map() -> Mutex<HashMap<String, String>> {
        Mutex::new(HashMap::from([("s1".to_string(), "profile".to_string())]))
    }

    #[tokio::test]
    async fn take_staged_keeps_entry_when_install_fails() {
        let map = staged_map();
        let r: Result<(), String> =
            take_staged_on_success(&map, "s1", |_| async { Err("disk full".to_string()) }).await;
        assert_eq!(r, Err("disk full".to_string()));
        assert!(
            map.lock().unwrap().contains_key("s1"),
            "retry must still find it"
        );
    }

    #[tokio::test]
    async fn take_staged_removes_entry_after_successful_install() {
        let map = staged_map();
        let r = take_staged_on_success(&map, "s1", |p| async move { Ok(p.len()) }).await;
        assert_eq!(r, Ok(7));
        assert!(!map.lock().unwrap().contains_key("s1"));
    }

    #[tokio::test]
    async fn take_staged_errors_on_unknown_id() {
        let map = staged_map();
        let r: Result<(), String> =
            take_staged_on_success(&map, "nope", |_| async { Ok(()) }).await;
        assert!(r.unwrap_err().contains("no staged profile 'nope'"));
    }

    #[test]
    fn pending_jobs_are_cancelled_when_dropped_but_not_once_answered() {
        let cancelled = std::cell::RefCell::new(Vec::new());
        {
            let mut g = PendingJobs::new(|id| cancelled.borrow_mut().push(id));
            g.ids.extend([1, 2]);
        }
        assert_eq!(*cancelled.borrow(), vec![1, 2]);
        cancelled.borrow_mut().clear();
        {
            let mut g = PendingJobs::new(|id| cancelled.borrow_mut().push(id));
            g.ids.extend([3]);
            g.ids.clear();
        }
        assert!(cancelled.borrow().is_empty());
    }
}
