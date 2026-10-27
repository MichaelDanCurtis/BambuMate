//! The calibration tests as Bambu Studio runs them (spec 2026-10-25, "Facts
//! from Bambu Studio"): each test's setup and its defaults, how a reading
//! off the print becomes the preset value, the labels the page draws, and
//! the range a value must fall in. Pure: no IO.
//!
//! The formulas were learned from Bambu Studio's source and Bambu Lab's
//! wiki and are reimplemented here. No code is copied (AGPL).

use serde::{Deserialize, Serialize};

/// Studio's temperature tower changes temperature every 5 °C.
pub const TEMP_STEP: i32 = 5;
/// The coarse pass's modifiers in percent: −20 to +20 in steps of 5.
pub const COARSE_MODIFIERS: [i32; 9] = [-20, -15, -10, -5, 0, 5, 10, 15, 20];
/// The fine pass's modifiers in percent: −9 to 0.
pub const FINE_MODIFIERS: [i32; 10] = [-9, -8, -7, -6, -5, -4, -3, -2, -1, 0];
/// Allowance for heights typed to 0.1 mm: `1.4 - 0.4` is `0.9999…`.
const HEIGHT_EPSILON: f64 = 1e-6;

pub const OTHER_TEST: &str = "That reading is for another test.";
pub const NO_FLOW_RATIO: &str =
    "This preset has no flow ratio for this nozzle variant to start from.";
/// The most labels a test may have. A ruler has one per whole mm, so this
/// is a tower of about 400 mm, taller than any Bambu printer's build
/// volume (the H2D's is 325 mm). Without it a tiny step would build a vast
/// label list.
pub const MAX_LABELS: usize = 400;
/// A tower with more labels than [`MAX_LABELS`]: taller than any printer.
pub const TOO_TALL: &str =
    "That tower would be taller than any printer can print. Use a bigger step or a shorter range.";
const NUMBERS_ONLY: &str = "Enter numbers only.";
const NOT_A_PATCH: &str = "That patch isn't on this print.";

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

    /// The `test` column and the wire name.
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

    /// The preset key a saved value goes to. `None` for pressure advance:
    /// on a Bambu printer K lives in the printer's own table, and the
    /// preset's `pressure_advance` is ignored.
    pub fn preset_key(self) -> Option<&'static str> {
        match self {
            TestKind::Temperature => Some("nozzle_temperature"),
            TestKind::FlowCoarse | TestKind::FlowFine => Some("filament_flow_ratio"),
            TestKind::Retraction => Some("filament_retraction_length"),
            TestKind::MaxVolumetricSpeed => Some("filament_max_volumetric_speed"),
            TestKind::PressureAdvance => None,
        }
    }

    /// How the history and the journal chip name it.
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

    /// Decimals the value is rounded to before it is written.
    fn places(self) -> usize {
        match self {
            TestKind::Temperature => 0,
            TestKind::FlowCoarse | TestKind::FlowFine => 5,
            TestKind::Retraction => 2,
            TestKind::MaxVolumetricSpeed => 1,
            TestKind::PressureAdvance => 3,
        }
    }
}

/// The values a test was run with in Bambu Studio, stored with the result
/// so the reading maths matches what was printed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "test", rename_all = "snake_case")]
pub enum Setup {
    /// °C. `start` is the bottom, hottest block. The step is fixed at 5.
    Temperature {
        start: i32,
        end: i32,
    },
    /// The preset's flow ratio the patches multiply.
    FlowCoarse {
        base: f64,
    },
    FlowFine {
        base: f64,
    },
    /// mm, and mm of retraction per mm of height.
    Retraction {
        start: f64,
        end: f64,
        step: f64,
    },
    /// mm³/s, and mm³/s per mm of height.
    MaxVolumetricSpeed {
        start: f64,
        end: f64,
        step: f64,
    },
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

/// What the user read off the print.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "test", rename_all = "snake_case")]
pub enum Reading {
    /// The block counted from the bed: 0 is the bottom, hottest one.
    Temperature {
        band: u32,
    },
    /// The patch's modifier, in percent.
    FlowCoarse {
        modifier: i32,
    },
    FlowFine {
        modifier: i32,
    },
    /// The height from the bed, in mm.
    Retraction {
        height_mm: f64,
    },
    MaxVolumetricSpeed {
        height_mm: f64,
    },
    /// The K the user used in Studio's Flow Dynamics wizard.
    PressureAdvance {
        k: f64,
    },
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

/// What the preset holds now, at the variant being calibrated.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Current {
    pub flow_ratio: Option<f64>,
    pub max_volumetric_speed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub index: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub modifier: i32,
    pub label: String,
}

/// What the page draws for the user to tap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Diagram {
    /// One band per 5 °C, the bottom (index 0, the hottest) first.
    Bands { bands: Vec<Band> },
    /// One patch per modifier, left to right.
    Patches { patches: Vec<Patch> },
    /// Heights from the bed, 0 to `max_mm`.
    Ruler { max_mm: f64 },
    /// Pressure advance: nothing is read off a print here.
    None {},
}

