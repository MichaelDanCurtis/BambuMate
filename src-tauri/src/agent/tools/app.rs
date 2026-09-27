use super::{ToolOutput, ToolRegistry, ToolSpec};
use serde_json::Value;

pub fn specs() -> Vec<ToolSpec> {
    Vec::new()
}

pub async fn handle(_reg: &ToolRegistry, _name: &str, _args: &Value) -> Option<ToolOutput> {
    None
}
