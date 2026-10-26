//! Mirrors of `commands::calibration`'s answers and of
//! `calibration::tests`' setups and readings. The field and tag names are
//! the backend's serde names.

use serde::{Deserialize, Serialize};

use crate::journal::types::PrintEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestKind {
    Temperature,
    FlowCoarse,
    FlowFine,
    Retraction,
    MaxVolumetricSpeed,
    PressureAdvance,
}

impl TestKind {
    pub const ALL: [TestKind; 6] = [
        TestKind::Temperature,
        TestKind::FlowCoarse,
        TestKind::FlowFine,
        TestKind::Retraction,
        TestKind::MaxVolumetricSpeed,
        TestKind::PressureAdvance,
    ];

    pub fn key(self) -> &'static str {
        match self {
            TestKind::Temperature => "temperature",
            TestKind::FlowCoarse => "flow_coarse",
            TestKind::FlowFine => "flow_fine",
            TestKind::Retraction => "retraction",
            TestKind::MaxVolumetricSpeed => "max_volumetric_speed",
            TestKind::PressureAdvance => "pressure_advance",
        }
    }

    pub fn from_key(key: &str) -> Option<TestKind> {
        TestKind::ALL.into_iter().find(|t| t.key() == key)
    }

    /// The same titles as the backend's: the journal chip uses them.
    pub fn title(self) -> &'static str {
        match self {
            TestKind::Temperature => "Temperature",
            TestKind::FlowCoarse => "Flow (coarse)",
            TestKind::FlowFine => "Flow (fine)",
            TestKind::Retraction => "Retraction",
            TestKind::MaxVolumetricSpeed => "Max speed",
            TestKind::PressureAdvance => "Pressure advance",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            TestKind::Temperature => "°C",
            TestKind::Retraction => "mm",
            TestKind::MaxVolumetricSpeed => "mm³/s",
            TestKind::FlowCoarse | TestKind::FlowFine | TestKind::PressureAdvance => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "test", rename_all = "snake_case")]
pub enum Setup {
    Temperature { start: i32, end: i32 },
    FlowCoarse { base: f64 },
    FlowFine { base: f64 },
    Retraction { start: f64, end: f64, step: f64 },
    MaxVolumetricSpeed { start: f64, end: f64, step: f64 },
    PressureAdvance {},
}

