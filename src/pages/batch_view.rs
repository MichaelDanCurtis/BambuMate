//! What Batch shows after a run (spec 2026-10-17 §4). The backend returns
//! the whole batch at once, so there is no live progress.

pub use crate::commands::BatchProgress;

/// Segments in the results bar.
pub const BATCH_SEGMENTS: u32 = 12;

pub fn batch_line(p: &BatchProgress) -> String {
    match (p.total, p.failed) {
        (0, _) => "Nothing to generate".into(),
        (_, 0) => format!("All {} generated", p.total),
        (_, n) => format!("{n} failed"),
    }
}

pub fn batch_percent(p: &BatchProgress) -> u32 {
    if p.total == 0 {
        0
    } else {
        (p.succeeded * 100 / p.total) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(total: usize, succeeded: usize) -> BatchProgress {
        BatchProgress {
            total,
            completed: total,
            succeeded,
            failed: total - succeeded,
            results: vec![],
        }
    }

    #[test]
    fn the_line_says_what_failed() {
        assert_eq!(batch_line(&run(3, 3)), "All 3 generated");
        assert_eq!(batch_line(&run(3, 2)), "1 failed");
        assert_eq!(batch_line(&run(5, 1)), "4 failed");
    }

    #[test]
    fn an_empty_batch_says_there_was_nothing_to_do() {
        assert_eq!(batch_line(&run(0, 0)), "Nothing to generate");
    }

    #[test]
    fn percent_is_the_share_that_succeeded() {
        assert_eq!(batch_percent(&run(3, 2)), 66);
        assert_eq!(batch_percent(&run(4, 4)), 100);
        assert_eq!(batch_percent(&run(0, 0)), 0);
    }
}
