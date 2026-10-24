//! Serde mirrors of the backend's journal types
//! (src-tauri/src/journal/types.rs). Times are Unix seconds.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Printing,
    Finished,
    Failed,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Good,
    Usable,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictChoice {
    Good,
    Usable,
    Failed,
    Clear,
}

impl From<Verdict> for VerdictChoice {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Good => VerdictChoice::Good,
            Verdict::Usable => VerdictChoice::Usable,
            Verdict::Failed => VerdictChoice::Failed,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TrayUse {
    pub ams_id: u32,
    pub tray_id: u32,
    pub label: String,
    pub tray_type: String,
    pub color: String,
    pub brand: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SliceFilament {
    pub slot: u32,
    pub filament_type: String,
    pub color: String,
    pub used_g: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
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
    pub has_thumbnail: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Photo {
    pub id: i64,
    pub file_name: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
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
    pub hms: Vec<String>,
    pub verdict: Option<Verdict>,
    pub tags: Vec<String>,
    pub notes: String,
    pub slice: Option<SliceSnapshot>,
    pub trays: Vec<TrayUse>,
    pub photos: Vec<Photo>,
    pub analyses: Vec<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    #[default]
    All,
    NeedsVerdict,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JournalFilter {
    pub kind: FilterKind,
    pub query: String,
}

/// An annotation change; `None` fields are left out and stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EntryUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MonthSummary {
    pub prints: u32,
    pub good: u32,
    pub usable: u32,
    pub finished: u32,
    pub failed: u32,
    pub printing_seconds: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_deserializes_from_the_backend_shape() {
        let json = r#"{"id":2,"job_key":"local:cube|","name":"cube","gcode_file":null,
            "plate":1,"printer_model":"H2D","started_at":100,"start_approx":true,
            "ended_at":200,"outcome":"finished","error_code":null,"hms":[],
            "verdict":"usable","tags":["stringing"],"notes":"ok","slice":null,
            "trays":[{"ams_id":0,"tray_id":1,"label":"A2","tray_type":"PLA",
            "color":"FFFFFFFF","brand":"PLA Basic"}],
            "photos":[{"id":7,"file_name":"a.jpg","added_at":150}],
            "analyses":[41],"updated_at":200}"#;
        let e: PrintEntry = serde_json::from_str(json).unwrap();
        assert_eq!(
            (e.outcome, e.verdict),
            (Outcome::Finished, Some(Verdict::Usable))
        );
        assert_eq!(e.trays[0].label, "A2");
        assert_eq!(e.photos[0].id, 7);
        let odd: Outcome = serde_json::from_str("\"cancelled\"").unwrap();
        assert_eq!(odd, Outcome::Unknown);
    }

    #[test]
    fn requests_serialize_as_the_backend_reads_them() {
        let filter = JournalFilter {
            kind: FilterKind::NeedsVerdict,
            query: "petg".into(),
        };
        assert_eq!(
            serde_json::to_string(&filter).unwrap(),
            r#"{"kind":"needs_verdict","query":"petg"}"#
        );
        let update = EntryUpdate {
            verdict: Some(VerdictChoice::Clear),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&update).unwrap(),
            r#"{"verdict":"clear"}"#
        );
    }
}
