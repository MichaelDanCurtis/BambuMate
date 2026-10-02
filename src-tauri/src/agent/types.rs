use serde::{Deserialize, Serialize};

/// Which agent CLI drives a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

/// Whether a backend can start a session right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Readiness {
    Ready { detail: String },
    NotInstalled { hint: String },
    NeedsLogin { hint: String },
    NeedsApiKey,
}

/// One piece of user input for a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserInput {
    Text {
        text: String,
    },
    /// Absolute path to a local image file.
    Image {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskRequest {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    /// When true the UI also offers a free-text answer.
    pub allow_other: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
}

/// Normalized event stream. The UI renders only these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// What the user is looking at. Pushed by the frontend on every change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub route: String,
    pub selected_profile: Option<String>,
    pub selected_filament: Option<String>,
    pub photo_path: Option<String>,
    pub last_analysis_session: Option<i64>,
    #[serde(default)]
    pub slice: Option<SliceContext>,
}

/// Choices and selection on BambuMate's Slice page (not Studio's live GUI).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SliceContext {
    pub model_path: Option<String>,
    pub printer: String,
    pub process: String,
    pub filament: String,
    pub bed_type: String,
    pub compare_filaments: Vec<String>,
    pub selected_job_id: Option<u64>,
    pub selected_plate: Option<u32>,
}

/// Instructions from the agent to the UI, emitted on `agent://ui`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentModel {
    pub id: String,
    pub display_name: String,
    pub efforts: Vec<String>,
    pub is_default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_event_uses_kind_tag_and_snake_case() {
        let e = AgentEvent::MessageDelta {
            session_id: "s1".into(),
            item_id: "i1".into(),
            text: "hi".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(
            v,
            json!({"kind":"message_delta","session_id":"s1","item_id":"i1","text":"hi"})
        );
    }

    #[test]
    fn readiness_uses_state_tag() {
        let v = serde_json::to_value(Readiness::NeedsApiKey).unwrap();
        assert_eq!(v, json!({"state":"needs_api_key"}));
        let v = serde_json::to_value(Readiness::Ready {
            detail: "ok".into(),
        })
        .unwrap();
        assert_eq!(v, json!({"state":"ready","detail":"ok"}));
    }

    #[test]
    fn user_input_round_trips() {
        let inputs = vec![
            UserInput::Text {
                text: "fix stringing".into(),
            },
            UserInput::Image {
                path: "/tmp/p.jpg".into(),
            },
        ];
        let s = serde_json::to_string(&inputs).unwrap();
        assert_eq!(serde_json::from_str::<Vec<UserInput>>(&s).unwrap(), inputs);
    }

    #[test]
    fn ui_command_uses_action_tag() {
        let v = serde_json::to_value(UiCommand::Navigate {
            route: "/profiles".into(),
            profile_path: None,
        })
        .unwrap();
        assert_eq!(
            v,
            json!({"action":"navigate","route":"/profiles","profile_path":null})
        );
    }

    #[test]
    fn provider_is_lowercase() {
        assert_eq!(
            serde_json::to_value(Provider::Codex).unwrap(),
            json!("codex")
        );
        assert_eq!(
            serde_json::from_value::<Provider>(json!("claude")).unwrap(),
            Provider::Claude
        );
    }
}
