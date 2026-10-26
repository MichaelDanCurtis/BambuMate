//! The Calibrate tab's pure rules: links, step labels, Bambu Studio's
//! steps, setup fields, diffs, ticks and lines. Tested without a browser.
//! Every string from a preset or the printer is cut and cleaned by
//! `untrusted_text` and rendered as a plain text node.

use super::types::{
    ChangeView, Diagram, Reading, ResultView, RunSummary, RunView, Setup, StepKey, StepStatus,
    Suggestion, TestKind,
};
use crate::journal::format::local_time_text;
use crate::journal::types::PrintEntry;
use crate::pages::printer::short_preset;
use crate::printer::types::{untrusted_text, PrinterView, SlotStatus, SlotView};

/// The backend's `NO_PRESET`, which the page shows as its own state.
pub const NO_PRESET: &str = "This slot has no profile yet.";
pub const RESULT_GONE: &str = "That calibration result isn't in the history any more.";
pub const FINE_AFTER_COARSE: &str =
    "Save the coarse pass first: Bambu Studio slices the fine pass from it.";
pub const DEVELOPER_MODE: &str = "For a Bambu printer, Bambu Studio shows the Calibration menu only with Developer Mode on: Preferences → Developer Mode.";
pub const DISCARD_CHANGES: &str =
    "Bambu Studio leaves its presets with unsaved test changes. When it asks, discard them.";
pub const STUDIO_RUNNING: &str = "Bambu Studio is open: restart it so it reads the new value.";
pub const PA_NOTE: &str =
    "BambuMate records K only. Bambu Studio's Flow Dynamics wizard saves it on the printer.";
pub const NOT_A_BAND: &str = "That temperature isn't one of the bands on this tower.";
pub const ENTER_A_NUMBER: &str = "Enter a number.";
/// Shown on the fine pass after a coarse save made while Bambu Studio was
/// open: Studio slices the fine pass from the flow ratio it has loaded.
pub const RESTART_BEFORE_FINE: &str = "Bambu Studio was open when the coarse pass was saved: restart it before you slice the fine pass, so it reads the new flow ratio.";
/// Most characters shown of a preset or print name.
const NAME_MAX: usize = 96;
/// Most characters shown of an error the backend passed on.
const MESSAGE_MAX: usize = 200;
/// Most characters shown of the AI's reason.
const REASON_MAX: usize = 400;
/// Most characters shown of a printer model.
const MODEL_MAX: usize = 32;

/// A URL query component: unreserved characters as they are, every other
/// byte as `%XX`. Pure, so it works in native tests.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn slot_href(label: &str) -> String {
    format!("/filament/calibrate?slot={}", encode_component(label))
}

pub fn preset_href(path: &str) -> String {
    format!("/filament/calibrate?preset={}", encode_component(path))
}

pub fn result_href(id: i64) -> String {
    format!("/filament/calibrate?result={id}")
}

pub fn test_for(step: StepKey, fine: bool) -> TestKind {
    match step {
        StepKey::Temperature => TestKind::Temperature,
        StepKey::Flow if fine => TestKind::FlowFine,
        StepKey::Flow => TestKind::FlowCoarse,
        StepKey::Retraction => TestKind::Retraction,
        StepKey::MaxSpeed => TestKind::MaxVolumetricSpeed,
        StepKey::PressureAdvance => TestKind::PressureAdvance,
    }
}

pub fn step_of(test: TestKind) -> StepKey {
    match test {
        TestKind::Temperature => StepKey::Temperature,
        TestKind::FlowCoarse | TestKind::FlowFine => StepKey::Flow,
        TestKind::Retraction => StepKey::Retraction,
        TestKind::MaxVolumetricSpeed => StepKey::MaxSpeed,
        TestKind::PressureAdvance => StepKey::PressureAdvance,
    }
}

/// The strip's names: pressure advance says it is optional.
pub fn step_name(step: StepKey) -> &'static str {
    match step {
        StepKey::PressureAdvance => "Pressure advance (optional)",
        s => s.title(),
    }
}

/// `data-status` on a strip step.
pub fn status_key(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::NotRun => "not_run",
        StepStatus::Saved { .. } => "saved",
        StepStatus::Recorded { .. } => "recorded",
        StepStatus::Skipped => "skipped",
    }
}

