//! The contract both agent lanes implement. The service and UI never branch
//! on provider beyond choosing which backend to call.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use super::tools::ToolRegistry;
use super::types::{AgentModel, Provider, Readiness, UserInput};

#[derive(Clone)]
pub struct SessionOpts {
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Lift the folder scoping: Codex dangerFullAccess / Claude skip-permissions.
    pub full_access: bool,
    /// Working directory for the agent process.
    pub cwd: PathBuf,
    /// Folders the agent may write when not in full access.
    pub writable_roots: Vec<PathBuf>,
    pub registry: Arc<ToolRegistry>,
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    fn provider(&self) -> Provider;
    async fn readiness(&self) -> Readiness;
    async fn models(&self) -> Result<Vec<AgentModel>, String>;
    /// Starts an interactive login when the backend supports it; returns a URL to open.
    async fn login(&self) -> Result<Option<String>, String>;
    /// Returns the backend's own conversation id (Codex thread id / Claude session id).
    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String>;
    async fn resume_session(
        &self,
        session_id: &str,
        backend_id: &str,
        opts: SessionOpts,
    ) -> Result<(), String>;
    /// Drops conversation history from user message `to_seq` onward. Returns
    /// false when the backend cannot rewind its conversation. The service then
    /// rewinds files only and tells the agent so in the next message.
    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String>;
    /// Starts a turn and returns the backend's turn id. Progress arrives as AgentEvents.
    async fn send(
        &self,
        session_id: &str,
        seq: u32,
        input: Vec<UserInput>,
    ) -> Result<String, String>;
    async fn interrupt(&self, session_id: &str) -> Result<(), String>;
    async fn end_session(&self, session_id: &str);
}
