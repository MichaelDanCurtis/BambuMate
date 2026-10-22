//! HMS codes the printer reported this session, newest first, so Printer
//! can show what came and went. Session only; nothing is stored.

use crate::printer::types::ErrorView;

/// Most entries kept.
const CAP: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SeenError {
    pub code: String,
    pub text: Option<String>,
    pub first_seen_ms: f64,
}

pub fn merge_seen(history: &[SeenError], active: &[ErrorView], now_ms: f64) -> Vec<SeenError> {
    // New codes from this report, in the printer's order, ahead of
    // everything older.
    let mut out: Vec<SeenError> = active
        .iter()
        .filter(|e| !history.iter().any(|h| h.code == e.code))
        .map(|e| SeenError {
            code: e.code.clone(),
            text: e.text.clone(),
            first_seen_ms: now_ms,
        })
        .collect();
    out.extend(history.iter().cloned());
    out.truncate(CAP);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(code: &str) -> ErrorView {
        ErrorView {
            kind: "hms".into(),
            code: code.into(),
            text: Some(format!("text {code}")),
            wiki_url: String::new(),
        }
    }

    #[test]
    fn new_codes_go_first_and_repeats_are_kept_once() {
        let h = merge_seen(&[], &[err("A"), err("B")], 1.0);
        assert_eq!(
            h.iter().map(|e| e.code.as_str()).collect::<Vec<_>>(),
            ["A", "B"]
        );
        let h = merge_seen(&h, &[err("B"), err("C")], 2.0);
        assert_eq!(
            h.iter().map(|e| e.code.as_str()).collect::<Vec<_>>(),
            ["C", "A", "B"]
        );
        assert_eq!(h[0].first_seen_ms, 2.0);
        assert_eq!(h[0].text.as_deref(), Some("text C"));
    }

    #[test]
    fn the_history_is_capped() {
        let mut h = Vec::new();
        for i in 0..30 {
            h = merge_seen(&h, &[err(&format!("E{i}"))], i as f64);
        }
        assert_eq!(h.len(), 20);
        assert_eq!(h[0].code, "E29");
    }
}
