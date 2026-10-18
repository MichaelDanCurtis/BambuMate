//! Remembers the print that finished most recently, for Home's idle hero.
//! Session only: nothing is written to disk.

/// A print the app watched finish.
#[derive(Debug, Clone, PartialEq)]
pub struct LastPrint {
    /// The file name the printer reported. Untrusted: render it through
    /// `untrusted_text`.
    pub name: String,
    pub finished_at_ms: f64,
    /// Known only when the app saw the print start.
    pub duration_ms: Option<f64>,
}

/// What the tracker knows about the job in progress.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PrintTracker {
    /// When the current job was first seen active, if one is.
    active_since_ms: Option<f64>,
    /// Whether that first sighting was the job's start.
    saw_start: bool,
}

/// `gcode_state`s of a job that hasn't ended.
const ACTIVE: &[&str] = &["PREPARE", "SLICING", "RUNNING", "PAUSE"];

/// Feeds one status report to the tracker. Returns the finished print on a
/// transition from an active state to `FINISH`, and nothing otherwise.
pub fn track_print(
    t: PrintTracker,
    gcode: Option<&str>,
    name: Option<&str>,
    percent: Option<u32>,
    now_ms: f64,
) -> (PrintTracker, Option<LastPrint>) {
    let active = gcode.is_some_and(|g| ACTIVE.contains(&g));
    match t.active_since_ms {
        None if active => (
            PrintTracker {
                active_since_ms: Some(now_ms),
                saw_start: gcode == Some("PREPARE") || percent == Some(0),
            },
            None,
        ),
        Some(_) if active => (t, None),
        Some(since) if gcode == Some("FINISH") => (
            PrintTracker::default(),
            Some(LastPrint {
                name: name.unwrap_or_default().to_string(),
                finished_at_ms: now_ms,
                duration_ms: t.saw_start.then_some(now_ms - since),
            }),
        ),
        _ => (PrintTracker::default(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(steps: &[(&str, Option<u32>, f64)]) -> Option<LastPrint> {
        let mut t = PrintTracker::default();
        let mut out = None;
        for &(gcode, percent, now) in steps {
            let (next, done) = track_print(t, Some(gcode), Some("Benchy.3mf"), percent, now);
            t = next;
            out = done.or(out);
        }
        out
    }

    #[test]
    fn a_print_seen_from_its_start_records_its_duration() {
        let p = feed(&[
            ("PREPARE", Some(0), 1_000.0),
            ("RUNNING", Some(50), 2_000.0),
            ("FINISH", Some(100), 7_000.0),
        ])
        .unwrap();
        assert_eq!(p.name, "Benchy.3mf");
        assert_eq!(p.finished_at_ms, 7_000.0);
        assert_eq!(p.duration_ms, Some(6_000.0));
    }

    #[test]
    fn a_print_joined_midway_has_no_duration() {
        let p = feed(&[
            ("RUNNING", Some(40), 1_000.0),
            ("FINISH", Some(100), 5_000.0),
        ])
        .unwrap();
        assert_eq!(p.duration_ms, None);
    }

    #[test]
    fn a_pause_does_not_end_the_print() {
        let p = feed(&[
            ("RUNNING", Some(0), 1_000.0),
            ("PAUSE", Some(30), 2_000.0),
            ("RUNNING", Some(31), 3_000.0),
            ("FINISH", Some(100), 4_000.0),
        ])
        .unwrap();
        assert_eq!(p.duration_ms, Some(3_000.0));
    }

    #[test]
    fn only_running_to_finish_counts() {
        // Failed, never seen running, or idle throughout: nothing recorded.
        assert_eq!(
            feed(&[("RUNNING", Some(5), 1_000.0), ("FAILED", Some(5), 2_000.0)]),
            None
        );
        assert_eq!(feed(&[("FINISH", Some(100), 1_000.0)]), None);
        assert_eq!(
            feed(&[("IDLE", None, 1_000.0), ("IDLE", None, 2_000.0)]),
            None
        );
        // A finished print is reported once, not on every later FINISH report.
        assert_eq!(
            feed(&[
                ("RUNNING", Some(5), 1_000.0),
                ("FINISH", Some(100), 2_000.0),
                ("FINISH", Some(100), 3_000.0)
            ])
            .map(|p| p.finished_at_ms),
            Some(2_000.0)
        );
    }
}
