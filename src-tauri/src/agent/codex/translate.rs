use serde_json::Value;

use crate::agent::types::{AgentEvent, TurnStatus};

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

pub fn translate(session_id: &str, seq: u32, method: &str, params: &Value) -> Vec<AgentEvent> {
    let sid = session_id.to_string();
    match method {
        "item/agentMessage/delta" => vec![AgentEvent::MessageDelta {
            session_id: sid,
            item_id: s(params, "itemId"),
            text: s(params, "delta"),
        }],
        "item/completed" => {
            let item = params.get("item").cloned().unwrap_or(Value::Null);
            match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "agentMessage" => vec![AgentEvent::MessageDone { session_id: sid, item_id: s(&item, "id"), text: s(&item, "text") }],
                // `failed` stays a Command: its exitCode carries the failure.
                "commandExecution" if item.get("status").and_then(|x| x.as_str()) == Some("declined") => {
                    vec![AgentEvent::Error {
                        session_id: Some(sid),
                        message: format!("Command declined: {}", s(&item, "command")),
                    }]
                }
                "commandExecution" => vec![AgentEvent::Command {
                    session_id: sid,
                    command: s(&item, "command"),
                    exit_code: item.get("exitCode").and_then(|c| c.as_i64()).map(|c| c as i32),
                }],
                "fileChange" => {
                    // A missing status is treated as applied.
                    let status = item.get("status").and_then(|x| x.as_str()).unwrap_or("completed");
                    item.get("changes")
                        .and_then(|c| c.as_array())
                        .map(|changes| {
                            changes
                                .iter()
                                .map(|c| match status {
                                    "completed" => AgentEvent::FileChange {
                                        session_id: sid.clone(),
                                        path: s(c, "path"),
                                        diff: s(c, "diff"),
                                    },
                                    other => AgentEvent::Error {
                                        session_id: Some(sid.clone()),
                                        message: format!("Codex did not apply changes to {} ({other})", s(c, "path")),
                                    },
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                "webSearch" => vec![AgentEvent::WebSearch { session_id: sid, query: s(&item, "query") }],
                "imageGeneration" => match item.get("savedPath").and_then(|p| p.as_str()) {
                    Some(p) => vec![AgentEvent::ImageGenerated { session_id: sid, path: p.to_string() }],
                    None => Vec::new(),
                },
                _ => Vec::new(),
            }
        }
        "turn/completed" => {
            let turn = params.get("turn").cloned().unwrap_or(Value::Null);
            let status = match turn.get("status").and_then(|x| x.as_str()) {
                Some("interrupted") => TurnStatus::Interrupted,
                Some("failed") => TurnStatus::Failed,
                _ => TurnStatus::Completed,
            };
            let mut out = Vec::new();
            if let Some(msg) = turn.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
                out.push(AgentEvent::Error { session_id: Some(sid.clone()), message: msg.to_string() });
            }
            out.push(AgentEvent::TurnDone { session_id: sid, seq, status });
            out
        }
        "account/rateLimits/updated" => {
            let primary = params.get("rateLimits").and_then(|r| r.get("primary"));
            vec![AgentEvent::Usage {
                session_id: sid,
                used_percent: primary.and_then(|p| p.get("usedPercent")).and_then(|u| u.as_f64()),
                resets_at: primary.and_then(|p| p.get("resetsAt")).and_then(|r| r.as_i64()),
            }]
        }
        "error" => {
            if params.get("willRetry").and_then(|w| w.as_bool()).unwrap_or(false) {
                return Vec::new();
            }
            let message = params
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Codex reported an error")
                .to_string();
            vec![AgentEvent::Error { session_id: Some(sid), message }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::TurnStatus;
    use serde_json::json;

    fn t(method: &str, params: Value) -> Vec<AgentEvent> {
        translate("s1", 2, method, &params)
    }

    #[test]
    fn agent_message_delta() {
        assert_eq!(
            t("item/agentMessage/delta", json!({"threadId":"th","turnId":"tu","itemId":"m1","delta":"Hel"})),
            vec![AgentEvent::MessageDelta { session_id: "s1".into(), item_id: "m1".into(), text: "Hel".into() }]
        );
    }

    #[test]
    fn completed_items_map_to_activity() {
        let msg = t("item/completed", json!({"item":{"type":"agentMessage","id":"m1","text":"Done."}}));
        assert_eq!(msg, vec![AgentEvent::MessageDone { session_id: "s1".into(), item_id: "m1".into(), text: "Done.".into() }]);

        let cmd = t("item/completed", json!({"item":{"type":"commandExecution","id":"c","command":"ls","exitCode":0,"commandActions":[],"cwd":"/","status":"completed"}}));
        assert_eq!(cmd, vec![AgentEvent::Command { session_id: "s1".into(), command: "ls".into(), exit_code: Some(0) }]);

        let fc = t("item/completed", json!({"item":{"type":"fileChange","id":"f","status":"completed","changes":[
            {"path":"/a.json","kind":{"type":"update"},"diff":"-1\n+2"},{"path":"/b.json","kind":{"type":"add"},"diff":"+x"}]}}));
        assert_eq!(fc.len(), 2);
        assert!(matches!(&fc[0], AgentEvent::FileChange { path, .. } if path == "/a.json"));

        let ws = t("item/completed", json!({"item":{"type":"webSearch","id":"w","query":"polyterra pla temp"}}));
        assert_eq!(ws, vec![AgentEvent::WebSearch { session_id: "s1".into(), query: "polyterra pla temp".into() }]);

        let img = t("item/completed", json!({"item":{"type":"imageGeneration","id":"i","result":"","status":"completed","savedPath":"/tmp/i.png"}}));
        assert_eq!(img, vec![AgentEvent::ImageGenerated { session_id: "s1".into(), path: "/tmp/i.png".into() }]);
    }

    #[test]
    fn declined_file_changes_become_errors() {
        let fc = t("item/completed", json!({"item":{"type":"fileChange","id":"f","status":"declined","changes":[
            {"path":"/a.json","kind":{"type":"update"},"diff":"-1\n+2"},{"path":"/b.json","kind":{"type":"add"},"diff":"+x"}]}}));
        assert_eq!(
            fc,
            vec![
                AgentEvent::Error { session_id: Some("s1".into()), message: "Codex did not apply changes to /a.json (declined)".into() },
                AgentEvent::Error { session_id: Some("s1".into()), message: "Codex did not apply changes to /b.json (declined)".into() },
            ]
        );
        let failed = t("item/completed", json!({"item":{"type":"fileChange","id":"f","status":"failed","changes":[{"path":"/a.json","diff":""}]}}));
        assert_eq!(failed, vec![AgentEvent::Error { session_id: Some("s1".into()), message: "Codex did not apply changes to /a.json (failed)".into() }]);
    }

    #[test]
    fn declined_commands_become_errors_but_failed_ones_stay_commands() {
        let declined = t("item/completed", json!({"item":{"type":"commandExecution","id":"c","command":"rm -rf x","commandActions":[],"cwd":"/","status":"declined"}}));
        assert_eq!(declined, vec![AgentEvent::Error { session_id: Some("s1".into()), message: "Command declined: rm -rf x".into() }]);
        let failed = t("item/completed", json!({"item":{"type":"commandExecution","id":"c","command":"false","exitCode":1,"commandActions":[],"cwd":"/","status":"failed"}}));
        assert_eq!(failed, vec![AgentEvent::Command { session_id: "s1".into(), command: "false".into(), exit_code: Some(1) }]);
    }

    #[test]
    fn dynamic_tool_items_are_ignored_because_the_backend_reports_them() {
        assert!(t("item/completed", json!({"item":{"type":"dynamicToolCall","id":"d","tool":"bm_app_state","arguments":{},"status":"completed"}})).is_empty());
    }

    #[test]
    fn turn_completed_maps_status_and_surfaces_errors() {
        assert_eq!(
            t("turn/completed", json!({"threadId":"th","turn":{"id":"tu","items":[],"status":"interrupted"}})),
            vec![AgentEvent::TurnDone { session_id: "s1".into(), seq: 2, status: TurnStatus::Interrupted }]
        );
        let failed = t("turn/completed", json!({"turn":{"id":"tu","items":[],"status":"failed","error":{"message":"usage limit reached"}}}));
        assert_eq!(failed.len(), 2);
        assert!(matches!(&failed[0], AgentEvent::Error { message, .. } if message == "usage limit reached"));
        assert!(matches!(&failed[1], AgentEvent::TurnDone { status: TurnStatus::Failed, .. }));
    }

    #[test]
    fn rate_limits_become_usage() {
        assert_eq!(
            t("account/rateLimits/updated", json!({"rateLimits":{"primary":{"usedPercent":42,"resetsAt":1790000000}}})),
            vec![AgentEvent::Usage { session_id: "s1".into(), used_percent: Some(42.0), resets_at: Some(1790000000) }]
        );
    }

    #[test]
    fn retrying_errors_are_suppressed() {
        assert!(t("error", json!({"error":{"message":"x"},"willRetry":true,"threadId":"th","turnId":"tu"})).is_empty());
        assert_eq!(t("error", json!({"error":{"message":"boom"},"willRetry":false})).len(), 1);
    }

    #[test]
    fn unknown_methods_are_ignored() {
        assert!(t("thread/tokenUsage/updated", json!({})).is_empty());
    }
}
