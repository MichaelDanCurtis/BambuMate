//! The Health summary that leads Settings (spec 2026-10-17 §4, amended
//! 2026-10-22). Only Bambu Studio and the profile folder are required; API
//! keys are optional (one provider is enough, a local model needs none), so
//! they're counted but never an issue.

use crate::commands::HealthReport;
use crate::components::patterns::Tone;

/// Checks that must pass.
const REQUIRED: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct HealthSummary {
    pub word: String,
    pub tone: Tone,
    pub issues: usize,
    pub keys_set: usize,
}

pub fn health_summary(r: &HealthReport) -> HealthSummary {
    let issues = [r.bambu_studio_installed, r.profile_dir_accessible]
        .iter()
        .filter(|ok| !**ok)
        .count();
    let keys_set = [
        r.claude_api_key_set,
        r.openai_api_key_set,
        r.kimi_api_key_set,
        r.openrouter_api_key_set,
    ]
    .iter()
    .filter(|k| **k)
    .count();
    let (word, tone) = match issues {
        0 => ("ALL GOOD".to_string(), Tone::Signal),
        1 => ("1 ISSUE".to_string(), Tone::Accent),
        n => (format!("{n} ISSUES"), Tone::Accent),
    };
    HealthSummary {
        word,
        tone,
        issues,
        keys_set,
    }
}

pub fn health_detail(s: &HealthSummary, ago: &str) -> String {
    let keys = if s.keys_set == 1 {
        "1 API key set".to_string()
    } else {
        format!("{} API keys set", s.keys_set)
    };
    if ago.is_empty() {
        format!("{REQUIRED} required checks · {keys}")
    } else {
        format!("{REQUIRED} required checks · {keys} · last run {ago}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(bs: bool, dir: bool, keys: [bool; 4]) -> HealthReport {
        HealthReport {
            bambu_studio_installed: bs,
            bambu_studio_path: None,
            profile_dir_accessible: dir,
            profile_dir_path: None,
            claude_api_key_set: keys[0],
            openai_api_key_set: keys[1],
            kimi_api_key_set: keys[2],
            openrouter_api_key_set: keys[3],
        }
    }

    #[test]
    fn only_required_checks_count_as_issues() {
        let s = health_summary(&report(true, true, [false; 4]));
        assert_eq!(
            (s.word.as_str(), s.tone, s.issues, s.keys_set),
            ("ALL GOOD", Tone::Signal, 0, 0)
        );
        let s = health_summary(&report(false, true, [true, false, false, false]));
        assert_eq!(
            (s.word.as_str(), s.tone, s.issues, s.keys_set),
            ("1 ISSUE", Tone::Accent, 1, 1)
        );
        let s = health_summary(&report(false, false, [true; 4]));
        assert_eq!((s.word.as_str(), s.issues, s.keys_set), ("2 ISSUES", 2, 4));
    }

    #[test]
    fn the_detail_says_what_was_checked() {
        let s = health_summary(&report(true, true, [true, false, false, false]));
        assert_eq!(
            health_detail(&s, "just now"),
            "2 required checks · 1 API key set · last run just now"
        );
        let s = health_summary(&report(true, true, [true, true, false, false]));
        assert_eq!(health_detail(&s, ""), "2 required checks · 2 API keys set");
        assert_eq!(
            health_detail(&s, "5m ago"),
            "2 required checks · 2 API keys set · last run 5m ago"
        );
    }
}
