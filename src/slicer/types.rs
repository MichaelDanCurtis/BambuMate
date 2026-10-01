//! Mirrors of the backend's slicing types (`src-tauri/src/slicer`). Field
//! names are snake_case, as the backend serializes them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Progress {
    pub plate: u32,
    pub percent: u8,
    pub stage: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ErrorView {
    pub kind: String,
    pub message: String,
}

/// `ErrorView::kind` when a finished job's files were cleared
/// (`slicer_thumbnail`, `slicer_open_in_bambu_studio`).
pub const FILES_CLEARED_KIND: &str = "files_cleared";

impl ErrorView {
    /// A refusal that came as plain text.
    pub fn text(message: String) -> Self {
        Self {
            kind: String::new(),
            message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningLevel {
    Notice,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SliceWarning {
    pub level: WarningLevel,
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SliceObject {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FilamentUse {
    pub slot: u32,
    pub filament_type: String,
    pub color: String,
    pub used_g: f64,
    pub used_m: f64,
    #[serde(default)]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FilamentInfo {
    pub slot: u32,
    pub preset: String,
    pub filament_type: String,
    pub color: String,
    #[serde(default)]
    pub cost_per_kg: Option<f64>,
    #[serde(default)]
    pub density: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PlateResult {
    pub index: u32,
    pub time_seconds: u64,
    pub weight_g: f64,
    #[serde(default)]
    pub cost: Option<f64>,
    pub filaments: Vec<FilamentUse>,
    pub warnings: Vec<SliceWarning>,
    pub objects: Vec<SliceObject>,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SliceResult {
    pub plates: Vec<PlateResult>,
    pub printer: String,
    pub printer_model: String,
    pub process: String,
    pub filaments: Vec<FilamentInfo>,
    #[serde(default)]
    pub bed_type: Option<String>,
    pub bambu_studio_version: String,
    pub output_path: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Queued { position: usize },
    Running { progress: Option<Progress> },
    Done { result: SliceResult, cached: bool },
    Failed { error: ErrorView },
    Cancelled,
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Done { .. } | JobState::Failed { .. } | JobState::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOrigin {
    Manual,
    Auto,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct JobView {
    pub id: u64,
    pub origin: JobOrigin,
    pub source_path: String,
    pub model_name: String,
    pub printer: String,
    pub process: String,
    pub filament: String,
    pub bed_type: String,
    pub state: JobState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetSource {
    User,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PresetOption {
    pub name: String,
    pub source: PresetSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PresetLists {
    pub printers: Vec<PresetOption>,
    pub processes: Vec<PresetOption>,
    pub filaments: Vec<PresetOption>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SlicerStatus {
    pub installed: bool,
    #[serde(default)]
    pub version: Option<String>,
    pub supported: bool,
    pub min_version: String,
    pub tested_version: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SlicerSettings {
    pub printer: Option<String>,
    pub process: Option<String>,
    pub filament: Option<String>,
    pub bed_type: Option<String>,
    pub auto_slice: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SlicerSettingsView {
    pub saved: SlicerSettings,
    pub effective: SlicerSettings,
    pub bed_types: Vec<String>,
}
