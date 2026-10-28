//! The moments a print is photographed at, read off the watcher's events.
//! A moment whose window was missed (the app joined at 70 %) is skipped,
//! not taken late. Taking each once is the store's job (`claim_snapshot`).

use super::types::Outcome;
use super::watch::JournalEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Moment {
    FirstLayer,
    Half,
    Finished,
    Failed,
    Paused,
}

impl Moment {
    /// The key kept in the entry's snapshot log.
    pub fn key(self) -> &'static str {
        match self {
            Moment::FirstLayer => "first_layer",
            Moment::Half => "half",
            Moment::Finished => "finished",
            Moment::Failed => "failed",
            Moment::Paused => "paused",
        }
    }

    /// The photo's label, as the page shows it.
    pub fn label(self) -> &'static str {
        match self {
            Moment::FirstLayer => "FIRST LAYER",
            Moment::Half => "50 %",
            Moment::Finished => "FINISHED",
            Moment::Failed => "FAILED",
            Moment::Paused => "PAUSED",
        }
    }
}

/// The moments `event` reaches, with the job key they belong to.
pub fn due(event: &JournalEvent) -> Vec<(&str, Moment)> {
    match event {
        JournalEvent::Progress {
            key,
            percent,
            layer,
        } => {
            let mut out = Vec::new();
            if matches!(layer, Some(2..=3)) {
                out.push((key.as_str(), Moment::FirstLayer));
            }
            if (50..=55).contains(percent) {
                out.push((key.as_str(), Moment::Half));
            }
            out
        }
        JournalEvent::Ended {
            key,
            outcome: Outcome::Finished,
            ..
        } => vec![(key.as_str(), Moment::Finished)],
        JournalEvent::Ended {
            key,
            outcome: Outcome::Failed,
            ..
        } => vec![(key.as_str(), Moment::Failed)],
        JournalEvent::Paused { key } => vec![(key.as_str(), Moment::Paused)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(percent: u32, layer: Option<u32>) -> JournalEvent {
        JournalEvent::Progress {
            key: "k".into(),
            percent,
            layer,
        }
    }

    #[test]
    fn the_first_layer_and_half_way_have_windows() {
        assert_eq!(due(&progress(1, Some(1))), vec![]);
        assert_eq!(due(&progress(2, Some(2))), vec![("k", Moment::FirstLayer)]);
        assert_eq!(due(&progress(3, Some(3))), vec![("k", Moment::FirstLayer)]);
        assert_eq!(due(&progress(4, Some(4))), vec![]);
        assert_eq!(due(&progress(4, None)), vec![]);
        assert_eq!(due(&progress(49, Some(80))), vec![]);
        assert_eq!(due(&progress(50, Some(80))), vec![("k", Moment::Half)]);
        assert_eq!(due(&progress(55, Some(90))), vec![("k", Moment::Half)]);
        assert_eq!(due(&progress(56, Some(91))), vec![]);
        assert_eq!(
            due(&progress(52, Some(2))),
            vec![("k", Moment::FirstLayer), ("k", Moment::Half)]
        );
    }

    #[test]
    fn ends_and_error_pauses_are_moments() {
        let ended = |outcome| JournalEvent::Ended {
            key: "k".into(),
            outcome,
            ended_at: 0,
            error_code: None,
            hms: vec![],
        };
        assert_eq!(
            due(&ended(Outcome::Finished)),
            vec![("k", Moment::Finished)]
        );
        assert_eq!(due(&ended(Outcome::Failed)), vec![("k", Moment::Failed)]);
        assert_eq!(due(&ended(Outcome::Unknown)), vec![]);
        assert_eq!(
            due(&JournalEvent::Paused { key: "k".into() }),
            vec![("k", Moment::Paused)]
        );
        assert_eq!(due(&JournalEvent::Lost { key: "k".into() }), vec![]);
    }

    #[test]
    fn labels_are_the_page_text() {
        let all = [
            Moment::FirstLayer,
            Moment::Half,
            Moment::Finished,
            Moment::Failed,
            Moment::Paused,
        ];
        let labels: Vec<_> = all.iter().map(|m| m.label()).collect();
        assert_eq!(
            labels,
            ["FIRST LAYER", "50 %", "FINISHED", "FAILED", "PAUSED"]
        );
    }
}
