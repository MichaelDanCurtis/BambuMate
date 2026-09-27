//! Questions from the agent to the user, parked until the UI answers.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::{broadcast, oneshot};

use super::types::{AgentEvent, AskOption, AskRequest};

type PendingAsk = (String, oneshot::Sender<Vec<String>>);

pub struct AskBroker {
    pending: Mutex<HashMap<String, PendingAsk>>,
    events: broadcast::Sender<AgentEvent>,
}

impl Clone for AskBroker {
    fn clone(&self) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            events: self.events.clone(),
        }
    }
}

impl AskBroker {
    pub fn new(events: broadcast::Sender<AgentEvent>) -> Self {
        Self { pending: Mutex::new(HashMap::new()), events }
    }

    pub async fn ask(
        &self,
        session_id: &str,
        header: &str,
        question: &str,
        options: Vec<AskOption>,
        allow_other: bool,
    ) -> Result<Vec<String>, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), (session_id.to_string(), tx));
        let _ = self.events.send(AgentEvent::Ask {
            session_id: session_id.to_string(),
            request: AskRequest {
                id,
                header: header.to_string(),
                question: question.to_string(),
                options,
                allow_other,
            },
        });
        rx.await.map_err(|_| "question was cancelled".to_string())
    }

    pub async fn confirm(&self, session_id: &str, prompt: &str) -> bool {
        let options = vec![
            AskOption { label: "Yes".into(), description: "Go ahead".into() },
            AskOption { label: "No".into(), description: "Don't do it".into() },
        ];
        matches!(
            self.ask(session_id, "Confirm", prompt, options, false).await.as_deref(),
            Ok([first, ..]) if first == "Yes"
        )
    }

    pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String> {
        let (_, tx) = self
            .pending
            .lock()
            .unwrap()
            .remove(ask_id)
            .ok_or_else(|| format!("no pending question {ask_id}"))?;
        tx.send(answers).map_err(|_| "asker is gone".to_string())
    }

    pub fn cancel_session(&self, session_id: &str) {
        self.pending.lock().unwrap().retain(|_, (s, _)| s != session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn broker() -> (Arc<AskBroker>, broadcast::Receiver<AgentEvent>) {
        let (tx, rx) = broadcast::channel(16);
        (Arc::new(AskBroker::new(tx)), rx)
    }

    async fn next_ask(rx: &mut broadcast::Receiver<AgentEvent>) -> AskRequest {
        match rx.recv().await.unwrap() {
            AgentEvent::Ask { request, .. } => request,
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ask_emits_event_and_returns_answer() {
        let (b, mut rx) = broker();
        let b2 = b.clone();
        let task = tokio::spawn(async move {
            b2.ask("s1", "Nozzle", "Which nozzle?", vec![AskOption { label: "0.4".into(), description: "".into() }], false)
                .await
        });
        let req = next_ask(&mut rx).await;
        assert_eq!(req.question, "Which nozzle?");
        b.answer(&req.id, vec!["0.4".into()]).unwrap();
        assert_eq!(task.await.unwrap().unwrap(), vec!["0.4".to_string()]);
    }

    #[tokio::test]
    async fn confirm_is_true_only_for_yes() {
        let (b, mut rx) = broker();
        for (answer, expected) in [("Yes", true), ("No", false)] {
            let b2 = b.clone();
            let task = tokio::spawn(async move { b2.confirm("s1", "Install anyway?").await });
            let req = next_ask(&mut rx).await;
            assert_eq!(req.options.len(), 2);
            b.answer(&req.id, vec![answer.into()]).unwrap();
            assert_eq!(task.await.unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn cancel_session_fails_pending_asks() {
        let (b, mut rx) = broker();
        let b2 = b.clone();
        let task = tokio::spawn(async move { b2.ask("s1", "h", "q", vec![], true).await });
        let _ = next_ask(&mut rx).await;
        b.cancel_session("s1");
        assert!(task.await.unwrap().is_err());
    }

    #[test]
    fn answering_unknown_ask_is_an_error() {
        let (b, _rx) = broker();
        assert!(b.answer("nope", vec![]).is_err());
    }
}
