//! Pure chat-state reducer for the agent drawer. Host-testable.

use super::types::{AgentEvent, AskRequest, RewindPlan, TodoItem, TurnStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Tool,
    File,
    Command,
    Search,
    Image,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    User {
        seq: Option<u32>,
        text: String,
        images: Vec<String>,
    },
    Agent {
        item_id: String,
        text: String,
        done: bool,
    },
    Activity {
        id: String,
        kind: ActivityKind,
        title: String,
        detail: String,
        ok: Option<bool>,
    },
    Ask {
        request: AskRequest,
        answered: Option<Vec<String>>,
    },
    Notice {
        text: String,
        is_error: bool,
    },
    /// Asks before a rewind to `seq`, listing the profile files (names only)
    /// it would delete or overwrite.
    RewindConfirm {
        seq: u32,
        delete: Vec<String>,
        overwrite: Vec<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatState {
    pub session_id: Option<String>,
    pub entries: Vec<Entry>,
    pub running: bool,
    pub todos: Vec<TodoItem>,
    pub used_percent: Option<f64>,
    pub invalid: Vec<String>,
    /// Counts TurnStarted/TurnDone events for this session, so the drawer can
    /// tell a turn event arrived even when `running` ends where it started.
    pub turn_events: u32,
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

impl ChatState {
    pub fn push_user(&mut self, text: String, images: Vec<String>) {
        self.entries.push(Entry::User {
            seq: None,
            text,
            images,
        });
    }

    fn activity(
        &mut self,
        id: String,
        kind: ActivityKind,
        title: String,
        detail: String,
        ok: Option<bool>,
    ) {
        self.entries.push(Entry::Activity {
            id,
            kind,
            title,
            detail,
            ok,
        });
    }

    pub fn apply(&mut self, ev: &AgentEvent) {
        // Session-less errors (e.g. a backend crash) always show.
        if let Some(sid) = ev.session_id() {
            if Some(sid) != self.session_id.as_deref() {
                return;
            }
        }
        match ev {
            AgentEvent::SessionReady { .. } => {}
            AgentEvent::TurnStarted { seq, .. } => {
                self.running = true;
                self.turn_events = self.turn_events.wrapping_add(1);
                if let Some(Entry::User { seq: s, .. }) = self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, Entry::User { seq: None, .. }))
                {
                    *s = Some(*seq);
                }
            }
            AgentEvent::MessageDelta { item_id, text, .. } => {
                match self.entries.iter_mut().rev().find(
                    |e| matches!(e, Entry::Agent { item_id: i, done: false, .. } if i == item_id),
                ) {
                    Some(Entry::Agent { text: t, .. }) => t.push_str(text),
                    _ => self.entries.push(Entry::Agent {
                        item_id: item_id.clone(),
                        text: text.clone(),
                        done: false,
                    }),
                }
            }
            AgentEvent::MessageDone { item_id, text, .. } => {
                match self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, Entry::Agent { item_id: i, .. } if i == item_id))
                {
                    Some(Entry::Agent { text: t, done, .. }) => {
                        *t = text.clone();
                        *done = true;
                    }
                    _ => self.entries.push(Entry::Agent {
                        item_id: item_id.clone(),
                        text: text.clone(),
                        done: true,
                    }),
                }
            }
            AgentEvent::ToolCall {
                call_id,
                name,
                args,
                ..
            } => self.activity(
                call_id.clone(),
                ActivityKind::Tool,
                name.clone(),
                clip(&args.to_string(), 120),
                None,
            ),
            AgentEvent::ToolResult {
                call_id,
                ok,
                summary,
                ..
            } => {
                if let Some(Entry::Activity { ok: o, detail, .. }) = self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, Entry::Activity { id, .. } if id == call_id))
                {
                    *o = Some(*ok);
                    *detail = clip(summary, 200);
                }
            }
            AgentEvent::FileChange { path, diff, .. } => self.activity(
                format!("file-{}-{path}", self.entries.len()),
                ActivityKind::File,
                file_name(path),
                clip(diff, 600),
                Some(true),
            ),
            AgentEvent::Command {
                command, exit_code, ..
            } => self.activity(
                format!("cmd-{}", self.entries.len()),
                ActivityKind::Command,
                clip(command, 80),
                exit_code.map(|c| format!("exit {c}")).unwrap_or_default(),
                exit_code.map(|c| c == 0),
            ),
            AgentEvent::WebSearch { query, .. } => self.activity(
                format!("web-{}", self.entries.len()),
                ActivityKind::Search,
                clip(query, 80),
                String::new(),
                Some(true),
            ),
            AgentEvent::ImageGenerated { path, .. } => self.activity(
                format!("img-{}", self.entries.len()),
                ActivityKind::Image,
                file_name(path),
                path.clone(),
                Some(true),
            ),
            AgentEvent::Ask { request, .. } => self.entries.push(Entry::Ask {
                request: request.clone(),
                answered: None,
            }),
            AgentEvent::Todo { items, .. } => self.todos = items.clone(),
            AgentEvent::Usage { used_percent, .. } => self.used_percent = *used_percent,
            AgentEvent::TurnDone { status, .. } => {
                self.running = false;
                self.turn_events = self.turn_events.wrapping_add(1);
                if *status == TurnStatus::Interrupted {
                    self.entries.push(Entry::Notice {
                        text: "Stopped.".into(),
                        is_error: false,
                    });
                }
            }
            AgentEvent::InvalidProfiles { paths, .. } => {
                self.invalid = paths.clone();
                let names: Vec<String> = paths.iter().map(|p| file_name(p)).collect();
                self.entries.push(Entry::Notice {
                    text: format!(
                        "Bambu Studio may reject: {}. Rewind this turn to restore them.",
                        names.join(", ")
                    ),
                    is_error: true,
                });
            }
            AgentEvent::Error { message, .. } => self.entries.push(Entry::Notice {
                text: message.clone(),
                is_error: true,
            }),
        }
    }

    pub fn mark_answered(&mut self, ask_id: &str, answers: Vec<String>) {
        if let Some(Entry::Ask { answered, .. }) = self
            .entries
            .iter_mut()
            .find(|e| matches!(e, Entry::Ask { request, .. } if request.id == ask_id))
        {
            *answered = Some(answers);
        }
    }

    /// Undo `mark_answered` when delivering the answer failed, so the user can retry.
    pub fn unmark_answered(&mut self, ask_id: &str) {
        if let Some(Entry::Ask { answered, .. }) = self
            .entries
            .iter_mut()
            .find(|e| matches!(e, Entry::Ask { request, .. } if request.id == ask_id))
        {
            *answered = None;
        }
    }

    /// Shows the confirm card for a rewind to `seq`, replacing any other.
    pub fn show_rewind_confirm(&mut self, seq: u32, plan: &RewindPlan) {
        self.dismiss_rewind_confirm();
        let names = |paths: &[String]| paths.iter().map(|p| file_name(p)).collect();
        self.entries.push(Entry::RewindConfirm {
            seq,
            delete: names(&plan.delete),
            overwrite: names(&plan.overwrite),
        });
    }

    pub fn dismiss_rewind_confirm(&mut self) {
        self.entries
            .retain(|e| !matches!(e, Entry::RewindConfirm { .. }));
    }

    pub fn rewind_to(&mut self, seq: u32) {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| matches!(e, Entry::User { seq: Some(s), .. } if *s == seq))
        {
            self.entries.truncate(pos);
        }
        self.running = false;
        self.invalid.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::*;

    fn ev(json: &str) -> AgentEvent {
        serde_json::from_str(json).unwrap()
    }

    fn state() -> ChatState {
        ChatState {
            session_id: Some("s1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn deltas_accumulate_then_done_finalizes() {
        let mut s = state();
        s.push_user("hi".into(), vec![]);
        s.apply(&ev(r#"{"kind":"turn_started","session_id":"s1","seq":1}"#));
        s.apply(&ev(
            r#"{"kind":"message_delta","session_id":"s1","item_id":"m","text":"Hel"}"#,
        ));
        s.apply(&ev(
            r#"{"kind":"message_delta","session_id":"s1","item_id":"m","text":"lo"}"#,
        ));
        assert!(s.running);
        assert_eq!(
            s.entries[1],
            Entry::Agent {
                item_id: "m".into(),
                text: "Hello".into(),
                done: false
            }
        );
        s.apply(&ev(
            r#"{"kind":"message_done","session_id":"s1","item_id":"m","text":"Hello."}"#,
        ));
        assert_eq!(
            s.entries[1],
            Entry::Agent {
                item_id: "m".into(),
                text: "Hello.".into(),
                done: true
            }
        );
        assert_eq!(
            s.entries[0],
            Entry::User {
                seq: Some(1),
                text: "hi".into(),
                images: vec![]
            }
        );
    }

    #[test]
    fn tool_call_then_result_updates_one_activity() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"tool_call","session_id":"s1","call_id":"c1","name":"bm_read_profile","args":{"path":"A.json"}}"#));
        s.apply(&ev(r#"{"kind":"tool_result","session_id":"s1","call_id":"c1","ok":true,"summary":"{name: A}"}"#));
        assert_eq!(s.entries.len(), 1);
        match &s.entries[0] {
            Entry::Activity {
                kind,
                title,
                ok,
                detail,
                ..
            } => {
                assert_eq!(*kind, ActivityKind::Tool);
                assert_eq!(title, "bm_read_profile");
                assert_eq!(*ok, Some(true));
                assert_eq!(detail, "{name: A}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn events_for_other_sessions_are_ignored() {
        let mut s = state();
        s.apply(&ev(
            r#"{"kind":"web_search","session_id":"other","query":"x"}"#,
        ));
        assert!(s.entries.is_empty());
    }

    #[test]
    fn ask_is_shown_and_can_be_marked_answered() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"ask","session_id":"s1","request":{"id":"a1","header":"Confirm","question":"Install?","options":[{"label":"Yes","description":""},{"label":"No","description":""}],"allow_other":false}}"#));
        s.mark_answered("a1", vec!["Yes".into()]);
        assert!(
            matches!(&s.entries[0], Entry::Ask { answered: Some(a), .. } if a == &vec!["Yes".to_string()])
        );
    }

    #[test]
    fn failed_answer_reverts_the_ask_to_unanswered() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"ask","session_id":"s1","request":{"id":"a1","header":"Confirm","question":"Install?","options":[{"label":"Yes","description":""}],"allow_other":false}}"#));
        s.mark_answered("a1", vec!["Yes".into()]);
        s.unmark_answered("a1");
        assert!(matches!(&s.entries[0], Entry::Ask { answered: None, .. }));
    }

    #[test]
    fn turn_done_stops_running_and_errors_become_notices() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"turn_started","session_id":"s1","seq":1}"#));
        s.apply(&ev(
            r#"{"kind":"error","session_id":"s1","message":"usage limit"}"#,
        ));
        s.apply(&ev(
            r#"{"kind":"turn_done","session_id":"s1","seq":1,"status":"failed"}"#,
        ));
        assert!(!s.running);
        assert!(
            matches!(&s.entries[0], Entry::Notice { is_error: true, text } if text == "usage limit")
        );
    }

    #[test]
    fn todo_usage_and_invalid_profiles_update_state() {
        let mut s = state();
        s.apply(&ev(
            r#"{"kind":"todo","session_id":"s1","items":[{"text":"Read","done":true}]}"#,
        ));
        s.apply(&ev(
            r#"{"kind":"usage","session_id":"s1","used_percent":37.0,"resets_at":null}"#,
        ));
        s.apply(&ev(
            r#"{"kind":"invalid_profiles","session_id":"s1","seq":1,"paths":["/p/B.json"]}"#,
        ));
        assert_eq!(s.todos.len(), 1);
        assert_eq!(s.used_percent, Some(37.0));
        assert_eq!(s.invalid, vec!["/p/B.json".to_string()]);
        assert!(matches!(
            s.entries.last(),
            Some(Entry::Notice { is_error: true, .. })
        ));
    }

    #[test]
    fn rewind_drops_the_message_and_everything_after() {
        let mut s = state();
        for (i, t) in ["one", "two", "three"].iter().enumerate() {
            s.push_user(t.to_string(), vec![]);
            s.apply(&ev(&format!(
                r#"{{"kind":"turn_started","session_id":"s1","seq":{}}}"#,
                i + 1
            )));
            s.apply(&ev(&format!(
                r#"{{"kind":"message_done","session_id":"s1","item_id":"m{i}","text":"ok"}}"#
            )));
        }
        s.rewind_to(2);
        assert_eq!(s.entries.len(), 2);
        assert!(matches!(&s.entries[0], Entry::User { seq: Some(1), .. }));
    }

    #[test]
    fn rewind_confirm_lists_file_names_and_replaces_an_earlier_one() {
        let mut s = state();
        s.push_user("one".into(), vec![]);
        let plan = RewindPlan {
            delete: vec!["/p/New.json".into()],
            overwrite: vec!["C:\\u\\A.json".into(), "/p/B.info".into()],
        };
        s.show_rewind_confirm(1, &RewindPlan::default());
        s.show_rewind_confirm(1, &plan);
        let confirms: Vec<&Entry> = s
            .entries
            .iter()
            .filter(|e| matches!(e, Entry::RewindConfirm { .. }))
            .collect();
        assert_eq!(
            confirms,
            vec![&Entry::RewindConfirm {
                seq: 1,
                delete: vec!["New.json".into()],
                overwrite: vec!["A.json".into(), "B.info".into()],
            }]
        );
        s.dismiss_rewind_confirm();
        assert_eq!(s.entries.len(), 1, "only the user message is left");
    }

    #[test]
    fn turn_events_count_starts_and_ends_of_this_sessions_turns() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"turn_started","session_id":"s1","seq":1}"#));
        s.apply(&ev(
            r#"{"kind":"turn_done","session_id":"s1","seq":1,"status":"failed"}"#,
        ));
        s.apply(&ev(
            r#"{"kind":"turn_started","session_id":"other","seq":1}"#,
        ));
        assert_eq!(s.turn_events, 2);
        assert!(!s.running);
    }
}
