//! Serde mirrors of src-tauri/src/agent/types.rs. Keep tags and field names identical.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Readiness {
    Ready { detail: String },
    NotInstalled { hint: String },
    NeedsLogin { hint: String },
    NeedsApiKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AskRequest {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    pub allow_other: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionReady {
        session_id: String,
        provider: Provider,
    },
    TurnStarted {
        session_id: String,
        seq: u32,
    },
    MessageDelta {
        session_id: String,
        item_id: String,
        text: String,
    },
    MessageDone {
        session_id: String,
        item_id: String,
        text: String,
    },
    ToolCall {
        session_id: String,
        call_id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        session_id: String,
        call_id: String,
        ok: bool,
        summary: String,
    },
    FileChange {
        session_id: String,
        path: String,
        diff: String,
    },
    Command {
        session_id: String,
        command: String,
        exit_code: Option<i32>,
    },
    WebSearch {
        session_id: String,
        query: String,
    },
    ImageGenerated {
        session_id: String,
        path: String,
    },
    Ask {
        session_id: String,
        request: AskRequest,
    },
    Todo {
        session_id: String,
        items: Vec<TodoItem>,
    },
    Usage {
        session_id: String,
        used_percent: Option<f64>,
        resets_at: Option<i64>,
    },
    TurnDone {
        session_id: String,
        seq: u32,
        status: TurnStatus,
    },
    InvalidProfiles {
        session_id: String,
        seq: u32,
        paths: Vec<String>,
    },
    Error {
        session_id: Option<String>,
        message: String,
    },
}

impl AgentEvent {
    pub fn session_id(&self) -> Option<&str> {
        use AgentEvent::*;
        match self {
            SessionReady { session_id, .. }
            | TurnStarted { session_id, .. }
            | MessageDelta { session_id, .. }
            | MessageDone { session_id, .. }
            | ToolCall { session_id, .. }
            | ToolResult { session_id, .. }
            | FileChange { session_id, .. }
            | Command { session_id, .. }
            | WebSearch { session_id, .. }
            | ImageGenerated { session_id, .. }
            | Ask { session_id, .. }
            | Todo { session_id, .. }
            | Usage { session_id, .. }
            | TurnDone { session_id, .. }
            | InvalidProfiles { session_id, .. } => Some(session_id),
            Error { session_id, .. } => session_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AppState {
    pub route: String,
    pub selected_profile: Option<String>,
    pub selected_filament: Option<String>,
    pub photo_path: Option<String>,
    pub last_analysis_session: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiCommand {
    Navigate {
        route: String,
        profile_path: Option<String>,
    },
    Refresh {
        what: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentModel {
    pub id: String,
    pub display_name: String,
    pub efforts: Vec<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentSettings {
    pub full_access: bool,
    pub claude_auth_mode: AuthMode,
    pub claude_auth_modes: Vec<AuthMode>,
}
