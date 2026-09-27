use serde_json::{json, Value};

use super::{arg_str, ToolOutput, ToolRegistry, ToolSpec};
use crate::agent::types::{AgentEvent, AskOption, TodoItem};

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_ask",
            description: "Ask the user a multiple-choice question in the BambuMate panel and wait for the answer. Use when a choice is genuinely theirs.",
            input_schema: json!({"type":"object","properties":{
                "header":{"type":"string","description":"Short label, max 12 chars"},
                "question":{"type":"string"},
                "options":{"type":"array","items":{"type":"object","properties":{"label":{"type":"string"},"description":{"type":"string"}},"required":["label","description"]}},
                "allow_other":{"type":"boolean"}
            },"required":["header","question","options"]}),
        },
        ToolSpec {
            name: "bm_confirm",
            description: "Ask the user a yes/no confirmation before a risky action. Returns {\"confirmed\": bool}.",
            input_schema: json!({"type":"object","properties":{"prompt":{"type":"string"}},"required":["prompt"]}),
        },
        ToolSpec {
            name: "bm_todo",
            description: "Show or update a live checklist of your plan in the BambuMate panel. Send the full list each time.",
            input_schema: json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"text":{"type":"string"},"done":{"type":"boolean"}},"required":["text","done"]}}},"required":["items"]}),
        },
    ]
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "bm_ask" => {
            let header = match arg_str(args, "header") { Ok(v) => v, Err(e) => return Some(e) };
            let question = match arg_str(args, "question") { Ok(v) => v, Err(e) => return Some(e) };
            let options: Vec<AskOption> = args
                .get("options")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let allow_other = args.get("allow_other").and_then(|v| v.as_bool()).unwrap_or(false);
            match reg.asks().ask(reg.session_id(), &header, &question, options, allow_other).await {
                Ok(answers) => ToolOutput::json(&json!({"answers": answers})),
                Err(e) => ToolOutput::error(e),
            }
        }
        "bm_confirm" => {
            let prompt = match arg_str(args, "prompt") { Ok(v) => v, Err(e) => return Some(e) };
            let confirmed = reg.asks().confirm(reg.session_id(), &prompt).await;
            ToolOutput::json(&json!({"confirmed": confirmed}))
        }
        "bm_todo" => {
            let items: Vec<TodoItem> = match args.get("items").cloned().map(serde_json::from_value) {
                Some(Ok(items)) => items,
                _ => return Some(ToolOutput::error("'items' must be a list of {text, done}")),
            };
            reg.asks().emit(AgentEvent::Todo { session_id: reg.session_id().to_string(), items });
            ToolOutput::text("checklist updated")
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::agent::types::AgentEvent;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn bm_todo_emits_todo_event() {
        let host = Arc::new(FakeHost::new());
        let (reg, mut rx) = registry_with(host.clone());
        let out = reg.call("bm_todo", json!({"items":[{"text":"Read profile","done":true}]})).await;
        assert!(out.ok);
        match rx.recv().await.unwrap() {
            AgentEvent::Todo { items, .. } => assert_eq!(items[0].text, "Read profile"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn bm_ask_returns_answers_as_json() {
        let host = Arc::new(FakeHost::new());
        let (reg, mut rx) = registry_with(host);
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_ask", json!({"header":"Nozzle","question":"Which?","options":[{"label":"0.4","description":"std"}]})).await
        });
        let id = match rx.recv().await.unwrap() {
            AgentEvent::Ask { request, .. } => request.id,
            other => panic!("unexpected {other:?}"),
        };
        reg.asks().answer(&id, vec!["0.4".into()]).unwrap();
        let out = t.await.unwrap();
        assert!(out.ok);
        assert!(out.summary().contains("0.4"));
    }

    #[tokio::test]
    async fn unknown_tool_is_an_error_output() {
        let (reg, _rx) = registry_with(Arc::new(FakeHost::new()));
        let out = reg.call("bm_nope", json!({})).await;
        assert!(!out.ok);
        assert!(out.summary().contains("unknown tool"));
    }

    #[test]
    fn codex_dynamic_tools_are_function_specs() {
        let tools = crate::agent::tools::ToolRegistry::codex_dynamic_tools();
        let first = &tools.as_array().unwrap()[0];
        assert_eq!(first["type"], "function");
        assert!(first["name"].as_str().unwrap().starts_with("bm_"));
        assert!(first["inputSchema"].is_object());
    }

    #[test]
    fn to_codex_response_encodes_images_as_data_urls() {
        use crate::agent::tools::{ToolContent, ToolOutput};
        let out = ToolOutput {
            ok: true,
            content: vec![
                ToolContent::Text("photo".into()),
                ToolContent::Image { mime: "image/jpeg".into(), base64: "AAAA".into() },
            ],
        };
        let v = out.to_codex_response();
        assert_eq!(v["success"], true);
        assert_eq!(v["contentItems"][0], json!({"type":"inputText","text":"photo"}));
        assert_eq!(v["contentItems"][1], json!({"type":"inputImage","imageUrl":"data:image/jpeg;base64,AAAA"}));
    }
}