fn round_to(v: f64, places: usize) -> f64 {
    let f = 10f64.powi(places as i32);
    (v * f).round() / f
}

/// Studio's temperature dialog: the material radio for a filament type,
/// and that radio's start and end. Anything Studio doesn't list is Custom.
pub fn temperature_preset(filament_type: &str) -> (&'static str, i32, i32) {
    match filament_type.trim().to_ascii_uppercase().as_str() {
        "PLA" => ("PLA", 230, 190),
        "ABS" | "ASA" => ("ABS/ASA", 270, 230),
        "PETG" => ("PETG", 250, 230),
        "PCTG" => ("PCTG", 280, 240),
        "TPU" | "TPU-AMS" => ("TPU", 240, 210),
        "PA-CF" => ("PA-CF", 320, 280),
        "PET-CF" => ("PET-CF", 320, 280),
        _ => ("Custom", 230, 190),
    }
}

/// The setup a step opens with: Studio's defaults, and for flow and max
/// speed the preset's current values.
pub fn default_setup(
    test: TestKind,
    filament_type: &str,
    current: Current,
) -> Result<Setup, String> {
    Ok(match test {
        TestKind::Temperature => {
            let (_, start, end) = temperature_preset(filament_type);
            Setup::Temperature { start, end }
        }
        TestKind::FlowCoarse => Setup::FlowCoarse {
            base: current.flow_ratio.ok_or(NO_FLOW_RATIO)?,
        },
        TestKind::FlowFine => Setup::FlowFine {
            base: current.flow_ratio.ok_or(NO_FLOW_RATIO)?,
        },
        TestKind::Retraction => Setup::Retraction {
            start: 0.0,
            end: 2.0,
            step: 0.1,
        },
        TestKind::MaxVolumetricSpeed => {
            let end = current
                .max_volumetric_speed
                .map_or(20.0, |v| (v + 5.0).max(20.0));
            Setup::MaxVolumetricSpeed {
                start: 5.0,
                end: round_to(end, 1),
                step: 0.5,
            }
        }
        TestKind::PressureAdvance => Setup::PressureAdvance {},
    })
}

/// Refuses a setup Studio wouldn't run, with a sentence for the card.
pub fn validate_setup(setup: &Setup) -> Result<(), String> {
    let finite = |vals: &[f64]| vals.iter().all(|v| v.is_finite());
    match *setup {
        Setup::Temperature { start, end } => {
            if start % TEMP_STEP != 0 || end % TEMP_STEP != 0 {
                return Err(
                    "Use multiples of 5 °C: Bambu Studio rounds the start and end down to them."
                        .into(),
                );
            }
            if end < 180 || start > 350 {
                return Err("Bambu Studio's temperature tower runs from 180 to 350 °C.".into());
            }
            if start < end + TEMP_STEP {
                return Err(
                    "The start (the bottom, hottest block) must be at least 5 °C above the end."
                        .into(),
                );
            }
        }
        Setup::FlowCoarse { base } | Setup::FlowFine { base } => {
            if !finite(&[base]) || base <= 0.0 || base >= 2.0 {
                return Err("The base flow ratio must be between 0 and 2.".into());
            }
        }
        Setup::Retraction { start, end, step } => {
            if !finite(&[start, end, step]) {
                return Err(NUMBERS_ONLY.into());
            }
            if start < 0.0 {
                return Err("The start length can't be negative.".into());
            }
            if step <= 0.0 {
                return Err("The step must be more than 0.".into());
            }
            if end < start + step - HEIGHT_EPSILON {
                return Err("The end must be at least one step above the start.".into());
            }
            if end > 10.0 {
                return Err("Retraction can't go past 10 mm.".into());
            }
            fits_labels(retraction_tower_mm(start, end, step))?;
        }
        Setup::MaxVolumetricSpeed { start, end, step } => {
            if !finite(&[start, end, step]) {
                return Err(NUMBERS_ONLY.into());
            }
            if start <= 0.0 {
                return Err("The start must be more than 0 mm³/s.".into());
            }
            if step <= 0.0 {
                return Err("The step must be more than 0.".into());
            }
            if end < start + step - HEIGHT_EPSILON {
                return Err("The end must be at least one step above the start.".into());
            }
            if end > 200.0 {
                return Err("Max volumetric speed can't go past 200 mm³/s.".into());
            }
            fits_labels(speed_tower_mm(start, end, step))?;
        }
        Setup::PressureAdvance {} => {}
    }
    Ok(())
}

