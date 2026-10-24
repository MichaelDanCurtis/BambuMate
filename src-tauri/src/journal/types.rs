//! What the journal stores and the page shows. The serde names are the
//! wire format the frontend mirrors in `src/journal/types.rs`.

use serde::{Deserialize, Serialize};

/// How a print ended. `Printing` while the entry is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Printing,
    Finished,
    Failed,
    /// The job disappeared while BambuMate wasn't watching.
    Unknown,
}

impl Outcome {
    /// The `outcome` column.
    pub fn key(self) -> &'static str {
        match self {
            Outcome::Printing => "printing",
            Outcome::Finished => "finished",
            Outcome::Failed => "failed",
            Outcome::Unknown => "unknown",
        }
    }

    /// Anything unrecognised reads as `Unknown`, never as a guess.
    pub fn from_key(key: &str) -> Outcome {
        match key {
            "printing" => Outcome::Printing,
            "finished" => Outcome::Finished,
            "failed" => Outcome::Failed,
            _ => Outcome::Unknown,
        }
    }
}

/// The user's verdict on a print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Good,
    Usable,
    Failed,
}

impl Verdict {
    pub fn key(self) -> &'static str {
        match self {
            Verdict::Good => "good",
            Verdict::Usable => "usable",
            Verdict::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Verdict> {
        match key {
            "good" => Some(Verdict::Good),
            "usable" => Some(Verdict::Usable),
            "failed" => Some(Verdict::Failed),
            _ => None,
        }
    }
}

/// A verdict change: one of the verdicts, or `Clear` to remove it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictChoice {
    Good,
    Usable,
    Failed,
    Clear,
}

impl VerdictChoice {
    pub fn verdict(self) -> Option<Verdict> {
        match self {
            VerdictChoice::Good => Some(Verdict::Good),
            VerdictChoice::Usable => Some(Verdict::Usable),
            VerdictChoice::Failed => Some(Verdict::Failed),
            VerdictChoice::Clear => None,
        }
    }
}

/// A tray the printer reported as current during the print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrayUse {
    pub ams_id: u32,
    pub tray_id: u32,
    /// `A2`, `HT1`, `Ext-L`, …
    pub label: String,
    pub tray_type: String,
    /// `RRGGBBAA`, as the printer reports it.
    pub color: String,
    /// `tray_sub_brands`.
    pub brand: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceFilament {
    pub slot: u32,
    pub filament_type: String,
    pub color: String,
    pub used_g: f64,
}

/// What the linked slice said, copied in: the slice cache can be cleared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceSnapshot {
    pub model_name: String,
    pub model_path: String,
    pub printer: String,
    pub process: String,
    pub filament: String,
    pub plate: u32,
    pub time_seconds: u64,
    pub weight_g: f64,
    pub filaments: Vec<SliceFilament>,
    /// The plate thumbnail was saved to `thumbs/<entry-id>.png`.
    #[serde(default)]
    pub has_thumbnail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Photo {
    pub id: i64,
    pub file_name: String,
    pub added_at: i64,
}

/// One print. Times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintEntry {
    pub id: i64,
    pub job_key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    pub printer_model: Option<String>,
    pub started_at: i64,
    pub start_approx: bool,
    pub ended_at: Option<i64>,
    pub outcome: Outcome,
    pub error_code: Option<u32>,
    /// HMS codes in the `0300_0100_0001_0007` form.
    pub hms: Vec<String>,
    pub verdict: Option<Verdict>,
    pub tags: Vec<String>,
    pub notes: String,
    pub slice: Option<SliceSnapshot>,
    pub trays: Vec<TrayUse>,
    pub photos: Vec<Photo>,
    /// `refinement_sessions` ids of the analyses made from this entry.
    pub analyses: Vec<i64>,
    pub updated_at: i64,
}

/// A print as the recorder opens it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPrint {
    pub job_key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    pub printer_model: Option<String>,
    pub started_at: i64,
    pub start_approx: bool,
}

/// How the recorder closes a print.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintEnd {
    pub outcome: Outcome,
    /// `None` for `Unknown`: when it ended was not seen.
    pub ended_at: Option<i64>,
    pub error_code: Option<u32>,
    pub hms: Vec<String>,
}

