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
