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

pub struct FakeHost {
    pub user_dir: tempfile::TempDir,
    pub system_dir: Option<PathBuf>,
    pub state: Mutex<AppState>,
    pub ui: Mutex<Vec<UiCommand>>,
    pub calls: Mutex<Vec<String>>,
    pub bs_running: AtomicBool,
    pub printer: Mutex<PrinterView>,
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
}