impl PrintEnd {
    pub fn unknown() -> Self {
        Self {
            outcome: Outcome::Unknown,
            ended_at: None,
            error_code: None,
            hms: Vec::new(),
        }
    }
}

/// An entry still printing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenEntry {
    pub id: i64,
    pub job_key: String,
    /// Unix seconds; back-dated when the start was approximate.
    pub started_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    #[default]
    All,
    /// Ended, with no verdict yet.
    NeedsVerdict,
    /// A Failed verdict, or no verdict and a failed outcome.
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JournalFilter {
    pub kind: FilterKind,
    /// Matches the name, the file, the notes and the trays' type and brand.
    pub query: String,
}

/// Most characters kept of an entry's notes.
pub const NOTES_MAX: usize = 10_000;

/// An annotation change. `None` leaves that field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EntryUpdate {
    pub verdict: Option<VerdictChoice>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
}

impl EntryUpdate {
    /// Keeps only the known tags, each once and in the order given, and
    /// cuts the notes to [`NOTES_MAX`] characters.
    pub fn cleaned(mut self, known_tags: &[String]) -> Self {
        if let Some(tags) = self.tags.take() {
            let mut kept: Vec<String> = Vec::new();
            for tag in tags {
                let tag = tag.trim().to_string();
                if known_tags.contains(&tag) && !kept.contains(&tag) {
                    kept.push(tag);
                }
            }
            self.tags = Some(kept);
        }
        if let Some(notes) = self.notes.as_mut() {
            if notes.chars().count() > NOTES_MAX {
                *notes = notes.chars().take(NOTES_MAX).collect();
            }
        }
        self
    }
}

/// The Journal hero's tally for one month.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthSummary {
    pub prints: u32,
    pub good: u32,
    pub usable: u32,
    /// Finished, with no verdict yet.
    pub finished: u32,
    /// A Failed verdict, or no verdict and a failed outcome.
    pub failed: u32,
    /// Summed over entries with a known end.
    pub printing_seconds: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_and_verdict_keys_round_trip() {
        for o in [
            Outcome::Printing,
            Outcome::Finished,
            Outcome::Failed,
            Outcome::Unknown,
        ] {
            assert_eq!(Outcome::from_key(o.key()), o);
        }
        assert_eq!(Outcome::from_key("cancelled"), Outcome::Unknown);
        for v in [Verdict::Good, Verdict::Usable, Verdict::Failed] {
            assert_eq!(Verdict::from_key(v.key()), Some(v));
        }
        assert_eq!(Verdict::from_key("meh"), None);
        assert_eq!(VerdictChoice::Clear.verdict(), None);
        assert_eq!(VerdictChoice::Usable.verdict(), Some(Verdict::Usable));
    }

    #[test]
    fn serde_names_match_the_stored_keys() {
        assert_eq!(
            serde_json::to_string(&Outcome::Finished).unwrap(),
            "\"finished\""
        );
        assert_eq!(serde_json::to_string(&Verdict::Good).unwrap(), "\"good\"");
        let choice: VerdictChoice = serde_json::from_str("\"clear\"").unwrap();
        assert_eq!(choice, VerdictChoice::Clear);
        let filter: JournalFilter =
            serde_json::from_str(r#"{"kind":"needs_verdict","query":"petg"}"#).unwrap();
        assert_eq!(filter.kind, FilterKind::NeedsVerdict);
        let partial: EntryUpdate = serde_json::from_str(r#"{"notes":"x"}"#).unwrap();
        assert_eq!(partial.verdict, None);
        assert_eq!(partial.notes.as_deref(), Some("x"));
    }

    #[test]
    fn updates_keep_only_known_tags_once_and_cap_the_notes() {
        let known = vec!["stringing".to_string(), "warping".to_string()];
        let u = EntryUpdate {
            tags: Some(vec![
                "warping".into(),
                " stringing ".into(),
                "warping".into(),
                "bogus".into(),
            ]),
            notes: Some("n".repeat(NOTES_MAX + 5)),
            ..Default::default()
        }
        .cleaned(&known);
        assert_eq!(
            u.tags,
            Some(vec!["warping".to_string(), "stringing".to_string()])
        );
        assert_eq!(u.notes.unwrap().chars().count(), NOTES_MAX);
        assert_eq!(
            EntryUpdate::default().cleaned(&known),
            EntryUpdate::default()
        );
    }
}