/// A ruler of `tower_mm` (as [`diagram`] rounds it) has one label per
/// whole mm from 0: at most [`MAX_LABELS`]. A temperature tower is bounded
/// by its 180–350 °C range (35 bands), and the patches are fixed.
fn fits_labels(tower_mm: f64) -> Result<(), String> {
    let max_mm = round_to(tower_mm, 1);
    if max_mm.is_finite() && (max_mm + HEIGHT_EPSILON).floor() + 1.0 <= MAX_LABELS as f64 {
        Ok(())
    } else {
        Err(TOO_TALL.into())
    }
}

/// The temperature of band `band`, counted from the bed.
pub fn temperature_at(start: i32, band: u32) -> i32 {
    start - TEMP_STEP * band as i32
}

/// How many 5 °C bands a tower from `start` down to `end` has.
pub fn band_count(start: i32, end: i32) -> u32 {
    ((start - end) / TEMP_STEP + 1).max(0) as u32
}

pub fn flow_ratio(base: f64, modifier: i32) -> f64 {
    base * f64::from(100 + modifier) / 100.0
}

/// The retraction tower's height: 1.4 mm of base, then 1 mm per step.
pub fn retraction_tower_mm(start: f64, end: f64, step: f64) -> f64 {
    1.4 + (end - start) / step
}

/// Retraction rises one step per mm of height after the first 1.4 mm.
/// The tower's single top layer prints `end + step`, so the value is held
/// at `end`.
pub fn retraction_at(start: f64, end: f64, step: f64, height_mm: f64) -> f64 {
    let steps = ((height_mm - 0.4).max(0.0) + HEIGHT_EPSILON).floor();
    (start + steps * step).min(end)
}

/// The max volumetric speed spiral's height.
pub fn speed_tower_mm(start: f64, end: f64, step: f64) -> f64 {
    (end - start + 1.0) / step
}

pub fn max_speed_at(start: f64, step: f64, height_mm: f64) -> f64 {
    start + height_mm * step
}

/// `+5`, `0`, `-20`: ASCII, as the AI must answer with it.
pub fn modifier_label(modifier: i32) -> String {
    if modifier > 0 {
        format!("+{modifier}")
    } else {
        modifier.to_string()
    }
}

fn patches(modifiers: &[i32]) -> Diagram {
    Diagram::Patches {
        patches: modifiers
            .iter()
            .map(|&m| Patch {
                modifier: m,
                label: modifier_label(m),
            })
            .collect(),
    }
}

/// What the page draws for `setup`.
pub fn diagram(setup: &Setup) -> Diagram {
    match *setup {
        Setup::Temperature { start, end } => Diagram::Bands {
            bands: (0..band_count(start, end))
                .map(|index| Band {
                    index,
                    label: format!("{} °C", temperature_at(start, index)),
                })
                .collect(),
        },
        Setup::FlowCoarse { .. } => patches(&COARSE_MODIFIERS),
        Setup::FlowFine { .. } => patches(&FINE_MODIFIERS),
        Setup::Retraction { start, end, step } => Diagram::Ruler {
            max_mm: round_to(retraction_tower_mm(start, end, step), 1),
        },
        Setup::MaxVolumetricSpeed { start, end, step } => Diagram::Ruler {
            max_mm: round_to(speed_tower_mm(start, end, step), 1),
        },
        Setup::PressureAdvance {} => Diagram::None {},
    }
}

