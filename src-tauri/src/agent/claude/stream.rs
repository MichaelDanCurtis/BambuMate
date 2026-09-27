//! Claude CLI `--input-format/--output-format stream-json` codec.

use std::path::Path;

use serde_json::{json, Value};

use crate::agent::tools::app::load_photo;
use crate::agent::types::{AgentEvent, TurnStatus, UserInput};

pub fn encode_user_message(input: &[UserInput]) -> Result<String, String> {
    let mut content = Vec::new();
    for item in input {
        match item {
            UserInput::Text { text } => content.push(json!({"type":"text","text":text})),
            UserInput::Image { path } => {
                let (mime, data) = load_photo(Path::new(path))?;
                content.push(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}));
            }
        }
    }
    let mut line = json!({"type":"user","message":{"role":"user","content":content}}).to_string();
    line.push('\n');
    Ok(line)
}

#[derive(Debug, Default)]
pub struct Decoded {
    pub events: Vec<AgentEvent>,
    pub turn_finished: bool,
    pub assistant_uuid: Option<String>,
}

pub struct ClaudeDecoder {
    session_id: String,
    seq: u32,
    current_item: String,
    interrupted: bool,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

impl ClaudeDecoder {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            seq: 0,
            current_item: String::new(),
            interrupted: false,
        }
    }

    pub fn begin_turn(&mut self, seq: u32) {
        self.seq = seq;
        self.interrupted = false;
    }

    pub fn mark_interrupted(&mut self) {
        self.interrupted = true;
    }

    pub fn decode_line(&mut self, line: &str) -> Decoded {
        let mut out = Decoded::default();
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return out;
        };
        let sid = self.session_id.clone();
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "stream_event" => {
                let ev = &v["event"];
                match ev.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "message_start" => self.current_item = s(&ev["message"], "id"),
                    "content_block_delta" if ev["delta"]["type"] == "text_delta" => {
                        out.events.push(AgentEvent::MessageDelta {
                            session_id: sid,
                            item_id: self.current_item.clone(),
                            text: s(&ev["delta"], "text"),
                        });
                    }
                    _ => {}
                }
            }
            "assistant" => {
                out.assistant_uuid = v.get("uuid").and_then(|u| u.as_str()).map(str::to_string);
                let msg = &v["message"];
                let item_id = s(msg, "id");
                let blocks = msg
                    .get("content")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default();
                let text: String = blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .map(|b| s(b, "text"))
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    out.events.push(AgentEvent::MessageDone {
                        session_id: sid.clone(),
                        item_id,
                        text,
                    });
                }
                for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let name = s(b, "name");
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    if name.starts_with("mcp__bambumate__") {
                        continue;
                    }
                    out.events.push(match name.as_str() {
                        "WebSearch" => AgentEvent::WebSearch {
                            session_id: sid.clone(),
                            query: s(&input, "query"),
                        },
                        "Bash" => AgentEvent::Command {
                            session_id: sid.clone(),
                            command: s(&input, "command"),
                            exit_code: None,
                        },
                        "Edit" | "MultiEdit" => AgentEvent::FileChange {
                            session_id: sid.clone(),
                            path: s(&input, "file_path"),
                            diff: format!(
                                "-{}\n+{}",
                                s(&input, "old_string"),
                                s(&input, "new_string")
                            ),
                        },
                        "Write" => AgentEvent::FileChange {
                            session_id: sid.clone(),
                            path: s(&input, "file_path"),
                            diff: "(file written)".into(),
                        },
                        _ => AgentEvent::ToolCall {
                            session_id: sid.clone(),
                            call_id: s(b, "id"),
                            name,
                            args: input,
                        },
                    });
                }
            }
            "result" => {
                out.turn_finished = true;
                let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                let status = if self.interrupted {
                    TurnStatus::Interrupted
                } else if is_error {
                    let msg = v
                        .get("result")
                        .and_then(|r| r.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| s(&v, "subtype"));
                    out.events.push(AgentEvent::Error {
                        session_id: Some(sid.clone()),
                        message: msg,
                    });
                    TurnStatus::Failed
                } else {
                    TurnStatus::Completed
                };
                out.events.push(AgentEvent::TurnDone {
                    session_id: sid,
                    seq: self.seq,
                    status,
                });
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::TurnStatus;
    use serde_json::json;

    fn dec() -> ClaudeDecoder {
        let mut d = ClaudeDecoder::new("s1");
        d.begin_turn(4);
        d
    }

    #[test]
    fn encodes_text_and_images_as_one_user_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p.png");
        image::RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0]))
            .save(&p)
            .unwrap();
        let line = encode_user_message(&[
            UserInput::Text {
                text: "look".into(),
            },
            UserInput::Image {
                path: p.to_string_lossy().into_owned(),
            },
        ])
        .unwrap();
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(
            v["message"]["content"][0],
            json!({"type":"text","text":"look"})
        );
        assert_eq!(
            v["message"]["content"][1]["source"]["media_type"],
            "image/jpeg"
        );
    }

    #[test]
    fn streams_text_deltas_under_the_current_message_id() {
        let mut d = dec();
        d.decode_line(&json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1"}}}).to_string());
        let out = d.decode_line(&json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}).to_string());
        assert_eq!(
            out.events,
            vec![AgentEvent::MessageDelta {
                session_id: "s1".into(),
                item_id: "msg_1".into(),
                text: "Hi".into()
            }]
        );
    }

    #[test]
    fn assistant_message_maps_text_and_native_tools_but_skips_bambumate_tools() {
        let mut d = dec();
        let out = d.decode_line(&json!({"type":"assistant","uuid":"u-9","message":{"id":"msg_1","content":[
            {"type":"text","text":"Checking specs."},
            {"type":"tool_use","id":"t1","name":"WebSearch","input":{"query":"polyterra temp"}},
            {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls"}},
            {"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/p/A.json","old_string":"220","new_string":"215"}},
            {"type":"tool_use","id":"t4","name":"mcp__bambumate__bm_app_state","input":{}}
        ]}}).to_string());
        assert_eq!(out.assistant_uuid.as_deref(), Some("u-9"));
        assert_eq!(out.events.len(), 4);
        assert_eq!(
            out.events[0],
            AgentEvent::MessageDone {
                session_id: "s1".into(),
                item_id: "msg_1".into(),
                text: "Checking specs.".into()
            }
        );
        assert_eq!(
            out.events[1],
            AgentEvent::WebSearch {
                session_id: "s1".into(),
                query: "polyterra temp".into()
            }
        );
        assert_eq!(
            out.events[2],
            AgentEvent::Command {
                session_id: "s1".into(),
                command: "ls".into(),
                exit_code: None
            }
        );
        assert!(
            matches!(&out.events[3], AgentEvent::FileChange { path, diff, .. } if path == "/p/A.json" && diff.contains("+215"))
        );
    }

    #[test]
    fn result_finishes_the_turn() {
        let mut d = dec();
        let ok = d.decode_line(&json!({"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"x"}).to_string());
        assert!(ok.turn_finished);
        assert_eq!(
            ok.events,
            vec![AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 4,
                status: TurnStatus::Completed
            }]
        );

        let mut d = dec();
        let bad = d.decode_line(&json!({"type":"result","subtype":"error_during_execution","is_error":true,"result":"Invalid API key"}).to_string());
        assert!(
            matches!(&bad.events[0], AgentEvent::Error { message, .. } if message.contains("Invalid API key"))
        );
        assert!(matches!(
            &bad.events[1],
            AgentEvent::TurnDone {
                status: TurnStatus::Failed,
                ..
            }
        ));

        let mut d = dec();
        d.mark_interrupted();
        let int = d.decode_line(
            &json!({"type":"result","subtype":"error_during_execution","is_error":true})
                .to_string(),
        );
        assert_eq!(
            int.events,
            vec![AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 4,
                status: TurnStatus::Interrupted
            }]
        );
    }

    #[test]
    fn garbage_lines_are_ignored() {
        let mut d = dec();
        let out = d.decode_line("not json");
        assert!(out.events.is_empty() && !out.turn_finished);
    }
}