/// `24 Oct 2026` at `offset_minutes` east of UTC.
pub fn date_text(unix_secs: i64, offset_minutes: i32) -> String {
    local_time_text(unix_secs, offset_minutes)
        .split(' ')
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn status_text(status: &StepStatus, offset_minutes: i32) -> String {
    match status {
        StepStatus::NotRun => "Not run".into(),
        StepStatus::Saved { value, at, .. } => {
            format!("Saved · {value} · {}", date_text(*at, offset_minutes))
        }
        StepStatus::Recorded { value, at, .. } => {
            format!("Recorded · {value} · {}", date_text(*at, offset_minutes))
        }
        StepStatus::Skipped => "Skipped".into(),
    }
}

/// The step a run opens on: the first not run, in the recommended order.
pub fn first_open(run: &RunView) -> StepKey {
    run.steps
        .iter()
        .find(|s| s.status == StepStatus::NotRun)
        .map_or(StepKey::Temperature, |s| s.step)
}

/// The result behind a step's Saved or Recorded status.
pub fn step_result(run: &RunView, step: StepKey) -> Option<i64> {
    run.steps
        .iter()
        .find(|s| s.step == step)
        .and_then(|s| match s.status {
            StepStatus::Saved { result_id, .. } | StepStatus::Recorded { result_id, .. } => {
                Some(result_id)
            }
            _ => None,
        })
}

pub fn step_of_result(run: &RunView, result_id: i64) -> Option<StepKey> {
    run.results
        .iter()
        .find(|r| r.id == result_id)
        .map(|r| step_of(r.test))
}

pub fn run_for_result(runs: &[RunSummary], result_id: i64) -> Option<i64> {
    runs.iter()
        .find(|r| r.result_ids.contains(&result_id))
        .map(|r| r.id)
}

/// `1.029`, `0.8`, `225`: no trailing zeros.
fn number(v: f64) -> String {
    let s = format!("{v:.5}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What to do in Bambu Studio, with the values the reading maths uses.
pub fn studio_steps(setup: &Setup, material_label: Option<&str>) -> Vec<String> {
    match *setup {
        Setup::Temperature { start, end } => vec![format!(
            "Calibration → Temperature · {} · start {start} · end {end}",
            material_label.unwrap_or("Custom")
        )],
        Setup::FlowCoarse { base } => vec![
            "Calibration → Flow rate → Coarse".into(),
            format!(
                "Bambu Studio uses the preset's flow ratio, {}.",
                number(base)
            ),
        ],
        Setup::FlowFine { base } => vec![
            "Calibration → Flow rate → Fine".into(),
            format!(
                "Bambu Studio uses the preset's flow ratio, {}.",
                number(base)
            ),
        ],
        Setup::Retraction { start, end, step } => vec![format!(
            "Calibration → Retraction test · start {} · end {} · step {}",
            number(start),
            number(end),
            number(step)
        )],
        Setup::MaxVolumetricSpeed { start, end, step } => vec![format!(
            "Calibration → More → Max flowrate · start {} · end {} · step {}",
            number(start),
            number(end),
            number(step)
        )],
        Setup::PressureAdvance {} => {
            vec!["The Calibration tab → Flow Dynamics: run it as Bambu Studio guides you.".into()]
        }
    }
}

/// One editable setup value.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
    pub unit: &'static str,
}

fn field(key: &'static str, label: &'static str, value: String, unit: &'static str) -> Field {
    Field {
        key,
        label,
        value,
        unit,
    }
}

pub fn setup_fields(setup: &Setup) -> Vec<Field> {
    match *setup {
        Setup::Temperature { start, end } => vec![
            field("start", "Start", start.to_string(), "°C"),
            field("end", "End", end.to_string(), "°C"),
        ],
        Setup::FlowCoarse { base } | Setup::FlowFine { base } => {
            vec![field("base", "Base flow ratio", number(base), "")]
        }
        Setup::Retraction { start, end, step } => vec![
            field("start", "Start", number(start), "mm"),
            field("end", "End", number(end), "mm"),
            field("step", "Step", number(step), "mm/mm"),
        ],
        Setup::MaxVolumetricSpeed { start, end, step } => vec![
            field("start", "Start", number(start), "mm³/s"),
            field("end", "End", number(end), "mm³/s"),
            field("step", "Step", number(step), "mm³/s per mm"),
        ],
        Setup::PressureAdvance {} => Vec::new(),
    }
}

/// `setup` with the field `key` set from `text`; `None` when the text
/// isn't a number. The backend checks the range.
pub fn with_field(setup: &Setup, key: &str, text: &str) -> Option<Setup> {
    let text = text.trim();
    let int = || text.parse::<i32>().ok();
    let float = || text.parse::<f64>().ok().filter(|v| v.is_finite());
    Some(match (*setup, key) {
        (Setup::Temperature { end, .. }, "start") => Setup::Temperature { start: int()?, end },
        (Setup::Temperature { start, .. }, "end") => Setup::Temperature { start, end: int()? },
        (Setup::FlowCoarse { .. }, "base") => Setup::FlowCoarse { base: float()? },
        (Setup::FlowFine { .. }, "base") => Setup::FlowFine { base: float()? },
        (Setup::Retraction { end, step, .. }, "start") => Setup::Retraction {
            start: float()?,
            end,
            step,
        },
        (Setup::Retraction { start, step, .. }, "end") => Setup::Retraction {
            start,
            end: float()?,
            step,
        },
        (Setup::Retraction { start, end, .. }, "step") => Setup::Retraction {
            start,
            end,
            step: float()?,
        },
        (Setup::MaxVolumetricSpeed { end, step, .. }, "start") => Setup::MaxVolumetricSpeed {
            start: float()?,
            end,
            step,
        },
        (Setup::MaxVolumetricSpeed { start, step, .. }, "end") => Setup::MaxVolumetricSpeed {
            start,
            end: float()?,
            step,
        },
        (Setup::MaxVolumetricSpeed { start, end, .. }, "step") => Setup::MaxVolumetricSpeed {
            start,
            end,
            step: float()?,
        },
        _ => return None,
    })
}

/// A value the backend sent for display, cut and cleaned. The old value is
/// shown from the backend's `old_display` (`printer default` only for a
/// retraction override left to the printer), never by labelling a raw
/// `"nil"` here: `"nil"` under another key is shown as it is.
fn shown(s: &str) -> String {
    untrusted_text(s, NAME_MAX)
}

/// `nozzle_temperature 220 → 225`.
pub fn diff_line(change: &ChangeView) -> String {
    format!(
        "{} {} → {}",
        shown(&change.preset_key),
        shown(&change.old_display),
        shown(&change.new)
    )
}

/// `First layer 225 → 230`, when the first layer moves too.
pub fn extra_line(change: &ChangeView) -> Option<String> {
    change
        .extra
        .as_ref()
        .map(|e| format!("First layer {} → {}", shown(&e.old), shown(&e.new)))
}

/// The ruler's tap targets: every mm, or every 5 or 10 on a tall print.
pub fn ruler_ticks(max_mm: f64) -> Vec<u32> {
    let top = (max_mm + 1e-6).floor().max(0.0) as u32;
    let step = if top <= 40 {
        1
    } else if top <= 200 {
        5
    } else {
        10
    };
    (0..=top).step_by(step).collect()
}

/// A typed temperature as its band; `None` when it isn't one.
pub fn band_for_celsius(setup: &Setup, celsius: i32) -> Option<u32> {
    match *setup {
        Setup::Temperature { start, end }
            if (end..=start).contains(&celsius) && (start - celsius) % 5 == 0 =>
        {
            Some(((start - celsius) / 5) as u32)
        }
        _ => None,
    }
}

pub fn height_reading(test: TestKind, height_mm: f64) -> Option<Reading> {
    match test {
        TestKind::Retraction => Some(Reading::Retraction { height_mm }),
        TestKind::MaxVolumetricSpeed => Some(Reading::MaxVolumetricSpeed { height_mm }),
        _ => None,
    }
}

pub fn patch_reading(test: TestKind, modifier: i32) -> Option<Reading> {
    match test {
        TestKind::FlowCoarse => Some(Reading::FlowCoarse { modifier }),
        TestKind::FlowFine => Some(Reading::FlowFine { modifier }),
        _ => None,
    }
}

/// The reading an accepted AI pick stands for: one of the drawn labels.
pub fn reading_for_pick(setup: &Setup, diagram: &Diagram, pick: &str) -> Option<Reading> {
    let pick = pick.trim();
    match diagram {
        Diagram::Bands { bands } => bands
            .iter()
            .find(|b| b.label == pick)
            .map(|b| Reading::Temperature { band: b.index }),
        Diagram::Patches { patches } => patches
            .iter()
            .find(|p| p.label == pick)
            .and_then(|p| patch_reading(setup.test(), p.modifier)),
        Diagram::Ruler { max_mm } => {
            let mm: u32 = pick.strip_suffix("mm")?.trim().parse().ok()?;
            (f64::from(mm) <= max_mm + 1e-6)
                .then(|| height_reading(setup.test(), f64::from(mm)))
                .flatten()
        }
        Diagram::None {} => None,
    }
}

/// `AI suggests the 225 °C band`.
pub fn suggestion_line(test: TestKind, pick: &str) -> String {
    let pick = untrusted_text(pick, NAME_MAX);
    match test {
        TestKind::Temperature => format!("AI suggests the {pick} band"),
        TestKind::FlowCoarse | TestKind::FlowFine => format!("AI suggests the {pick} patch"),
        _ => format!("AI suggests {pick}"),
    }
}

/// `Bambu PLA Basic · A2 · H2D · 0.4 mm nozzle`.
pub fn target_line(run: &RunView) -> String {
    let printer = match (run.printer_model.trim(), run.nozzle.trim()) {
        ("", _) => None,
        (model, "") => Some(untrusted_text(model, MODEL_MAX)),
        (model, nozzle) => Some(format!(
            "{} · {nozzle} mm nozzle",
            untrusted_text(model, MODEL_MAX)
        )),
    };
    [
        Some(untrusted_text(&short_preset(&run.target_name), NAME_MAX)),
        run.slot_label.as_deref().map(|l| untrusted_text(l, 16)),
        printer,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// `Will save as Bambu PLA Basic (calibrated)`, before a system preset's
/// first save.
pub fn will_save_line(run: &RunView) -> Option<String> {
    run.will_save_as.as_deref().map(|n| {
        format!(
            "Will save as {}",
            untrusted_text(&short_preset(n), NAME_MAX)
        )
    })
}

/// The picker's context: the configured printer and its active nozzle.
pub fn context_line(view: &PrinterView) -> String {
    let Some(p) = view.printer.as_ref().filter(|_| view.configured) else {
        return "No printer set up: you pick the nozzle variant.".into();
    };
    let model = untrusted_text(&p.model, MODEL_MAX);
    let nozzle = view.state.as_ref().and_then(|s| {
        s.nozzles
            .iter()
            .find(|n| s.active_nozzle == Some(n.id))
            .or_else(|| s.nozzles.first())
            .and_then(|n| n.diameter)
    });
    match nozzle {
        Some(d) => format!("{model} · {d} mm nozzle"),
        None => model,
    }
}

/// The slot names a preset to calibrate: an assignment (an RFID spool's
/// report beats it) or a detected preset.
pub fn slot_has_preset(slot: &SlotView) -> bool {
    !slot.tray.empty
        && (slot.detected.is_some()
            || (slot.assigned_preset.is_some() && slot.status != SlotStatus::Rfid))
}

/// The preset a slot's picker button names, as the backend picks it.
pub fn slot_preset_name(slot: &SlotView) -> String {
    let assigned = slot
        .assigned_preset
        .as_deref()
        .filter(|_| slot.status != SlotStatus::Rfid);
    match (assigned, slot.detected.as_ref()) {
        (Some(name), _) => untrusted_text(&short_preset(name), NAME_MAX),
        (None, Some(d)) => untrusted_text(&short_preset(&d.preset_name), NAME_MAX),
        (None, None) => format!(
            "{}, no profile",
            untrusted_text(slot.tray.tray_type.trim(), NAME_MAX)
        ),
    }
}

/// The journal's chip for a print linked to a calibration step. The
/// journal (Task 9) shows it; remove the `allow` when it does.
#[cfg_attr(not(test), allow(dead_code))]
pub fn purpose_chip(test_key: &str) -> String {
    match TestKind::from_key(test_key) {
        Some(t) => format!("Calibration: {}", t.title()),
        None => "Calibration".into(),
    }
}

/// `Gear housing · 24 Oct 2026 14:05`.
pub fn candidate_line(e: &PrintEntry, offset_minutes: i32) -> String {
    format!(
        "{} · {}",
        untrusted_text(&e.name, NAME_MAX),
        local_time_text(e.started_at, offset_minutes)
    )
}

/// The quiet lines after a save.
pub fn save_notes(studio_running: bool, assign_error: Option<&str>) -> Vec<String> {
    let mut notes = Vec::new();
    if studio_running {
        notes.push(STUDIO_RUNNING.to_string());
    }
    if let Some(e) = assign_error {
        notes.push(format!(
            "Saved, but the slot couldn't be assigned: {}",
            untrusted_text(e, MESSAGE_MAX)
        ));
    }
    notes
}

/// A history row's value, marked when it was undone.
pub fn history_value(r: &ResultView) -> String {
    if r.undone {
        format!("{} · undone", r.value)
    } else {
        r.value.clone()
    }
}

/// `printer default → 0.6`: a history row's change, from `old_display`.
/// `None` for a recorded K, which changed no preset.
pub fn history_change(r: &ResultView) -> Option<String> {
    match (r.old_display.as_deref(), r.new_value.as_deref()) {
        (Some(old), Some(new)) => Some(format!("{} → {}", shown(old), shown(new))),
        _ => None,
    }
}

/// The AI's reason on one line (runs of whitespace, newlines and tabs
/// become one space), cut and cleaned. It is shown as a plain text node,
/// never as HTML.
pub fn reason_line(reason: &str) -> String {
    let one_line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    untrusted_text(&one_line, REASON_MAX)
}

/// The typed field's label and unit, for each test.
pub fn typed_label(test: TestKind) -> (&'static str, &'static str) {
    match test {
        TestKind::Temperature => ("Or type the temperature", "°C"),
        TestKind::FlowCoarse | TestKind::FlowFine => ("Or type the modifier", "%"),
        TestKind::Retraction | TestKind::MaxVolumetricSpeed => ("Or type the height", "mm"),
        TestKind::PressureAdvance => ("K you used", ""),
    }
}

/// A typed temperature, modifier, height or K as a reading for `setup`'s
/// test. `None` for an empty field; otherwise the reading, or the line
/// that says why it isn't one. The backend checks the range.
pub fn typed_reading(setup: &Setup, text: &str) -> Option<Result<Reading, &'static str>> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let float = || t.parse::<f64>().ok().filter(|v| v.is_finite());
    let test = setup.test();
    Some(match test {
        TestKind::Temperature => t
            .parse::<i32>()
            .ok()
            .and_then(|c| band_for_celsius(setup, c))
            .map(|band| Reading::Temperature { band })
            .ok_or(NOT_A_BAND),
        TestKind::FlowCoarse | TestKind::FlowFine => t
            .parse::<i32>()
            .ok()
            .and_then(|m| patch_reading(test, m))
            .ok_or(ENTER_A_NUMBER),
        TestKind::Retraction | TestKind::MaxVolumetricSpeed => float()
            .and_then(|h| height_reading(test, h))
            .ok_or(ENTER_A_NUMBER),
        TestKind::PressureAdvance => float()
            .map(|k| Reading::PressureAdvance { k })
            .ok_or(ENTER_A_NUMBER),
    })
}

/// A link candidate that another result already holds: linking moves the
/// print, so the list says so first.
pub fn linked_elsewhere(e: &PrintEntry, result_id: i64) -> Option<String> {
    let p = e
        .purpose
        .as_ref()
        .filter(|p| p.calibration_result_id != result_id)?;
    let to = TestKind::from_key(&p.test).map_or("another calibration", TestKind::title);
    Some(format!("Linked to {to}: linking moves it here"))
}

/// Shown with Undo when the save made the "(calibrated)" copy: undoing it
/// moves the copy away.
pub fn undo_copy_note(change: &ChangeView) -> Option<String> {
    change.will_copy.then(|| {
        format!(
            "Undo moves {} to the backups, so Bambu Studio no longer lists it.",
            untrusted_text(&short_preset(&change.target_name), NAME_MAX)
        )
    })
}

/// `Confidence 80% · <reason>`: the reason on one line, as plain text.
pub fn suggestion_detail(s: &Suggestion) -> String {
    let confidence = format!("Confidence {}%", (s.confidence * 100.0).round() as i64);
    let reason = reason_line(&s.reason);
    if reason.is_empty() {
        confidence
    } else {
        format!("{confidence} · {reason}")
    }
}

/// `The preset holds 220 °C now.`, from the defaults' `current`. A `"nil"`
/// (a retraction left to the printer) isn't labelled here: the diff names
/// it from the backend's `old_display`.
pub fn current_line(test: TestKind, current: Option<&str>) -> Option<String> {
    let c = current
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "nil")?;
    let c = untrusted_text(c, NAME_MAX);
    Some(match test.unit() {
        "" => format!("The preset holds {c} now."),
        unit => format!("The preset holds {c} {unit} now."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::types::{Band, Patch, ResultView, StepView};
    use crate::printer::types::{Detected, Nozzle, PrinterState, PrinterSummary, Tray};

    fn step(step: StepKey, status: StepStatus) -> StepView {
        StepView {
            step,
            status,
            fine_ready: false,
            opened_at: None,
        }
    }

    fn result(id: i64, test: TestKind) -> ResultView {
        ResultView {
            id,
            test,
            value: "225 °C".into(),
            preset_key: Some("nozzle_temperature".into()),
            old_value: Some("220".into()),
            old_display: Some("220".into()),
            new_value: Some("225".into()),
            target_name: "Bambu PLA Basic (calibrated) @BBL H2D".into(),
            variant: None,
            saved: true,
            undone: false,
            created_at: 1_792_850_700,
            photo: None,
            journal_entry_id: None,
            can_undo: true,
        }
    }

    fn run(steps: Vec<StepView>) -> RunView {
        RunView {
            id: 7,
            preset_name: "Bambu PLA Basic @BBL H2D".into(),
            preset_path: "/sys/Bambu PLA Basic @BBL H2D.json".into(),
            target_name: "Bambu PLA Basic @BBL H2D".into(),
            target_path: "/sys/Bambu PLA Basic @BBL H2D.json".into(),
            is_system: true,
            will_save_as: Some("Bambu PLA Basic (calibrated) @BBL H2D".into()),
            filament_id: Some("GFA00".into()),
            material: "PLA".into(),
            printer_model: "H2D".into(),
            nozzle: "0.4".into(),
            slot_label: Some("A2".into()),
            variant: Some("Direct Drive Standard".into()),
            variants: vec![],
            needs_variant: false,
            preset_error: None,
            steps,
            results: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn links_carry_their_query_encoded() {
        assert_eq!(slot_href("A2"), "/filament/calibrate?slot=A2");
        assert_eq!(slot_href("Ext-L"), "/filament/calibrate?slot=Ext-L");
        assert_eq!(
            preset_href("/Users/me/My PLA @BBL H2D.json"),
            "/filament/calibrate?preset=%2FUsers%2Fme%2FMy%20PLA%20%40BBL%20H2D.json"
        );
        assert_eq!(encode_component("é&="), "%C3%A9%26%3D");
        assert_eq!(result_href(11), "/filament/calibrate?result=11");
    }

    #[test]
    fn steps_map_to_tests_and_back() {
        assert_eq!(test_for(StepKey::Flow, false), TestKind::FlowCoarse);
        assert_eq!(test_for(StepKey::Flow, true), TestKind::FlowFine);
        assert_eq!(
            test_for(StepKey::MaxSpeed, true),
            TestKind::MaxVolumetricSpeed
        );
        assert_eq!(step_of(TestKind::FlowFine), StepKey::Flow);
        assert_eq!(
            step_name(StepKey::PressureAdvance),
            "Pressure advance (optional)"
        );
        assert_eq!(step_name(StepKey::MaxSpeed), "Max speed");
    }

    #[test]
    fn a_step_reads_not_run_saved_recorded_or_skipped() {
        assert_eq!(status_text(&StepStatus::NotRun, 0), "Not run");
        assert_eq!(
            status_text(
                &StepStatus::Saved {
                    result_id: 1,
                    value: "225 °C".into(),
                    at: 1_792_850_700
                },
                0
            ),
            "Saved · 225 °C · 24 Oct 2026"
        );
        assert_eq!(
            status_text(
                &StepStatus::Recorded {
                    result_id: 2,
                    value: "K 0.025".into(),
                    at: 1_792_850_700
                },
                0
            ),
            "Recorded · K 0.025 · 24 Oct 2026"
        );
        assert_eq!(status_text(&StepStatus::Skipped, 0), "Skipped");
        assert_eq!(status_key(&StepStatus::Skipped), "skipped");
    }

    #[test]
    fn the_first_step_opened_is_the_first_not_run() {
        let saved = StepStatus::Saved {
            result_id: 3,
            value: "225 °C".into(),
            at: 1,
        };
        let r = run(vec![
            step(StepKey::Temperature, saved.clone()),
            step(StepKey::Flow, StepStatus::NotRun),
        ]);
        assert_eq!(first_open(&r), StepKey::Flow);
        assert_eq!(step_result(&r, StepKey::Temperature), Some(3));
        assert_eq!(step_result(&r, StepKey::Flow), None);
        let done = run(vec![step(StepKey::Temperature, saved)]);
        assert_eq!(first_open(&done), StepKey::Temperature);
        let mut with_results = run(vec![]);
        with_results.results = vec![result(11, TestKind::FlowFine)];
        assert_eq!(step_of_result(&with_results, 11), Some(StepKey::Flow));
        let runs = vec![RunSummary {
            id: 7,
            preset_name: "x".into(),
            slot_label: None,
            updated_at: 0,
            result_ids: vec![10, 11],
        }];
        assert_eq!(run_for_result(&runs, 11), Some(7));
        assert_eq!(run_for_result(&runs, 12), None);
    }

    #[test]
    fn studio_steps_name_the_menu_and_the_values() {
        let temp = Setup::Temperature {
            start: 230,
            end: 190,
        };
        assert_eq!(
            studio_steps(&temp, Some("PLA")),
            vec!["Calibration → Temperature · PLA · start 230 · end 190".to_string()]
        );
        assert_eq!(
            studio_steps(&Setup::FlowCoarse { base: 0.98 }, None),
            vec![
                "Calibration → Flow rate → Coarse".to_string(),
                "Bambu Studio uses the preset's flow ratio, 0.98.".to_string()
            ]
        );
        assert_eq!(
            studio_steps(
                &Setup::Retraction {
                    start: 0.0,
                    end: 2.0,
                    step: 0.1
                },
                None
            ),
            vec!["Calibration → Retraction test · start 0 · end 2 · step 0.1".to_string()]
        );
        assert_eq!(
            studio_steps(
                &Setup::MaxVolumetricSpeed {
                    start: 5.0,
                    end: 20.0,
                    step: 0.5
                },
                None
            )[0],
            "Calibration → More → Max flowrate · start 5 · end 20 · step 0.5"
        );
        assert!(studio_steps(&Setup::PressureAdvance {}, None)[0].contains("Flow Dynamics"));
    }

    #[test]
    fn setup_fields_edit_one_value_at_a_time() {
        let temp = Setup::Temperature {
            start: 230,
            end: 190,
        };
        let fields = setup_fields(&temp);
        assert_eq!(
            fields.iter().map(|f| f.key).collect::<Vec<_>>(),
            ["start", "end"]
        );
        assert_eq!(fields[0].value, "230");
        assert_eq!(
            with_field(&temp, "end", " 200 "),
            Some(Setup::Temperature {
                start: 230,
                end: 200
            })
        );
        assert_eq!(with_field(&temp, "end", "two hundred"), None);
        let speed = Setup::MaxVolumetricSpeed {
            start: 5.0,
            end: 20.0,
            step: 0.5,
        };
        assert_eq!(
            with_field(&speed, "step", "1"),
            Some(Setup::MaxVolumetricSpeed {
                start: 5.0,
                end: 20.0,
                step: 1.0
            })
        );
        assert!(setup_fields(&Setup::PressureAdvance {}).is_empty());
        assert_eq!(
            setup_fields(&Setup::FlowFine { base: 1.029 })[0].value,
            "1.029"
        );
    }

    #[test]
    fn diffs_name_the_key_and_show_nil_as_the_printer_default() {
        let change = ChangeView {
            preset_key: "nozzle_temperature".into(),
            old: "220".into(),
            old_display: "220".into(),
            new: "225".into(),
            extra: Some(crate::calibration::types::KeyDiff {
                key: "nozzle_temperature_initial_layer".into(),
                old: "225".into(),
                new: "230".into(),
            }),
            target_name: "x".into(),
            will_copy: false,
            variant: None,
        };
        assert_eq!(diff_line(&change), "nozzle_temperature 220 → 225");
        assert_eq!(
            extra_line(&change).as_deref(),
            Some("First layer 225 → 230")
        );
        let nil = ChangeView {
            preset_key: "filament_retraction_length".into(),
            old: "nil".into(),
            old_display: "printer default".into(),
            new: "0.6".into(),
            extra: None,
            ..change
        };
        assert_eq!(
            diff_line(&nil),
            "filament_retraction_length printer default → 0.6"
        );
        assert_eq!(extra_line(&nil), None);
        // Shown from `old_display`, never by labelling a raw "nil": a "nil"
        // under a key that isn't an override stays as the backend sent it.
        let plain_nil = ChangeView {
            preset_key: "nozzle_temperature".into(),
            old: "nil".into(),
            old_display: "nil".into(),
            ..nil.clone()
        };
        assert_eq!(diff_line(&plain_nil), "nozzle_temperature nil → 0.6");
        let hostile = ChangeView {
            old_display: "2\u{202E}0".into(),
            ..nil
        };
        assert_eq!(
            diff_line(&hostile),
            "filament_retraction_length 2\u{FFFD}0 → 0.6"
        );
    }

    #[test]
    fn readings_come_from_taps_typed_values_and_picks() {
        assert_eq!(ruler_ticks(21.4).len(), 22);
        assert_eq!(ruler_ticks(80.0)[1], 5, "a tall ruler ticks every 5 mm");
        let temp = Setup::Temperature {
            start: 230,
            end: 190,
        };
        assert_eq!(band_for_celsius(&temp, 225), Some(1));
        assert_eq!(band_for_celsius(&temp, 226), None);
        assert_eq!(band_for_celsius(&temp, 185), None);
        assert_eq!(
            height_reading(TestKind::Retraction, 5.0),
            Some(Reading::Retraction { height_mm: 5.0 })
        );
        assert_eq!(height_reading(TestKind::Temperature, 5.0), None);
        assert_eq!(
            patch_reading(TestKind::FlowFine, -6),
            Some(Reading::FlowFine { modifier: -6 })
        );
        let bands = Diagram::Bands {
            bands: vec![
                Band {
                    index: 0,
                    label: "230 °C".into(),
                },
                Band {
                    index: 1,
                    label: "225 °C".into(),
                },
            ],
        };
        assert_eq!(
            reading_for_pick(&temp, &bands, "225 °C"),
            Some(Reading::Temperature { band: 1 })
        );
        let patches = Diagram::Patches {
            patches: vec![Patch {
                modifier: 5,
                label: "+5".into(),
            }],
        };
        assert_eq!(
            reading_for_pick(&Setup::FlowCoarse { base: 1.0 }, &patches, "+5"),
            Some(Reading::FlowCoarse { modifier: 5 })
        );
        let speed = Setup::MaxVolumetricSpeed {
            start: 10.0,
            end: 30.0,
            step: 1.0,
        };
        assert_eq!(
            reading_for_pick(&speed, &Diagram::Ruler { max_mm: 21.0 }, "14 mm"),
            Some(Reading::MaxVolumetricSpeed { height_mm: 14.0 })
        );
        assert_eq!(
            reading_for_pick(&speed, &Diagram::Ruler { max_mm: 21.0 }, "30 mm"),
            None
        );
        assert_eq!(
            suggestion_line(TestKind::Temperature, "225 °C"),
            "AI suggests the 225 °C band"
        );
        assert_eq!(
            suggestion_line(TestKind::FlowCoarse, "+5"),
            "AI suggests the +5 patch"
        );
        assert_eq!(
            suggestion_line(TestKind::MaxVolumetricSpeed, "14 mm"),
            "AI suggests 14 mm"
        );
    }

    #[test]
    fn lines_about_the_run_and_the_printer() {
        let r = run(vec![]);
        assert_eq!(
            target_line(&r),
            "Bambu PLA Basic · A2 · H2D · 0.4 mm nozzle"
        );
        assert_eq!(
            will_save_line(&r).as_deref(),
            Some("Will save as Bambu PLA Basic (calibrated)")
        );
        let view = PrinterView {
            configured: true,
            printer: Some(PrinterSummary {
                model: "H2D".into(),
                ..Default::default()
            }),
            state: Some(PrinterState {
                nozzles: vec![Nozzle {
                    id: 0,
                    diameter: Some(0.4),
                    ..Default::default()
                }],
                active_nozzle: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(context_line(&view), "H2D · 0.4 mm nozzle");
        assert_eq!(
            context_line(&PrinterView::default()),
            "No printer set up: you pick the nozzle variant."
        );
        let detected = SlotView {
            label: "A2".into(),
            tray: Tray {
                tray_type: "PLA".into(),
                ..Default::default()
            },
            status: SlotStatus::Rfid,
            assigned_preset: Some("Stale".into()),
            detected: Some(Detected {
                preset_name: "Bambu PLA Basic @BBL H2D".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(slot_has_preset(&detected));
        assert_eq!(
            slot_preset_name(&detected),
            "Bambu PLA Basic",
            "a spool beats a stale assignment"
        );
        let bare = SlotView {
            tray: Tray {
                tray_type: "PETG".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!slot_has_preset(&bare));
        assert_eq!(slot_preset_name(&bare), "PETG, no profile");
        assert_eq!(purpose_chip("temperature"), "Calibration: Temperature");
        assert_eq!(purpose_chip("vfa"), "Calibration");
        assert_eq!(
            save_notes(true, Some("No printer configured")),
            vec![
                STUDIO_RUNNING.to_string(),
                "Saved, but the slot couldn't be assigned: No printer configured".to_string()
            ]
        );
        assert_eq!(
            save_notes(false, Some("odd\u{202E}\nerror"))[0],
            "Saved, but the slot couldn't be assigned: odd\u{FFFD}\u{FFFD}error"
        );
        let mut undone = result(11, TestKind::Temperature);
        undone.undone = true;
        assert_eq!(history_value(&undone), "225 °C · undone");
        let mut retraction = result(12, TestKind::Retraction);
        retraction.old_value = Some("nil".into());
        retraction.old_display = Some("printer default".into());
        retraction.new_value = Some("0.6".into());
        assert_eq!(
            history_change(&retraction).as_deref(),
            Some("printer default → 0.6")
        );
        retraction.old_display = None;
        assert_eq!(
            history_change(&retraction),
            None,
            "never falls back to the raw old value"
        );
        let mut recorded = result(13, TestKind::PressureAdvance);
        recorded.old_value = None;
        recorded.old_display = None;
        assert_eq!(history_change(&recorded), None);
        assert_eq!(
            reason_line("  Sharp edge\n\tat +5.\r\n\r\nSlight stringing  "),
            "Sharp edge at +5. Slight stringing"
        );
        assert_eq!(reason_line("a\u{7}b\u{202E}c"), "a\u{FFFD}b\u{FFFD}c");
        assert_eq!(reason_line(&"x".repeat(1000)).chars().count(), 400);
    }

    #[test]
    fn a_typed_value_becomes_a_reading_or_says_why_not() {
        let temp = Setup::Temperature {
            start: 230,
            end: 190,
        };
        assert_eq!(
            typed_reading(&temp, "  "),
            None,
            "an empty field asks nothing"
        );
        assert_eq!(
            typed_reading(&temp, " 225 "),
            Some(Ok(Reading::Temperature { band: 1 }))
        );
        assert_eq!(typed_reading(&temp, "226"), Some(Err(NOT_A_BAND)));
        assert_eq!(typed_reading(&temp, "hot"), Some(Err(NOT_A_BAND)));
        assert_eq!(
            typed_reading(&Setup::FlowFine { base: 1.0 }, "-6"),
            Some(Ok(Reading::FlowFine { modifier: -6 }))
        );
        assert_eq!(
            typed_reading(&Setup::FlowCoarse { base: 1.0 }, "five"),
            Some(Err(ENTER_A_NUMBER))
        );
        let retraction = Setup::Retraction {
            start: 0.0,
            end: 2.0,
            step: 0.1,
        };
        assert_eq!(
            typed_reading(&retraction, "5.5"),
            Some(Ok(Reading::Retraction { height_mm: 5.5 }))
        );
        assert_eq!(typed_reading(&retraction, "inf"), Some(Err(ENTER_A_NUMBER)));
        assert_eq!(
            typed_reading(&Setup::PressureAdvance {}, "0.025"),
            Some(Ok(Reading::PressureAdvance { k: 0.025 }))
        );
        assert_eq!(
            typed_reading(&Setup::PressureAdvance {}, "NaN"),
            Some(Err(ENTER_A_NUMBER))
        );
        assert_eq!(
            typed_label(TestKind::Temperature),
            ("Or type the temperature", "°C")
        );
        assert_eq!(
            typed_label(TestKind::FlowFine),
            ("Or type the modifier", "%")
        );
        assert_eq!(
            typed_label(TestKind::MaxVolumetricSpeed),
            ("Or type the height", "mm")
        );
        assert_eq!(typed_label(TestKind::PressureAdvance), ("K you used", ""));
    }

    #[test]
    fn a_print_linked_to_another_result_says_so() {
        let mut e = PrintEntry {
            id: 42,
            name: "Tower".into(),
            ..Default::default()
        };
        assert_eq!(linked_elsewhere(&e, 11), None, "a free print says nothing");
        e.purpose = Some(crate::journal::types::PrintPurpose {
            calibration_result_id: 11,
            test: "temperature".into(),
        });
        assert_eq!(linked_elsewhere(&e, 11), None, "already this result's");
        assert_eq!(
            linked_elsewhere(&e, 12).as_deref(),
            Some("Linked to Temperature: linking moves it here")
        );
        e.purpose = Some(crate::journal::types::PrintPurpose {
            calibration_result_id: 9,
            test: "flow_fine".into(),
        });
        assert_eq!(
            linked_elsewhere(&e, 12).as_deref(),
            Some("Linked to Flow (fine): linking moves it here")
        );
        e.purpose = Some(crate::journal::types::PrintPurpose {
            calibration_result_id: 9,
            test: "vfa".into(),
        });
        assert_eq!(
            linked_elsewhere(&e, 12).as_deref(),
            Some("Linked to another calibration: linking moves it here")
        );
    }

    #[test]
    fn undoing_a_save_that_made_a_copy_warns_first() {
        let change = ChangeView {
            preset_key: "nozzle_temperature".into(),
            old: "220".into(),
            old_display: "220".into(),
            new: "225".into(),
            extra: None,
            target_name: "Bambu PLA Basic (calibrated) @BBL H2D".into(),
            will_copy: true,
            variant: None,
        };
        assert_eq!(
            undo_copy_note(&change).as_deref(),
            Some("Undo moves Bambu PLA Basic (calibrated) to the backups, so Bambu Studio no longer lists it.")
        );
        let in_place = ChangeView {
            will_copy: false,
            ..change.clone()
        };
        assert_eq!(undo_copy_note(&in_place), None);
        let hostile = ChangeView {
            target_name: "Evil\u{202E} @BBL H2D".into(),
            ..change
        };
        assert_eq!(
            undo_copy_note(&hostile).as_deref(),
            Some("Undo moves Evil\u{FFFD} to the backups, so Bambu Studio no longer lists it.")
        );
    }

    #[test]
    fn the_suggestion_shows_its_confidence_and_its_reason_as_text() {
        let sg = crate::calibration::types::Suggestion {
            pick: "+5".into(),
            confidence: 0.804,
            reason: " Sharpest <b>edge</b>\n at +5 ".into(),
        };
        assert_eq!(
            suggestion_detail(&sg),
            "Confidence 80% · Sharpest <b>edge</b> at +5"
        );
        let bare = crate::calibration::types::Suggestion {
            reason: "  ".into(),
            confidence: 1.0,
            ..sg
        };
        assert_eq!(suggestion_detail(&bare), "Confidence 100%");
    }

    #[test]
    fn the_card_names_what_the_preset_holds_now() {
        assert_eq!(
            current_line(TestKind::Temperature, Some("220")).as_deref(),
            Some("The preset holds 220 °C now.")
        );
        assert_eq!(
            current_line(TestKind::FlowCoarse, Some("0.98")).as_deref(),
            Some("The preset holds 0.98 now.")
        );
        assert_eq!(
            current_line(TestKind::Retraction, Some("nil")),
            None,
            "a retraction left to the printer isn't labelled here: the diff shows it from old_display"
        );
        assert_eq!(current_line(TestKind::MaxVolumetricSpeed, None), None);
        assert_eq!(current_line(TestKind::Temperature, Some("  ")), None);
    }
}