/// The labels the AI picks from: the bands, the patches, or each whole mm
/// of a ruler.
pub fn labels(diagram: &Diagram) -> Vec<String> {
    match diagram {
        Diagram::Bands { bands } => bands.iter().map(|b| b.label.clone()).collect(),
        Diagram::Patches { patches } => patches.iter().map(|p| p.label.clone()).collect(),
        Diagram::Ruler { max_mm } => (0..=(max_mm + HEIGHT_EPSILON).floor() as u32)
            .map(|mm| format!("{mm} mm"))
            .collect(),
        Diagram::None {} => Vec::new(),
    }
}

/// The reading a label stands for; `None` for a label not on this print.
pub fn reading_for_label(setup: &Setup, label: &str) -> Option<Reading> {
    let label = label.trim();
    match (setup, diagram(setup)) {
        (Setup::Temperature { .. }, Diagram::Bands { bands }) => bands
            .iter()
            .find(|b| b.label == label)
            .map(|b| Reading::Temperature { band: b.index }),
        (Setup::FlowCoarse { .. }, Diagram::Patches { patches }) => patches
            .iter()
            .find(|p| p.label == label)
            .map(|p| Reading::FlowCoarse {
                modifier: p.modifier,
            }),
        (Setup::FlowFine { .. }, Diagram::Patches { patches }) => patches
            .iter()
            .find(|p| p.label == label)
            .map(|p| Reading::FlowFine {
                modifier: p.modifier,
            }),
        (
            Setup::Retraction { .. } | Setup::MaxVolumetricSpeed { .. },
            Diagram::Ruler { max_mm },
        ) => {
            let mm: u32 = label.strip_suffix("mm")?.trim().parse().ok()?;
            let height_mm = f64::from(mm);
            if height_mm > max_mm + HEIGHT_EPSILON {
                return None;
            }
            Some(match setup {
                Setup::Retraction { .. } => Reading::Retraction { height_mm },
                _ => Reading::MaxVolumetricSpeed { height_mm },
            })
        }
        _ => None,
    }
}

fn check_height(height_mm: f64, max_mm: f64) -> Result<(), String> {
    if !height_mm.is_finite() || height_mm < 0.0 || height_mm > max_mm + HEIGHT_EPSILON {
        return Err(format!(
            "Heights on this print run from 0 to {} mm.",
            format_number(round_to(max_mm, 1), 1)
        ));
    }
    Ok(())
}

/// The value to save for `reading` off a print made with `setup`, rounded
/// as it will be written, and refused outside the test's range.
pub fn value_of(setup: &Setup, reading: &Reading) -> Result<f64, String> {
    validate_setup(setup)?;
    let raw = match (*setup, *reading) {
        (Setup::Temperature { start, end }, Reading::Temperature { band }) => {
            let count = band_count(start, end);
            if band >= count {
                return Err(format!(
                    "That band isn't on this tower: it has {count} bands."
                ));
            }
            f64::from(temperature_at(start, band))
        }
        (Setup::FlowCoarse { base }, Reading::FlowCoarse { modifier }) => {
            if !COARSE_MODIFIERS.contains(&modifier) {
                return Err(NOT_A_PATCH.into());
            }
            flow_ratio(base, modifier)
        }
        (Setup::FlowFine { base }, Reading::FlowFine { modifier }) => {
            if !FINE_MODIFIERS.contains(&modifier) {
                return Err(NOT_A_PATCH.into());
            }
            flow_ratio(base, modifier)
        }
        (Setup::Retraction { start, end, step }, Reading::Retraction { height_mm }) => {
            check_height(height_mm, retraction_tower_mm(start, end, step))?;
            retraction_at(start, end, step, height_mm)
        }
        (
            Setup::MaxVolumetricSpeed { start, end, step },
            Reading::MaxVolumetricSpeed { height_mm },
        ) => {
            check_height(height_mm, speed_tower_mm(start, end, step))?;
            max_speed_at(start, step, height_mm)
        }
        (Setup::PressureAdvance {}, Reading::PressureAdvance { k }) => k,
        _ => return Err(OTHER_TEST.into()),
    };
    let test = setup.test();
    let value = round_to(raw, test.places());
    check_range(test, value)?;
    Ok(value)
}

