use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use super::{ToolHost, ToolRegistry};
use crate::agent::asks::AskBroker;
use crate::agent::types::{AgentEvent, AppState, UiCommand};
use crate::printer::service::PrinterView;
use crate::slicer::jobs::{JobOrigin, JobState, JobView};

/// A finished job built from the real cube fixture.
pub fn done_job(id: u64) -> JobView {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/slicer/cube_h2c.gcode.3mf");
    let mut result = crate::slicer::result::parse_output(&fixture, None, None).unwrap();
    // A file that exists, as a finished job's output does.
    result.output_path = fixture.to_string_lossy().into_owned();
    JobView {
        id,
        origin: JobOrigin::Agent,
        source_path: "/models/cube.stl".into(),
        model_name: "cube.stl".into(),
        printer: "Bambu Lab H2C 0.4 nozzle".into(),
        process: "0.20mm Standard @BBL H2C".into(),
        filament: "Bambu PLA Basic @BBL H2C".into(),
        bed_type: "Textured PEI Plate".into(),
        state: JobState::Done {
            result,
            cached: false,
        },
    }
}

pub struct FakeHost {
    pub user_dir: tempfile::TempDir,
    pub system_dir: Option<PathBuf>,
    pub state: Mutex<AppState>,
    pub ui: Mutex<Vec<UiCommand>>,
    pub calls: Mutex<Vec<String>>,
    pub bs_running: AtomicBool,
    pub printer: Mutex<PrinterView>,
    pub slice_jobs: Mutex<std::collections::HashMap<u64, JobView>>,
    /// Makes `slice` wait forever, like a long queue.
    pub slice_hangs: AtomicBool,
    /// Set once a hanging `slice` is waiting.
    pub slice_waiting: AtomicBool,
    /// Set when a hanging `slice` future is dropped (its jobs get cancelled).
    pub slice_dropped: Arc<AtomicBool>,
}

struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl FakeHost {
    pub fn new() -> Self {
        Self {
            user_dir: tempfile::tempdir().unwrap(),
            system_dir: None,
            state: Mutex::new(AppState::default()),
            ui: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            bs_running: AtomicBool::new(false),
            printer: Mutex::new(PrinterView::unconfigured()),
            slice_jobs: Mutex::new(std::collections::HashMap::new()),
            slice_hangs: AtomicBool::new(false),
            slice_waiting: AtomicBool::new(false),
            slice_dropped: Arc::new(AtomicBool::new(false)),
        }
    }
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
}

impl Default for FakeHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry over a fake host plus a receiver subscribed *before* any tool runs.
pub fn registry_with(host: Arc<FakeHost>) -> (ToolRegistry, broadcast::Receiver<AgentEvent>) {
    let (tx, rx) = broadcast::channel(64);
    let asks = Arc::new(AskBroker::new(tx));
    (ToolRegistry::new("s1".into(), host, asks), rx)
}

#[async_trait]
impl ToolHost for FakeHost {
    fn user_filament_dir(&self) -> Result<PathBuf, String> {
        Ok(self.user_dir.path().to_path_buf())
    }
    fn system_filament_dir(&self) -> Option<PathBuf> {
        self.system_dir.clone()
    }
    fn app_state(&self) -> AppState {
        self.state.lock().unwrap().clone()
    }
    fn emit_ui(&self, cmd: UiCommand) {
        self.ui.lock().unwrap().push(cmd);
    }
    fn bambu_studio_running(&self) -> bool {
        self.bs_running.load(Ordering::SeqCst)
    }
    fn printer_view(&self) -> PrinterView {
        self.printer.lock().unwrap().clone()
    }
    async fn search_filament(&self, name: &str) -> Result<Value, String> {
        self.log(format!("search_filament:{name}"));
        Ok(json!({"brand":"Polymaker","serial":name,"material":"PLA"}))
    }
    async fn catalog_search(&self, query: &str, limit: usize) -> Result<Value, String> {
        self.log(format!("catalog_search:{query}:{limit}"));
        Ok(json!([]))
    }
    async fn generate_profile(
        &self,
        _specs: Value,
        tp: Option<String>,
        _b: Option<String>,
    ) -> Result<Value, String> {
        self.log(format!("generate_profile:{}", tp.unwrap_or_default()));
        Ok(
            json!({"staged_id":"stg1","profile_name":"Polymaker PLA","filename":"Polymaker PLA.json"}),
        )
    }
    async fn install_staged(&self, staged_id: &str, force: bool) -> Result<Value, String> {
        self.log(format!("install_staged:{staged_id}:{force}"));
        let p = self.user_dir.path().join("Polymaker PLA.json");
        std::fs::write(&p, r#"{"name":"Polymaker PLA","filament_id":"P1234567"}"#).unwrap();
        Ok(json!({"installed_path": p.to_string_lossy()}))
    }
    async fn run_analysis(&self, photo: &str, profile: Option<String>) -> Result<Value, String> {
        self.log(format!(
            "run_analysis:{photo}:{}",
            profile.unwrap_or_default()
        ));
        Ok(json!({"defect_report":{"defects":[]}}))
    }
    async fn history(&self, profile_path: &str) -> Result<Value, String> {
        self.log(format!("history:{profile_path}"));
        Ok(json!([]))
    }
    async fn launch_bambu_studio(&self, profile_path: Option<String>) -> Result<Value, String> {
        self.log(format!("launch:{}", profile_path.unwrap_or_default()));
        Ok(json!({"launched":true}))
    }
    async fn slice(&self, req: super::slicer::SliceToolRequest) -> Result<Vec<JobView>, String> {
        self.log(format!(
            "slice:{}:{}:{}",
            req.model_path,
            req.filament.clone().unwrap_or_default(),
            req.compare_filaments.join(",")
        ));
        if self.slice_hangs.load(Ordering::SeqCst) {
            let _dropped = SetOnDrop(self.slice_dropped.clone());
            self.slice_waiting.store(true, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
        Ok(vec![done_job(1)])
    }
    async fn slice_thumbnail(&self, job_id: u64, plate: u32) -> Result<Option<String>, String> {
        let view = self.slice_job(job_id).ok_or("no such job")?;
        crate::commands::slicer::plate_thumbnail(
            &view,
            plate,
            crate::commands::slicer::MAX_THUMBNAIL_BYTES,
        )
        .map_err(str::to_string)
    }
    fn slice_job(&self, job_id: u64) -> Option<JobView> {
        if let Some(job) = self.slice_jobs.lock().unwrap().get(&job_id).cloned() {
            return Some(job);
        }
        match job_id {
            1 => Some(done_job(1)),
            7 => Some(JobView {
                state: JobState::Failed {
                    error: crate::slicer::SlicerError::Slicer {
                        message: "No valid nozzle found. Please check nozzle count.".into(),
                    }
                    .view(),
                },
                ..done_job(7)
            }),
            // Finished, but its sliced file is gone.
            8 => {
                let mut job = done_job(8);
                if let JobState::Done { result, .. } = &mut job.state {
                    result.output_path = "/nope/output.gcode.3mf".into();
                }
                Some(job)
            }
            _ => None,
        }
    }
}
