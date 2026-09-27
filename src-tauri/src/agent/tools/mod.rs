//! The `bm_*` tool surface shared by both agent backends.

pub mod app;
#[cfg(test)]
pub mod fake_host;
pub mod interact;
pub mod profiles;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use super::asks::AskBroker;
use super::types::{AppState, UiCommand};

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolContent {
    Text(String),
    Image { mime: String, base64: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub ok: bool,
    pub content: Vec<ToolContent>,
}

impl ToolOutput {
    pub fn text(t: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: vec![ToolContent::Text(t.into())],
        }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
    }
    pub fn error(m: impl Into<String>) -> Self {
        Self {
            ok: false,
            content: vec![ToolContent::Text(format!("error: {}", m.into()))],
        }
    }
    pub fn summary(&self) -> String {
        let first = self
            .content
            .iter()
            .find_map(|c| match c {
                ToolContent::Text(t) => Some(t.as_str()),
                ToolContent::Image { .. } => None,
            })
            .unwrap_or("[image]");
        first.chars().take(200).collect()
    }
    pub fn to_codex_response(&self) -> Value {
        let items: Vec<Value> = self
            .content
            .iter()
            .map(|c| match c {
                ToolContent::Text(t) => json!({"type":"inputText","text":t}),
                ToolContent::Image { mime, base64 } => {
                    json!({"type":"inputImage","imageUrl":format!("data:{mime};base64,{base64}")})
                }
            })
            .collect();
        json!({"success": self.ok, "contentItems": items})
    }
}

/// Everything a tool needs from the running app. `TauriToolHost` implements
/// it for production; `FakeHost` for tests.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn user_filament_dir(&self) -> Result<PathBuf, String>;
    fn system_filament_dir(&self) -> Option<PathBuf>;
    fn app_state(&self) -> AppState;
    fn emit_ui(&self, cmd: UiCommand);
    fn bambu_studio_running(&self) -> bool;
    async fn search_filament(&self, name: &str) -> Result<Value, String>;
    async fn catalog_search(&self, query: &str, limit: usize) -> Result<Value, String>;
    /// Generates without writing; returns a summary that includes `staged_id`.
    async fn generate_profile(
        &self,
        specs: Value,
        target_printer: Option<String>,
        base_profile_path: Option<String>,
    ) -> Result<Value, String>;
    /// Installs a staged profile; returns JSON with `installed_path`.
    async fn install_staged(&self, staged_id: &str, force: bool) -> Result<Value, String>;
    async fn run_analysis(
        &self,
        photo_path: &str,
        profile_path: Option<String>,
    ) -> Result<Value, String>;
    async fn history(&self, profile_path: &str) -> Result<Value, String>;
    async fn launch_bambu_studio(&self, profile_path: Option<String>) -> Result<Value, String>;
}

pub fn arg_str(args: &Value, key: &str) -> Result<String, ToolOutput> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolOutput::error(format!("missing string argument '{key}'")))
}

pub fn arg_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Default)]
struct WriteScope {
    created: HashSet<PathBuf>,
    confirmed: HashSet<PathBuf>,
    /// Per-path async locks serializing the check-then-confirm sequence in
    /// `ensure_write_allowed`, so two concurrent writers to the same path
    /// can't both observe "not yet confirmed" and both prompt the user.
    locks: HashMap<PathBuf, Arc<AsyncMutex<()>>>,
}

/// One registry per agent session.
pub struct ToolRegistry {
    session_id: String,
    host: Arc<dyn ToolHost>,
    asks: Arc<AskBroker>,
    scope: Mutex<WriteScope>,
}

impl ToolRegistry {
    pub fn new(session_id: String, host: Arc<dyn ToolHost>, asks: Arc<AskBroker>) -> Self {
        Self {
            session_id,
            host,
            asks,
            scope: Mutex::new(WriteScope::default()),
        }
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn host(&self) -> &Arc<dyn ToolHost> {
        &self.host
    }
    pub fn asks(&self) -> &Arc<AskBroker> {
        &self.asks
    }

    pub fn specs() -> Vec<ToolSpec> {
        let mut all = interact::specs();
        all.extend(profiles::specs());
        all.extend(app::specs());
        all
    }

    pub fn codex_dynamic_tools() -> Value {
        Value::Array(
            Self::specs()
                .into_iter()
                .map(|s| json!({"type":"function","name":s.name,"description":s.description,"inputSchema":s.input_schema}))
                .collect(),
        )
    }

    pub async fn call(&self, name: &str, args: Value) -> ToolOutput {
        if let Some(out) = interact::handle(self, name, &args).await {
            return out;
        }
        if let Some(out) = profiles::handle(self, name, &args).await {
            return out;
        }
        if let Some(out) = app::handle(self, name, &args).await {
            return out;
        }
        ToolOutput::error(format!("unknown tool '{name}'"))
    }

    pub fn mark_created(&self, path: &Path) {
        self.scope
            .lock()
            .unwrap()
            .created
            .insert(path.to_path_buf());
    }

    /// Enforces the risky-write rules from the spec. Asks the user when needed.
    pub async fn ensure_write_allowed(&self, path: &Path) -> Result<(), String> {
        if self.host.bambu_studio_running()
            && !self
                .asks
                .confirm(
                    &self.session_id,
                    &format!("Bambu Studio is running. Write {} anyway?", path.display()),
                )
                .await
        {
            return Err("declined: Bambu Studio is running".into());
        }
        // Serialize the check-then-confirm sequence per path: fetch or create
        // this path's lock under the std mutex, then drop that guard before
        // awaiting so the std mutex is never held across an await point. Two
        // concurrent writers to the same path then run this section one at a
        // time, and the second sees `confirmed` already set and skips asking.
        let lock = {
            let mut s = self.scope.lock().unwrap();
            s.locks
                .entry(path.to_path_buf())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let _path_guard = lock.lock().await;
        let needs_confirm = {
            let s = self.scope.lock().unwrap();
            path.exists() && !s.created.contains(path) && !s.confirmed.contains(path)
        };
        if needs_confirm {
            if !self
                .asks
                .confirm(
                    &self.session_id,
                    &format!("Modify existing profile {}?", path.display()),
                )
                .await
            {
                return Err("declined by user".into());
            }
            self.scope
                .lock()
                .unwrap()
                .confirmed
                .insert(path.to_path_buf());
        }
        Ok(())
    }
}
