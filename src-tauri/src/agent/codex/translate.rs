use serde_json::Value;

use crate::agent::types::AgentEvent;

pub fn translate(_session_id: &str, _seq: u32, _method: &str, _params: &Value) -> Vec<AgentEvent> {
    Vec::new()
}
