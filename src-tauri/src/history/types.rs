use serde::{Deserialize, Serialize};

/// A recorded change to a profile parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedChange {
    pub parameter: String,
    pub old_value: f32,
    pub new_value: f32,
}

/// Summary of a refinement session for list views.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: i64,
    pub created_at: String,
    pub was_applied: bool,
}

/// Full details of a refinement session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub id: i64,
    pub profile_path: String,
    pub created_at: String,
    pub analysis_json: String,
    pub applied_changes: Option<Vec<AppliedChange>>,
    pub backup_path: Option<String>,
}

/// The filament preset the user says is loaded in one printer slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotAssignment {
    pub serial: String,
    /// 0-3 for AMS units, 128+ for AMS HT, 255 for external spools.
    pub ams_id: u32,
    /// 0-3 within an AMS; 254/255 for external spools.
    pub tray_id: u32,
    pub preset_name: String,
    /// The preset's `filament_id`, resolved through `inherits`.
    pub filament_id: Option<String>,
    /// The preset's JSON file, for checking its cloud-sync state later.
    pub preset_path: Option<String>,
    pub assigned_at: String,
}
