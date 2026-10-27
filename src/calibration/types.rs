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

// Only the wire tests ask a reading for its test: the page always knows it.
#[cfg(test)]
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
    pub fine_ready: bool,
    pub opened_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResultView {
    pub id: i64,
    pub test: TestKind,
    pub value: String,
    pub preset_key: Option<String>,
    pub old_value: Option<String>,
    /// `old_value` as shown: `printer default` for a retraction left to the
    /// printer. Show this, never `old_value`.
    pub old_display: Option<String>,
    pub new_value: Option<String>,
    pub target_name: String,
    pub variant: Option<String>,
    pub saved: bool,
    pub undone: bool,
    pub created_at: i64,
    pub photo: Option<String>,
    pub journal_entry_id: Option<i64>,
    pub can_undo: bool,
    /// This save made the "(calibrated)" copy: undoing it moves the copy to
    /// the backups.
    pub created_copy: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RunView {
    pub id: i64,
    pub preset_name: String,
    pub preset_path: String,
    pub target_name: String,
    pub target_path: String,
    pub is_system: bool,
    pub will_save_as: Option<String>,
    pub filament_id: Option<String>,
    pub material: String,
    pub printer_model: String,
    pub nozzle: String,
    pub slot_label: Option<String>,
    pub variant: Option<String>,
    pub variants: Vec<String>,
    pub needs_variant: bool,
    pub preset_error: Option<String>,
    pub steps: Vec<StepView>,
    pub results: Vec<ResultView>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RunSummary {
    pub id: i64,
    pub preset_name: String,
    pub slot_label: Option<String>,
    pub updated_at: i64,
    pub result_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Defaults {
    pub setup: Setup,
    pub current: Option<String>,
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
    pub extra: Option<KeyDiff>,
    pub target_name: String,
    pub will_copy: bool,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ValueView {
    pub value: f64,
    pub text: String,
    pub display: String,
    pub change: Option<ChangeView>,
    pub change_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PreviewView {
    pub diagram: Diagram,
    pub labels: Vec<String>,
    pub value: Option<ValueView>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SaveAnswer {
    Saved {
        run: RunView,
        result_id: i64,
        studio_running: bool,
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
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuggestAnswer {
    pub photo: String,
    pub suggestion: Option<Suggestion>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LinkCandidates {
    pub suggested: Option<i64>,
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
            "created_at":5,"photo":null,"journal_entry_id":null,"can_undo":true,
            "created_copy":false
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

    /// What the backend's own types serialize (`wire.json`, written and
    /// checked by `commands::calibration`'s fixture test).
    fn wire() -> serde_json::Value {
        serde_json::from_str(include_str!("wire.json")).unwrap()
    }

    fn from_wire<T: serde::de::DeserializeOwned>(key: &str) -> T {
        serde_json::from_value(wire()[key].clone()).unwrap_or_else(|e| panic!("{key}: {e}"))
    }

    #[test]
    fn every_setup_and_reading_goes_out_as_the_backend_reads_it() {
        let wire = wire();
        let setups = [
            (
                "temperature",
                Setup::Temperature {
                    start: 230,
                    end: 190,
                },
            ),
            ("flow_coarse", Setup::FlowCoarse { base: 0.98 }),
            ("flow_fine", Setup::FlowFine { base: 1.029 }),
            (
                "retraction",
                Setup::Retraction {
                    start: 0.0,
                    end: 2.0,
                    step: 0.1,
                },
            ),
            (
                "max_volumetric_speed",
                Setup::MaxVolumetricSpeed {
                    start: 5.0,
                    end: 20.0,
                    step: 0.5,
                },
            ),
            ("pressure_advance", Setup::PressureAdvance {}),
        ];
        assert_eq!(setups.len(), TestKind::ALL.len());
        for (key, setup) in setups {
            assert_eq!(
                serde_json::to_value(setup).unwrap(),
                wire["setups"][key],
                "{key}"
            );
            let back: Setup = serde_json::from_value(wire["setups"][key].clone()).unwrap();
            assert_eq!(back, setup, "{key}");
            assert_eq!(setup.test().key(), key);
        }
        let readings = [
            ("temperature", Reading::Temperature { band: 1 }),
            ("flow_coarse", Reading::FlowCoarse { modifier: 5 }),
            ("flow_fine", Reading::FlowFine { modifier: -6 }),
            ("retraction", Reading::Retraction { height_mm: 5.0 }),
            (
                "max_volumetric_speed",
                Reading::MaxVolumetricSpeed { height_mm: 14.0 },
            ),
            ("pressure_advance", Reading::PressureAdvance { k: 0.025 }),
        ];
        assert_eq!(readings.len(), TestKind::ALL.len());
        for (key, reading) in readings {
            assert_eq!(
                serde_json::to_value(reading).unwrap(),
                wire["readings"][key],
                "{key}"
            );
            let back: Reading = serde_json::from_value(wire["readings"][key].clone()).unwrap();
            assert_eq!(back, reading, "{key}");
            assert_eq!(reading.test().key(), key);
        }
    }

    #[test]
    fn a_populated_run_reads_in_full() {
        let runs: Vec<RunSummary> = from_wire("runs");
        assert_eq!(runs[0].result_ids, [10, 11]);
        assert_eq!(runs[0].slot_label.as_deref(), Some("A2"));
        let run: RunView = from_wire("run");
        assert_eq!(
            (
                run.id,
                run.is_system,
                run.needs_variant,
                run.nozzle.as_str()
            ),
            (1, false, false, "0.4")
        );
        assert_eq!(run.variants.len(), 2);
        assert_eq!(run.variant.as_deref(), Some("Direct Drive Standard"));
        assert_eq!(run.steps.len(), 4);
        assert_eq!(
            run.steps[0].status,
            StepStatus::Saved {
                result_id: 10,
                value: "225 °C".into(),
                at: 200
            }
        );
        assert_eq!(run.steps[0].opened_at, Some(150));
        assert_eq!(run.steps[1].status, StepStatus::Skipped);
        assert!(run.steps[1].fine_ready);
        assert!(matches!(
            run.steps[2].status,
            StepStatus::Recorded { result_id: 11, .. }
        ));
        assert_eq!(run.steps[3].status, StepStatus::NotRun);
        let r = &run.results[0];
        assert_eq!(r.test, TestKind::Retraction);
        assert_eq!(r.old_value.as_deref(), Some("nil"));
        assert_eq!(r.old_display.as_deref(), Some("printer default"));
        assert_eq!(r.new_value.as_deref(), Some("0.6"));
        assert_eq!(
            (r.saved, r.undone, r.can_undo, r.created_copy),
            (true, false, true, true)
        );
        assert_eq!(
            (r.photo.as_deref(), r.journal_entry_id),
            (Some("3f2a.jpg"), Some(42))
        );
        let d: Defaults = from_wire("defaults");
        assert_eq!(
            d.setup,
            Setup::Temperature {
                start: 230,
                end: 190
            }
        );
        assert_eq!(
            (d.current.as_deref(), d.material_label.as_deref()),
            (Some("220"), Some("PLA"))
        );
    }

    #[test]
    fn previews_read_their_diagrams_values_and_changes() {
        let bands: PreviewView = from_wire("preview_bands");
        let Diagram::Bands { bands: list } = &bands.diagram else {
            panic!("bands")
        };
        assert_eq!((list[0].index, list[0].label.as_str()), (0, "230 °C"));
        assert_eq!(bands.labels.len(), list.len());
        let value = bands.value.unwrap();
        assert_eq!(
            (value.value, value.text.as_str(), value.display.as_str()),
            (225.0, "225", "225 °C")
        );
        assert_eq!(value.change_error, None);
        let change = value.change.unwrap();
        assert_eq!(
            (change.old.as_str(), change.old_display.as_str()),
            ("nil", "printer default")
        );
        assert_eq!(change.new, "0.6");
        assert!(change.will_copy);
        assert_eq!(change.variant.as_deref(), Some("Direct Drive Standard"));
        assert_eq!(
            change.extra.unwrap().key,
            "nozzle_temperature_initial_layer"
        );
        let patches: PreviewView = from_wire("preview_patches");
        let Diagram::Patches { patches: list } = &patches.diagram else {
            panic!("patches")
        };
        assert_eq!((list[0].modifier, list[0].label.as_str()), (-9, "-9"));
        assert_eq!(patches.value, None);
        let ruler: PreviewView = from_wire("preview_ruler");
        assert!(matches!(ruler.diagram, Diagram::Ruler { max_mm } if (max_mm - 3.4).abs() < 1e-9));
        let none: PreviewView = from_wire("preview_none");
        assert_eq!(
            (none.diagram, none.labels.is_empty()),
            (Diagram::None {}, true)
        );
    }

    #[test]
    fn answers_read_as_the_backend_sends_them() {
        let saved: SaveAnswer = from_wire("saved");
        let SaveAnswer::Saved {
            run,
            result_id,
            studio_running,
            assign_error,
        } = saved
        else {
            panic!("saved")
        };
        assert_eq!((run.id, result_id, studio_running), (1, 11, true));
        assert_eq!(assign_error.as_deref(), Some("No printer configured"));
        let changed: SaveAnswer = from_wire("changed");
        assert!(
            matches!(changed, SaveAnswer::Changed { preview } if preview.diagram == Diagram::None {})
        );
        let suggest: SuggestAnswer = from_wire("suggest");
        let s = suggest.suggestion.unwrap();
        assert_eq!((s.pick.as_str(), s.confidence), ("225 °C", 0.8));
        assert!(
            s.reason.contains('\n'),
            "the raw reason keeps its line break"
        );
        assert_eq!((suggest.photo.as_str(), suggest.note), ("3f2a.jpg", None));
        let links: LinkCandidates = from_wire("link_candidates");
        assert_eq!(links.suggested, Some(42));
        assert_eq!(links.entries[0].name, "Gear housing");
        let purpose = links.entries[0].purpose.as_ref().unwrap();
        assert_eq!(
            (purpose.calibration_result_id, purpose.test.as_str()),
            (11, "retraction")
        );
    }

    #[test]
    fn a_renamed_field_fails_loudly_instead_of_reading_as_a_default() {
        let wire = wire();
        let without = |mut v: serde_json::Value, path: &[&str]| {
            let (last, parents) = path.split_last().unwrap();
            let mut at = &mut v;
            for p in parents {
                at = if let Ok(i) = p.parse::<usize>() {
                    &mut at[i]
                } else {
                    &mut at[*p]
                };
            }
            at.as_object_mut().unwrap().remove(*last).unwrap();
            v
        };
        let run = wire["run"].clone();
        for path in [
            vec!["needs_variant"],
            vec!["results"],
            vec!["variants"],
            vec!["steps", "0", "fine_ready"],
            vec!["results", "0", "can_undo"],
            vec!["results", "0", "created_copy"],
            vec!["results", "0", "target_name"],
        ] {
            assert!(
                serde_json::from_value::<RunView>(without(run.clone(), &path)).is_err(),
                "{path:?}"
            );
        }
        let saved = wire["saved"].clone();
        assert!(serde_json::from_value::<SaveAnswer>(without(saved, &["studio_running"])).is_err());
        let suggest = wire["suggest"].clone();
        assert!(serde_json::from_value::<SuggestAnswer>(without(
            suggest,
            &["suggestion", "reason"]
        ))
        .is_err());
        let links = wire["link_candidates"].clone();
        assert!(serde_json::from_value::<LinkCandidates>(without(links, &["entries"])).is_err());
        let bands = wire["preview_bands"].clone();
        assert!(serde_json::from_value::<PreviewView>(without(bands, &["labels"])).is_err());
    }
}