/// `v` with at most `places` decimals and no trailing zeros: `1.029`,
/// `0.8`, `225`.
pub fn format_number(v: f64, places: usize) -> String {
    let s = format!("{:.*}", places, v);
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// The text written into the preset: `225`, `1.029`, `0.8`, `24.0`,
/// `0.025`. Max volumetric speed keeps its one decimal place.
pub fn format_value(test: TestKind, v: f64) -> String {
    match test {
        TestKind::MaxVolumetricSpeed => format!("{v:.1}"),
        t => format_number(v, t.places()),
    }
}

/// What the page shows: `225 °C`, `0.97`, `0.8 mm`, `24.0 mm³/s`, `K 0.025`.
pub fn display_value(test: TestKind, v: f64) -> String {
    let text = format_value(test, v);
    match test {
        TestKind::PressureAdvance => format!("K {text}"),
        t if t.unit().is_empty() => text,
        t => format!("{text} {}", t.unit()),
    }
}

pub fn range_text(test: TestKind) -> &'static str {
    match test {
        TestKind::Temperature => "180–350 °C",
        TestKind::FlowCoarse | TestKind::FlowFine | TestKind::PressureAdvance => {
            "more than 0 and less than 2"
        }
        TestKind::Retraction => "0–10 mm",
        TestKind::MaxVolumetricSpeed => "more than 0 and up to 200 mm³/s",
    }
}

fn in_range(test: TestKind, v: f64) -> bool {
    match test {
        TestKind::Temperature => (180.0..=350.0).contains(&v),
        TestKind::FlowCoarse | TestKind::FlowFine | TestKind::PressureAdvance => v > 0.0 && v < 2.0,
        TestKind::Retraction => (0.0..=10.0).contains(&v),
        TestKind::MaxVolumetricSpeed => v > 0.0 && v <= 200.0,
    }
}