impl Setup {
    pub fn test(&self) -> TestKind {
        match self {
            Setup::Temperature { .. } => TestKind::Temperature,
            Setup::FlowCoarse { .. } => TestKind::FlowCoarse,
            Setup::FlowFine { .. } => TestKind::FlowFine,
            Setup::Retraction { .. } => TestKind::Retraction,
            Setup::MaxVolumetricSpeed { .. } => TestKind::MaxVolumetricSpeed,
            Setup::PressureAdvance {} => TestKind::PressureAdvance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "test", rename_all = "snake_case")]
pub enum Reading {
    Temperature { band: u32 },
    FlowCoarse { modifier: i32 },
    FlowFine { modifier: i32 },
    Retraction { height_mm: f64 },
    MaxVolumetricSpeed { height_mm: f64 },
    PressureAdvance { k: f64 },
}

impl Reading {
    pub fn test(&self) -> TestKind {
        match self {
            Reading::Temperature { .. } => TestKind::Temperature,
            Reading::FlowCoarse { .. } => TestKind::FlowCoarse,
            Reading::FlowFine { .. } => TestKind::FlowFine,
            Reading::Retraction { .. } => TestKind::Retraction,
            Reading::MaxVolumetricSpeed { .. } => TestKind::MaxVolumetricSpeed,
            Reading::PressureAdvance { .. } => TestKind::PressureAdvance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKey {
    Temperature,
    Flow,
    Retraction,
    MaxSpeed,
    PressureAdvance,
}

impl StepKey {
    pub const ALL: [StepKey; 5] = [
        StepKey::Temperature,
        StepKey::Flow,
        StepKey::Retraction,
        StepKey::MaxSpeed,
        StepKey::PressureAdvance,
    ];

    pub fn key(self) -> &'static str {
        match self {
            StepKey::Temperature => "temperature",
            StepKey::Flow => "flow",
            StepKey::Retraction => "retraction",
            StepKey::MaxSpeed => "max_speed",
            StepKey::PressureAdvance => "pressure_advance",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            StepKey::Temperature => "Temperature",
            StepKey::Flow => "Flow",
            StepKey::Retraction => "Retraction",
            StepKey::MaxSpeed => "Max speed",
            StepKey::PressureAdvance => "Pressure advance",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepStatus {
    NotRun,
    Saved {
        result_id: i64,
        value: String,
        at: i64,
    },
    Recorded {
        result_id: i64,
        value: String,
        at: i64,
    },
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StepView {
    pub step: StepKey,
    pub status: StepStatus,
    #[serde(default)]
    pub fine_ready: bool,
    #[serde(default)]
    pub opened_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResultView {
    pub id: i64,
    pub test: TestKind,
    pub value: String,
    #[serde(default)]
    pub preset_key: Option<String>,
    #[serde(default)]
    pub old_value: Option<String>,
    /// `old_value` as shown: `printer default` for a retraction left to the
    /// printer. Show this, never `old_value`.
    #[serde(default)]
    pub old_display: Option<String>,
    #[serde(default)]
    pub new_value: Option<String>,
    #[serde(default)]
    pub target_name: String,
    #[serde(default)]
    pub variant: Option<String>,
    pub saved: bool,
    pub undone: bool,
    pub created_at: i64,
    #[serde(default)]
    pub photo: Option<String>,
    #[serde(default)]
    pub journal_entry_id: Option<i64>,
    #[serde(default)]
    pub can_undo: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RunView {
    pub id: i64,
    pub preset_name: String,
    pub preset_path: String,
    pub target_name: String,
    pub target_path: String,
    pub is_system: bool,
    #[serde(default)]
    pub will_save_as: Option<String>,
    #[serde(default)]
    pub filament_id: Option<String>,
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub printer_model: String,
    #[serde(default)]
    pub nozzle: String,
    #[serde(default)]
    pub slot_label: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(default)]
    pub needs_variant: bool,
    #[serde(default)]
    pub preset_error: Option<String>,
    pub steps: Vec<StepView>,
    #[serde(default)]
    pub results: Vec<ResultView>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RunSummary {
    pub id: i64,
    pub preset_name: String,
    #[serde(default)]
    pub slot_label: Option<String>,
    pub updated_at: i64,
    #[serde(default)]
    pub result_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Defaults {
    pub setup: Setup,
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub material_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Band {
    pub index: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Patch {
    pub modifier: i32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Diagram {
    Bands { bands: Vec<Band> },
    Patches { patches: Vec<Patch> },
    Ruler { max_mm: f64 },
    None {},
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct KeyDiff {
    pub key: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChangeView {
    pub preset_key: String,
    /// The element exactly as the file holds it (`"nil"` included): what
    /// `save` sends back as `expected_old`. Never show it.
    pub old: String,
    /// `old` as shown (`printer default` for a retraction override left to
    /// the printer). Show this.
    pub old_display: String,
    pub new: String,
    #[serde(default)]
    pub extra: Option<KeyDiff>,
    pub target_name: String,
    pub will_copy: bool,
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ValueView {
    pub value: f64,
    pub text: String,
    pub display: String,
    #[serde(default)]
    pub change: Option<ChangeView>,
    #[serde(default)]
    pub change_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PreviewView {
    pub diagram: Diagram,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub value: Option<ValueView>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SaveAnswer {
    Saved {
        run: RunView,
        result_id: i64,
        #[serde(default)]
        studio_running: bool,
        #[serde(default)]
        assign_error: Option<String>,
    },
    Changed {
        preview: PreviewView,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Suggestion {
    pub pick: String,
    pub confidence: f64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuggestAnswer {
    pub photo: String,
    #[serde(default)]
    pub suggestion: Option<Suggestion>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LinkCandidates {
    #[serde(default)]
    pub suggested: Option<i64>,
    #[serde(default)]
    pub entries: Vec<PrintEntry>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn setups_and_readings_go_out_tagged_with_their_test() {
        assert_eq!(
            serde_json::to_value(Setup::Temperature {
                start: 230,
                end: 190
            })
            .unwrap(),
            json!({"test":"temperature","start":230,"end":190})
        );
        assert_eq!(
            serde_json::to_value(Reading::FlowCoarse { modifier: 5 }).unwrap(),
            json!({"test":"flow_coarse","modifier":5})
        );
        assert_eq!(
            serde_json::to_value(Setup::PressureAdvance {}).unwrap(),
            json!({"test":"pressure_advance"})
        );
    }

    #[test]
    fn answers_read_their_tags() {
        let status: StepStatus =
            serde_json::from_value(json!({"state":"saved","result_id":1,"value":"225 °C","at":5}))
                .unwrap();
        assert_eq!(
            status,
            StepStatus::Saved {
                result_id: 1,
                value: "225 °C".into(),
                at: 5
            }
        );
        let changed: SaveAnswer = serde_json::from_value(json!({
            "outcome":"changed",
            "preview":{"diagram":{"kind":"none"},"labels":[],"value":null}
        }))
        .unwrap();
        assert!(matches!(changed, SaveAnswer::Changed { .. }));
        let ruler: Diagram = serde_json::from_value(json!({"kind":"ruler","max_mm":21.4})).unwrap();
        assert_eq!(ruler, Diagram::Ruler { max_mm: 21.4 });
    }

    #[test]
    fn defaults_and_suggestions_read_the_backends_shapes() {
        let d: Defaults = serde_json::from_value(json!({
            "setup":{"test":"pressure_advance"},"current":null,"material_label":null
        }))
        .unwrap();
        assert_eq!(d.setup, Setup::PressureAdvance {});
        let t: Defaults = serde_json::from_value(json!({
            "setup":{"test":"temperature","start":230,"end":190},"current":"220","material_label":"PLA"
        }))
        .unwrap();
        assert_eq!(
            t.setup,
            Setup::Temperature {
                start: 230,
                end: 190
            }
        );
        let a: SuggestAnswer = serde_json::from_value(json!({
            "photo":"3f2a.jpg",
            "suggestion":{"pick":"225 °C","confidence":0.8,"reason":"<b>x</b>"},
            "note":null
        }))
        .unwrap();
        assert_eq!(a.suggestion.unwrap().reason, "<b>x</b>");
    }

    #[test]
    fn the_old_value_comes_raw_and_as_shown() {
        let change: ChangeView = serde_json::from_value(json!({
            "preset_key":"filament_retraction_length","old":"nil",
            "old_display":"printer default","new":"0.6","extra":null,
            "target_name":"x","will_copy":false,"variant":null
        }))
        .unwrap();
        assert_eq!(
            (change.old.as_str(), change.old_display.as_str()),
            ("nil", "printer default")
        );
        let result: ResultView = serde_json::from_value(json!({
            "id":1,"test":"retraction","value":"0.6","preset_key":"filament_retraction_length",
            "old_value":"nil","old_display":"printer default","new_value":"0.6",
            "target_name":"x","variant":null,"saved":true,"undone":false,
            "created_at":5,"photo":null,"journal_entry_id":null,"can_undo":true
        }))
        .unwrap();
        assert_eq!(result.old_display.as_deref(), Some("printer default"));
    }

    #[test]
    fn titles_match_the_backends() {
        assert_eq!(
            TestKind::from_key("max_volumetric_speed"),
            Some(TestKind::MaxVolumetricSpeed)
        );
        assert_eq!(TestKind::FlowFine.title(), "Flow (fine)");
        assert_eq!(StepKey::MaxSpeed.key(), "max_speed");
    }
}