/// Refuses a value outside what Studio accepts, naming the range.
pub fn check_range(test: TestKind, v: f64) -> Result<(), String> {
    if in_range(test, v) {
        return Ok(());
    }
    let shown = if v.is_finite() {
        display_value(test, v)
    } else {
        "That value".to_string()
    };
    Err(format!(
        "{shown} is outside the range Bambu Studio accepts ({}).",
        range_text(test)
    ))
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use serde_json::json;

    /// The text that would be written for `reading` off `setup`.
    fn value(setup: Setup, reading: Reading) -> String {
        let v = value_of(&setup, &reading).unwrap();
        format_value(setup.test(), v)
    }

    /// Bambu Studio's PA tower sets K = start + floor(height) × step. For
    /// reference only: BambuMate records the K the user used and never
    /// computes or writes one.
    fn pa_tower_k(start: f64, step: f64, height_mm: f64) -> f64 {
        start + (height_mm + 1e-6).floor() * step
    }

    #[test]
    fn flow_follows_the_wiki_examples() {
        assert_eq!(
            value(
                Setup::FlowCoarse { base: 0.98 },
                Reading::FlowCoarse { modifier: 5 }
            ),
            "1.029"
        );
        assert_eq!(
            value(
                Setup::FlowFine { base: 1.029 },
                Reading::FlowFine { modifier: -6 }
            ),
            "0.96726"
        );
    }

    #[test]
    fn max_speed_follows_the_wiki_example_to_one_decimal_place() {
        let wiki = Setup::MaxVolumetricSpeed {
            start: 10.0,
            end: 30.0,
            step: 1.0,
        };
        assert_eq!(
            value(wiki, Reading::MaxVolumetricSpeed { height_mm: 14.0 }),
            "24.0"
        );
        let studio = Setup::MaxVolumetricSpeed {
            start: 5.0,
            end: 20.0,
            step: 0.5,
        };
        assert_eq!(
            value(studio, Reading::MaxVolumetricSpeed { height_mm: 12.4 }),
            "11.2"
        );
        assert_eq!(
            diagram(&studio),
            Diagram::Ruler { max_mm: 32.0 },
            "(20 - 5 + 1) / 0.5"
        );
        assert!(
            value_of(&studio, &Reading::MaxVolumetricSpeed { height_mm: 33.0 })
                .unwrap_err()
                .contains("0 to 32 mm")
        );
    }

    #[test]
    fn retraction_rises_one_step_per_mm_after_the_base() {
        let setup = Setup::Retraction {
            start: 0.0,
            end: 2.0,
            step: 0.1,
        };
        let at = |h: f64| value(setup, Reading::Retraction { height_mm: h });
        assert_eq!(at(0.0), "0");
        assert_eq!(at(1.3), "0");
        assert_eq!(at(1.4), "0.1", "1.4 - 0.4 is 0.9999… in floating point");
        assert_eq!(at(5.0), "0.4");
        assert_eq!(at(21.4), "2", "the top layer is held at the end");
        assert!(value_of(&setup, &Reading::Retraction { height_mm: 21.5 })
            .unwrap_err()
            .contains("0 to 21.4 mm"));
    }

    #[test]
    fn temperature_bands_count_down_from_the_bottom() {
        let setup = Setup::Temperature {
            start: 230,
            end: 190,
        };
        let Diagram::Bands { bands } = diagram(&setup) else {
            panic!("a temperature tower draws bands");
        };
        assert_eq!(bands.len(), 9);
        assert_eq!(bands[0].label, "230 °C");
        assert_eq!(bands[8].label, "190 °C");
        assert_eq!(value(setup, Reading::Temperature { band: 1 }), "225");
        assert!(value_of(&setup, &Reading::Temperature { band: 9 })
            .unwrap_err()
            .contains("9 bands"));
    }

    #[test]
    fn flow_patches_are_labelled_by_their_modifier() {
        let coarse = labels(&diagram(&Setup::FlowCoarse { base: 1.0 }));
        assert_eq!(
            coarse,
            ["-20", "-15", "-10", "-5", "0", "+5", "+10", "+15", "+20"]
        );
        let fine = labels(&diagram(&Setup::FlowFine { base: 1.0 }));
        assert_eq!(fine.len(), 10);
        assert_eq!(fine.first().map(String::as_str), Some("-9"));
        assert_eq!(fine.last().map(String::as_str), Some("0"));
        assert!(
            value_of(
                &Setup::FlowFine { base: 1.0 },
                &Reading::FlowFine { modifier: 5 }
            )
            .is_err(),
            "+5 is a coarse patch"
        );
    }

    #[test]
    fn a_label_turns_back_into_its_reading() {
        let temp = Setup::Temperature {
            start: 230,
            end: 190,
        };
        assert_eq!(
            reading_for_label(&temp, "225 °C"),
            Some(Reading::Temperature { band: 1 })
        );
        assert_eq!(reading_for_label(&temp, "226 °C"), None);
        assert_eq!(
            reading_for_label(&Setup::FlowCoarse { base: 1.0 }, " +5 "),
            Some(Reading::FlowCoarse { modifier: 5 })
        );
        let speed = Setup::MaxVolumetricSpeed {
            start: 10.0,
            end: 30.0,
            step: 1.0,
        };
        assert_eq!(
            reading_for_label(&speed, "14 mm"),
            Some(Reading::MaxVolumetricSpeed { height_mm: 14.0 })
        );
        assert_eq!(reading_for_label(&speed, "99 mm"), None, "past the top");
        let ruler = labels(&diagram(&Setup::Retraction {
            start: 0.0,
            end: 2.0,
            step: 0.1,
        }));
        assert_eq!(ruler.first().map(String::as_str), Some("0 mm"));
        assert_eq!(ruler.last().map(String::as_str), Some("21 mm"));
        assert!(labels(&diagram(&Setup::PressureAdvance {})).is_empty());
    }

    #[test]
    fn temperature_defaults_follow_studios_material_radio() {
        assert_eq!(temperature_preset("PLA"), ("PLA", 230, 190));
        assert_eq!(temperature_preset(" abs "), ("ABS/ASA", 270, 230));
        assert_eq!(temperature_preset("ASA"), ("ABS/ASA", 270, 230));
        assert_eq!(temperature_preset("PETG"), ("PETG", 250, 230));
        assert_eq!(temperature_preset("PCTG"), ("PCTG", 280, 240));
        assert_eq!(temperature_preset("TPU-AMS"), ("TPU", 240, 210));
        assert_eq!(temperature_preset("PA-CF"), ("PA-CF", 320, 280));
        assert_eq!(temperature_preset("PET-CF"), ("PET-CF", 320, 280));
        assert_eq!(
            temperature_preset("PLA-CF"),
            ("Custom", 230, 190),
            "never a guess"
        );
        assert_eq!(
            default_setup(TestKind::Temperature, "PETG", Current::default()),
            Ok(Setup::Temperature {
                start: 250,
                end: 230
            })
        );
    }

    #[test]
    fn other_defaults_come_from_the_preset() {
        let current = Current {
            flow_ratio: Some(0.98),
            max_volumetric_speed: Some(25.0),
        };
        assert_eq!(
            default_setup(TestKind::FlowCoarse, "PLA", current),
            Ok(Setup::FlowCoarse { base: 0.98 })
        );
        assert_eq!(
            default_setup(TestKind::FlowFine, "PLA", Current::default()),
            Err(NO_FLOW_RATIO.to_string())
        );
        assert_eq!(
            default_setup(TestKind::Retraction, "PLA", current),
            Ok(Setup::Retraction {
                start: 0.0,
                end: 2.0,
                step: 0.1
            })
        );
        let end_of = |v: Option<f64>| match default_setup(
            TestKind::MaxVolumetricSpeed,
            "PLA",
            Current {
                flow_ratio: None,
                max_volumetric_speed: v,
            },
        ) {
            Ok(Setup::MaxVolumetricSpeed { start, end, step }) => {
                assert_eq!((start, step), (5.0, 0.5));
                end
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(end_of(Some(25.0)), 30.0);
        assert_eq!(end_of(Some(12.0)), 20.0);
        assert_eq!(end_of(None), 20.0);
        assert_eq!(
            default_setup(TestKind::PressureAdvance, "PLA", current),
            Ok(Setup::PressureAdvance {})
        );
    }

    #[test]
    fn setups_are_checked_against_what_studio_accepts() {
        let err = |s: Setup| validate_setup(&s).unwrap_err();
        assert!(err(Setup::Temperature {
            start: 232,
            end: 190
        })
        .contains("multiples of 5"));
        assert!(err(Setup::Temperature {
            start: 360,
            end: 300
        })
        .contains("180 to 350"));
        assert!(err(Setup::Temperature {
            start: 200,
            end: 200
        })
        .contains("at least 5 °C above"));
        assert!(err(Setup::FlowCoarse { base: 2.0 }).contains("between 0 and 2"));
        assert!(err(Setup::Retraction {
            start: 0.0,
            end: 0.05,
            step: 0.1
        })
        .contains("one step above"));
        assert!(err(Setup::Retraction {
            start: 0.0,
            end: 12.0,
            step: 0.1
        })
        .contains("10 mm"));
        assert!(err(Setup::MaxVolumetricSpeed {
            start: 0.0,
            end: 20.0,
            step: 0.5
        })
        .contains("more than 0"));
        assert!(err(Setup::MaxVolumetricSpeed {
            start: 5.0,
            end: f64::NAN,
            step: 0.5
        })
        .contains("numbers"));
        assert_eq!(validate_setup(&Setup::PressureAdvance {}), Ok(()));
    }

    #[test]
    fn a_tower_with_more_labels_than_any_printer_is_tall_is_refused_here_too() {
        let tall = [
            Setup::Retraction {
                start: 0.0,
                end: 10.0,
                step: 0.001,
            },
            Setup::MaxVolumetricSpeed {
                start: 1.0,
                end: 200.0,
                step: 0.0001,
            },
            // 400 mm: 401 labels, one over.
            Setup::MaxVolumetricSpeed {
                start: 1.0,
                end: 200.0,
                step: 0.5,
            },
        ];
        for s in tall {
            assert_eq!(validate_setup(&s), Err(TOO_TALL.to_string()), "{s:?}");
            assert!(
                value_of(&s, &Reading::Retraction { height_mm: 1.0 }).is_err(),
                "value_of validates first"
            );
        }
        // 399 mm: exactly 400 labels.
        let tallest = Setup::MaxVolumetricSpeed {
            start: 1.0,
            end: 199.5,
            step: 0.5,
        };
        assert_eq!(validate_setup(&tallest), Ok(()));
        assert_eq!(labels(&diagram(&tallest)).len(), MAX_LABELS);
        // Every setup that passes builds at most MAX_LABELS labels.
        for step in [0.01, 0.025, 0.05, 0.1, 0.5] {
            let s = Setup::Retraction {
                start: 0.0,
                end: 10.0,
                step,
            };
            if validate_setup(&s).is_ok() {
                assert!(labels(&diagram(&s)).len() <= MAX_LABELS, "{s:?}");
            }
        }
    }

    #[test]
    fn values_outside_studios_ranges_are_refused_with_the_range() {
        assert!(check_range(TestKind::Temperature, 355.0)
            .unwrap_err()
            .contains("180–350 °C"));
        assert!(value_of(
            &Setup::FlowCoarse { base: 1.9 },
            &Reading::FlowCoarse { modifier: 10 }
        )
        .unwrap_err()
        .contains("less than 2"));
        assert!(check_range(TestKind::MaxVolumetricSpeed, 0.0).is_err());
        assert!(check_range(TestKind::MaxVolumetricSpeed, 200.0).is_ok());
        assert!(check_range(TestKind::Retraction, 10.5).is_err());
        assert!(value_of(
            &Setup::PressureAdvance {},
            &Reading::PressureAdvance { k: 0.0 }
        )
        .is_err());
        assert_eq!(
            value_of(
                &Setup::Temperature {
                    start: 230,
                    end: 190
                },
                &Reading::FlowCoarse { modifier: 0 }
            ),
            Err(OTHER_TEST.to_string())
        );
    }

    #[test]
    fn the_pa_tower_reference_example_gives_0_03() {
        assert_eq!(format_number(pa_tower_k(0.0, 0.002, 15.0), 3), "0.03");
        assert_eq!(
            value(
                Setup::PressureAdvance {},
                Reading::PressureAdvance { k: 0.025 }
            ),
            "0.025"
        );
        assert_eq!(display_value(TestKind::PressureAdvance, 0.025), "K 0.025");
    }

    #[test]
    fn values_are_written_and_shown_per_test() {
        assert_eq!(format_value(TestKind::Temperature, 225.0), "225");
        assert_eq!(format_value(TestKind::FlowCoarse, 1.0), "1");
        assert_eq!(format_value(TestKind::Retraction, 0.8), "0.8");
        assert_eq!(format_value(TestKind::MaxVolumetricSpeed, 24.0), "24.0");
        assert_eq!(display_value(TestKind::Temperature, 225.0), "225 °C");
        assert_eq!(display_value(TestKind::FlowFine, 0.97), "0.97");
        assert_eq!(display_value(TestKind::Retraction, 0.8), "0.8 mm");
        assert_eq!(
            display_value(TestKind::MaxVolumetricSpeed, 24.0),
            "24.0 mm³/s"
        );
    }

    #[test]
    fn tests_have_their_keys_titles_and_preset_keys() {
        for t in TestKind::ALL {
            assert_eq!(TestKind::from_key(t.key()), Some(t));
        }
        assert_eq!(TestKind::from_key("vfa"), None);
        assert_eq!(TestKind::FlowFine.preset_key(), Some("filament_flow_ratio"));
        assert_eq!(TestKind::PressureAdvance.preset_key(), None);
        assert_eq!(TestKind::MaxVolumetricSpeed.title(), "Max speed");
    }

    #[test]
    fn setups_readings_and_diagrams_have_their_wire_shapes() {
        assert_eq!(
            serde_json::to_value(Setup::Temperature {
                start: 230,
                end: 190
            })
            .unwrap(),
            json!({"test":"temperature","start":230,"end":190})
        );
        assert_eq!(
            serde_json::to_value(Setup::PressureAdvance {}).unwrap(),
            json!({"test":"pressure_advance"})
        );
        let r: Reading = serde_json::from_value(json!({"test":"flow_fine","modifier":-6})).unwrap();
        assert_eq!(r, Reading::FlowFine { modifier: -6 });
        assert_eq!(
            serde_json::to_value(diagram(&Setup::Retraction {
                start: 0.0,
                end: 2.0,
                step: 0.1
            }))
            .unwrap(),
            json!({"kind":"ruler","max_mm":21.4})
        );
        assert_eq!(
            serde_json::to_value(diagram(&Setup::PressureAdvance {})).unwrap(),
            json!({"kind":"none"})
        );
    }
}
