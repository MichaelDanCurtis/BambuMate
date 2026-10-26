//! Commands for Filament → Calibrate (spec 2026-10-25 §3). Every database
//! and file call runs on a blocking thread. The logic lives in the helpers
//! below, which take a [`Ctx`] and a [`SlotAssigner`] so they are tested
//! against temp folders; the commands only gather the app's state.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::calibration::apply::{self, Change, LedgerNote, Planned, PresetFolders, SaveOutcome};
use crate::calibration::store::{
    latest_live, newest_saved, step_views, CalibrationPaths, CalibrationStore, KeyChange,
    NewResult, NewRun, Run, SlotSource, StepKey, StepView, PHOTO_FAILED, READ_FAILED, RESULT_GONE,
    RUN_GONE,
};
use crate::calibration::suggest::{self, Suggestion, VisionBackend};
use crate::calibration::tests::{self as cal, Current, Diagram, Reading, Setup, TestKind};
use crate::journal::photos;
use crate::journal::store::{JournalStore, Linked, GONE};
use crate::journal::types::{LinkCandidates, Outcome, PrintPurpose};
use crate::journal::{JournalPaths, CHANGED_EVENT};
use crate::printer::client::ConnectionState;
use crate::printer::service::{PrinterService, PrinterView};
use crate::printer::slots::{SlotStatus, SlotView};
use crate::printer::state::{PrinterState, TrayRef};
use crate::profile::BambuPaths;
use crate::slicer::jobs::SlicerService;
use crate::slicer::settings::PresetIndex;

pub const NO_PRESET: &str = "This slot has no profile yet.";
pub const NOT_CONNECTED: &str = "Can't use the AMS: printer not connected.";
pub const PICK_ONE: &str = "Pick a slot or a preset to calibrate.";
pub const COARSE_FIRST: &str =
    "Save the coarse pass first: Bambu Studio slices the fine pass from it.";
pub const NEWER_FIRST: &str = "Undo the newer save first.";
pub const NOTHING_TO_UNDO: &str = "A recorded K changed no preset, so there is nothing to undo.";
pub const ALREADY_UNDONE: &str = "That save is already undone.";
pub const NOT_A_VARIANT: &str = "That isn't one of this preset's nozzle variants.";
pub const NO_AI: &str = "No AI backend is set up, so the photo is kept without a suggestion.";
pub const PHOTO_UNAVAILABLE: &str = "Photo unavailable";
/// A tower with more labels than [`MAX_LABELS`]: taller than any printer.
pub const TOO_TALL: &str =
    "That tower would be taller than any printer can print. Use a bigger step or a shorter range.";
/// `"nil"` in a retraction override: the printer's own value.
pub const PRINTER_DEFAULT: &str = "printer default";
const PHOTO_GONE: &str = "That photo isn't in the calibration history any more.";
/// The save's history write failed, and the preset was put back.
pub const SAVE_NOT_RECORDED: &str =
    "The calibration history couldn't record the save, so the preset was put back as it was.";
/// The undo's file step happened but its history write failed. A second
/// Undo finishes it: it sees the file already put back.
pub const UNDO_NOT_RECORDED: &str =
    "The preset was put back, but the calibration history couldn't record the undo. Undo again to finish.";
pub const LINK_UNDONE: &str = "That save was undone, so there's no result to link a print to.";
pub const STILL_PRINTING: &str = "That print hasn't ended yet. Link it once it has.";
/// A bug stopped a preset change partway (a panic). Its ledger notes were
/// still recorded.
pub const INTERRUPTED: &str =
    "BambuMate hit a problem partway through. Check the preset in Bambu Studio before you try again.";
/// The most labels a test may have. A ruler has one per whole mm, so this
/// is a tower of about 400 mm, taller than any Bambu printer's build
/// volume (the H2D's is 325 mm). `validate_setup` has no upper bound on
/// the height, so a tiny step would otherwise build a vast label list.
pub const MAX_LABELS: usize = 400;

/// Serialises every command that reads a preset or the history and then
/// writes either: save, undo, record, skip, set variant, link and start.
/// Without it two overlapping saves could each read the file, and the
/// second write would drop the first's value; a double click on a system
/// preset would make two copies. Held only inside a blocking closure,
/// never across an `.await`.
static MUTATE: Mutex<()> = Mutex::new(());

/// The [`MUTATE`] guard. A panic while it was held poisons it; the lock
/// is still good (the files and the history are checked on every read),
/// so it is taken anyway.
fn mutating() -> MutexGuard<'static, ()> {
    MUTATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// What every helper reads and writes.
pub(crate) struct Ctx {
    pub store: CalibrationStore,
    pub presets: Arc<PresetIndex>,
    pub folders: PresetFolders,
}

/// Where a calibrated copy's slot assignment goes: the printer service in
/// the app, a recorder in tests.
pub(crate) trait SlotAssigner {
    fn assign(
        &self,
        ams_id: u32,
        tray_id: u32,
        name: &str,
        filament_id: Option<&str>,
        path: &str,
    ) -> Result<(), String>;
    fn clear(&self, ams_id: u32, tray_id: u32) -> Result<(), String>;
    /// The preset name assigned to the slot now.
    fn assigned_name(&self, ams_id: u32, tray_id: u32) -> Option<String>;
}

impl SlotAssigner for PrinterService {
    fn assign(
        &self,
        ams_id: u32,
        tray_id: u32,
        name: &str,
        filament_id: Option<&str>,
        path: &str,
    ) -> Result<(), String> {
        self.assign_slot(ams_id, tray_id, name, filament_id, Some(path))
            .map(|_| ())
    }

    fn clear(&self, ams_id: u32, tray_id: u32) -> Result<(), String> {
        self.clear_slot(ams_id, tray_id).map(|_| ())
    }

    fn assigned_name(&self, ams_id: u32, tray_id: u32) -> Option<String> {
        self.view()
            .slots
            .into_iter()
            .find(|s| s.ams_id == ams_id && s.tray_id == tray_id)
            .and_then(|s| s.assigned_preset)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResultView {
    pub id: i64,
    pub test: TestKind,
    /// `225 °C`, `0.97`, `K 0.025`.
    pub value: String,
    pub preset_key: Option<String>,
    pub old_value: Option<String>,
    /// `old_value` as shown: `printer default` for a retraction override
    /// left to the printer (`"nil"`), else `old_value` itself.
    pub old_display: Option<String>,
    pub new_value: Option<String>,
    /// The preset file's name, without `.json`.
    pub target_name: String,
    pub variant: Option<String>,
    pub saved: bool,
    pub undone: bool,
    pub created_at: i64,
    pub photo: Option<String>,
    pub journal_entry_id: Option<i64>,
    /// The newest saved result that isn't undone: the only one Undo takes back.
    pub can_undo: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunView {
    pub id: i64,
    pub preset_name: String,
    pub preset_path: String,
    /// The preset saves go to: the calibrated copy once there is one.
    pub target_name: String,
    pub target_path: String,
    /// The target is a Bambu system preset: a save makes a copy.
    pub is_system: bool,
    /// The copy's name a save would make.
    pub will_save_as: Option<String>,
    pub filament_id: Option<String>,
    pub material: String,
    pub printer_model: String,
    pub nozzle: String,
    pub slot_label: Option<String>,
    pub variant: Option<String>,
    pub variants: Vec<String>,
    /// Several variants and none chosen: the card asks.
    pub needs_variant: bool,
    pub preset_error: Option<String>,
    pub steps: Vec<StepView>,
    /// Newest first.
    pub results: Vec<ResultView>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunSummary {
    pub id: i64,
    pub preset_name: String,
    pub slot_label: Option<String>,
    pub updated_at: i64,
    pub result_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Defaults {
    pub setup: Setup,
    /// The preset's value now at the run's variant, as text.
    pub current: Option<String>,
    /// Temperature only: Studio's material radio (`PLA`, `ABS/ASA`, `Custom`).
    pub material_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyDiff {
    pub key: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeView {
    pub preset_key: String,
    /// The element now, exactly as the file holds it (`"nil"` included):
    /// what `calibration_save` takes back as `expected_old`.
    pub old: String,
    /// `old` as shown: `printer default` for a retraction override left
    /// to the printer, else `old` itself.
    pub old_display: String,
    pub new: String,
    /// The first-layer temperature, kept at its offset.
    pub extra: Option<KeyDiff>,
    pub target_name: String,
    pub will_copy: bool,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValueView {
    pub value: f64,
    /// As written: `225`, `1.029`.
    pub text: String,
    /// As shown: `225 °C`.
    pub display: String,
    pub change: Option<ChangeView>,
    /// Why there is no change to save yet, such as "Pick the nozzle variant".
    pub change_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewView {
    pub diagram: Diagram,
    pub labels: Vec<String>,
    pub value: Option<ValueView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SuggestAnswer {
    /// The kept photo's file name: pass it to save or record.
    pub photo: String,
    pub suggestion: Option<Suggestion>,
    pub note: Option<String>,
}

pub(crate) struct StartRequest {
    pub slot: Option<String>,
    pub preset: Option<String>,
    pub run: Option<i64>,
}

pub(crate) struct SaveRequest {
    pub run_id: i64,
    pub test: TestKind,
    pub setup: Setup,
    pub reading: Reading,
    pub expected_old: String,
    pub photo: Option<String>,
}

fn load_run(ctx: &Ctx, id: i64) -> Result<Run, String> {
    ctx.store.run(id)?.ok_or_else(|| RUN_GONE.to_string())
}

fn file_label(path: Option<&str>) -> String {
    path.and_then(|p| Path::new(p).file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// How an old element reads on the page. `"nil"` is the printer's own
/// value only in an override key (`filament_retraction_length`); in any
/// other key, Bambu Studio reads the parent's value, so the text stays.
pub(crate) fn old_display(key: &str, old: &str) -> String {
    let override_key = crate::profile::inheritance::FILAMENT_OVERRIDE_KEYS.contains(&key);
    if override_key && old.trim() == "nil" {
        PRINTER_DEFAULT.to_string()
    } else {
        old.to_string()
    }
}

/// The run as the page shows it, its preset read fresh.
pub(crate) fn run_view(ctx: &Ctx, run: Run) -> Result<RunView, String> {
    let results = ctx.store.results(run.id)?;
    let steps = ctx.store.steps(run.id)?;
    let (target_name, target_path) = run.target();
    let read = ctx
        .folders
        .checked(Path::new(target_path))
        .and_then(|(path, is_system)| {
            apply::flattened(&path, &ctx.presets).map(|(_, config)| (is_system, config))
        });
    let (is_system, variants, preset_error) = match read {
        Ok((is_system, config)) => (is_system, apply::variants_of(&config), None),
        Err(e) => (false, Vec::new(), Some(e)),
    };
    let will_save_as = if is_system {
        apply::copy_name(&ctx.folders, target_name).ok()
    } else {
        None
    };
    let newest = newest_saved(&results).map(|r| r.id);
    let result_views: Vec<ResultView> = results
        .iter()
        .rev()
        .map(|r| ResultView {
            id: r.id,
            test: r.test,
            value: cal::display_value(r.test, r.value),
            preset_key: r.preset_key.clone(),
            old_value: r.old_value.clone(),
            old_display: r
                .old_value
                .as_deref()
                .map(|old| old_display(r.preset_key.as_deref().unwrap_or_default(), old)),
            new_value: r.new_value.clone(),
            target_name: file_label(r.target_path.as_deref()),
            variant: r.variant.clone(),
            saved: r.saved,
            undone: r.undone,
            created_at: r.created_at,
            photo: r.photo_path.clone(),
            journal_entry_id: r.journal_entry_id,
            can_undo: Some(r.id) == newest,
        })
        .collect();
    Ok(RunView {
        id: run.id,
        preset_name: run.preset_name.clone(),
        preset_path: run.preset_path.clone(),
        target_name: target_name.to_string(),
        target_path: target_path.to_string(),
        is_system,
        will_save_as,
        filament_id: run.filament_id.clone(),
        material: run.material.clone(),
        printer_model: run.printer_model.clone(),
        nozzle: run.nozzle.clone(),
        slot_label: run.slot_label.clone(),
        needs_variant: run.variant.is_none() && variants.len() > 1,
        variant: run.variant.clone(),
        variants,
        preset_error,
        steps: step_views(&results, &steps),
        results: result_views,
        created_at: run.created_at,
        updated_at: run.updated_at,
    })
}

/// The printer's state, only while connected: stale slots never count.
fn trusted_state(view: &PrinterView) -> Option<&PrinterState> {
    if view.connection != ConnectionState::Connected {
        return None;
    }
    view.state.as_ref()
}

fn active_diameter(state: &PrinterState) -> Option<f64> {
    state
        .nozzles
        .iter()
        .find(|n| state.active_nozzle == Some(n.id))
        .or_else(|| state.nozzles.first())
        .and_then(|n| n.diameter)
}

/// The preset to calibrate a slot with: an RFID spool's detected preset,
/// because a stale assignment never beats the spool. Otherwise the
/// assignment (when still installed), else the detected preset.
fn slot_preset(slot: &SlotView, presets: &PresetIndex) -> Option<(PathBuf, SlotSource)> {
    if slot.tray.empty {
        return None;
    }
    if slot.status != SlotStatus::Rfid {
        if let Some(name) = &slot.assigned_preset {
            if let Some(entry) = presets
                .filament_entries()
                .into_iter()
                .find(|e| &e.name == name)
            {
                return Some((entry.path, SlotSource::Assigned));
            }
        }
    }
    slot.detected
        .as_ref()
        .map(|d| (PathBuf::from(&d.path), SlotSource::Detected))
}

fn first_string(config: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    match config.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.first()?.as_str().map(str::to_string),
        _ => None,
    }
}

/// Opens a run: by id, or for a slot or a preset file. The newest run for
/// the same file on the same printer and nozzle is reused.
pub(crate) fn start_run(
    ctx: &Ctx,
    req: StartRequest,
    view: &PrinterView,
    now: i64,
) -> Result<RunView, String> {
    let _held = mutating();
    if let Some(id) = req.run {
        return run_view(ctx, load_run(ctx, id)?);
    }
    let state = trusted_state(view);
    let (picked, slot) = match (req.slot.as_deref(), req.preset.as_deref()) {
        (Some(label), _) => {
            if state.is_none() {
                return Err(NOT_CONNECTED.into());
            }
            let slot = view
                .slots
                .iter()
                .find(|s| s.label == label)
                .ok_or_else(|| format!("There's no slot {label} on this printer."))?;
            let (path, source) = slot_preset(slot, &ctx.presets).ok_or(NO_PRESET)?;
            (path, Some((slot, source)))
        }
        (None, Some(p)) => (PathBuf::from(p), None),
        (None, None) => return Err(PICK_ONE.into()),
    };
    let (path, _) = ctx.folders.checked(&picked)?;
    let (profile, config) = apply::flattened(&path, &ctx.presets)?;
    let printer_model = view
        .printer
        .as_ref()
        .map(|p| p.model.trim().to_string())
        .unwrap_or_default();
    let nozzle = state
        .and_then(active_diameter)
        .map(|d| format!("{d}"))
        .unwrap_or_default();
    let path_text = path.to_string_lossy().into_owned();
    if let Some(run) = ctx.store.find_run(&path_text, &printer_model, &nozzle)? {
        return run_view(ctx, run);
    }
    let variants = apply::variants_of(&config);
    let tray = slot.map(|(s, _)| TrayRef {
        ams_id: s.ams_id,
        tray_id: s.tray_id,
    });
    let variant = state
        .and_then(|s| apply::printer_variant(s, tray))
        .and_then(|v| {
            variants
                .iter()
                .find(|n| n.eq_ignore_ascii_case(&v))
                .cloned()
        });
    let run = ctx.store.create_run(
        &NewRun {
            preset_name: profile.name().unwrap_or_default().to_string(),
            preset_path: path_text,
            filament_id: first_string(&config, "filament_id"),
            material: first_string(&config, "filament_type").unwrap_or_default(),
            printer_model,
            nozzle,
            slot_label: slot.map(|(s, _)| s.label.clone()),
            ams_id: slot.map(|(s, _)| s.ams_id),
            tray_id: slot.map(|(s, _)| s.tray_id),
            slot_source: slot.map(|(_, source)| source),
            variant,
        },
        now,
    )?;
    run_view(ctx, run)
}

/// A step's opening values, and the first time it was opened is stamped.
pub(crate) fn defaults_for(
    ctx: &Ctx,
    run_id: i64,
    test: TestKind,
    now: i64,
) -> Result<Defaults, String> {
    let run = load_run(ctx, run_id)?;
    ctx.store.note_opened(run.id, StepKey::of(test), now)?;
    let (path, _) = ctx.folders.checked(Path::new(run.target().1))?;
    let (_, config) = apply::flattened(&path, &ctx.presets)?;
    let variant = run.variant.as_deref();
    let at = |key: &str| {
        apply::variant_index(&config, key, variant).map(|i| apply::element(&config, key, i))
    };
    let number = |key: &str| {
        at(key)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<f64>().ok())
    };
    if matches!(test, TestKind::FlowCoarse | TestKind::FlowFine) {
        // The base is the variant's own ratio: without a variant, ask.
        at("filament_flow_ratio")?;
    }
    let current = Current {
        flow_ratio: number("filament_flow_ratio"),
        max_volumetric_speed: number("filament_max_volumetric_speed"),
    };
    Ok(Defaults {
        setup: cal::default_setup(test, &run.material, current)?,
        current: test.preset_key().and_then(|k| at(k).ok().flatten()),
        material_label: (test == TestKind::Temperature)
            .then(|| cal::temperature_preset(&run.material).0.to_string()),
    })
}

fn check_test(test: TestKind, setup: &Setup, reading: Option<&Reading>) -> Result<(), String> {
    if setup.test() != test || reading.is_some_and(|r| r.test() != test) {
        return Err(cal::OTHER_TEST.into());
    }
    Ok(())
}

/// A setup Studio would run, its diagram and its labels. A ruler taller
/// than [`MAX_LABELS`] mm is refused before its labels are built.
fn checked_diagram(setup: &Setup) -> Result<(Diagram, Vec<String>), String> {
    cal::validate_setup(setup)?;
    let diagram = cal::diagram(setup);
    if let Diagram::Ruler { max_mm } = diagram {
        // One label per whole mm, from 0.
        if !(max_mm.is_finite() && max_mm + 1.0 <= MAX_LABELS as f64) {
            return Err(TOO_TALL.into());
        }
    }
    let labels = cal::labels(&diagram);
    if labels.len() > MAX_LABELS {
        return Err(TOO_TALL.into());
    }
    Ok((diagram, labels))
}

fn coarse_saved(store: &CalibrationStore, run_id: i64) -> Result<bool, String> {
    Ok(latest_live(&store.results(run_id)?, TestKind::FlowCoarse).is_some_and(|r| r.saved))
}

/// The run, with its calibrated copy taken back from the history when the
/// run lost track of it: the live save that made the copy recorded its
/// path. A later save then edits that copy rather than making another.
fn reconciled_run(ctx: &Ctx, id: i64, now: i64) -> Result<Run, String> {
    let run = load_run(ctx, id)?;
    if run.copy_path.is_some() {
        return Ok(run);
    }
    let results = ctx.store.results(run.id)?;
    let made = results
        .iter()
        .filter(|r| r.saved && !r.undone && r.created_copy)
        .max_by_key(|r| r.id);
    let Some(path) = made.and_then(|r| r.target_path.as_deref()) else {
        return Ok(run);
    };
    let name = crate::profile::reader::read_profile(Path::new(path))
        .ok()
        .and_then(|p| p.name().map(str::to_string))
        .unwrap_or_else(|| file_label(Some(path)));
    tracing::warn!(
        "calibration: run {} had lost its copy {path}; taken back",
        run.id
    );
    ctx.store.set_copy(run.id, Some((&name, path)), now)?;
    load_run(ctx, id)
}

/// The file already holds every element a save changed as it was before:
/// an undo whose history write failed after it put them back.
fn already_put_back(ctx: &Ctx, target: &Path, changes: &[KeyChange]) -> bool {
    let Ok((path, _)) = ctx.folders.checked(target) else {
        return false;
    };
    let Ok((_, config)) = apply::flattened(&path, &ctx.presets) else {
        return false;
    };
    !changes.is_empty()
        && changes
            .iter()
            .all(|c| apply::element(&config, &c.key, c.index).as_deref() == Some(c.old.as_str()))
}

fn change_view(p: &Planned) -> ChangeView {
    let main = &p.changes[0];
    ChangeView {
        preset_key: main.key.clone(),
        old: main.old.clone(),
        old_display: old_display(&main.key, &main.old),
        new: main.new.clone(),
        extra: p.changes.get(1).map(|c| KeyDiff {
            key: c.key.clone(),
            old: c.old.clone(),
            new: c.new.clone(),
        }),
        target_name: p.target_name.clone(),
        will_copy: p.will_copy,
        variant: p.variant.clone(),
    }
}

fn value_view(ctx: &Ctx, run: &Run, setup: &Setup, reading: &Reading) -> Result<ValueView, String> {
    let test = setup.test();
    let value = cal::value_of(setup, reading)?;
    let (change, change_error) = if test.preset_key().is_none() {
        (None, None)
    } else {
        let change = Change {
            test,
            value,
            variant: run.variant.as_deref(),
        };
        match apply::plan(
            Path::new(run.target().1),
            &ctx.folders,
            &ctx.presets,
            change,
        ) {
            Ok(p) => (Some(change_view(&p)), None),
            Err(e) => (None, Some(e)),
        }
    };
    Ok(ValueView {
        value,
        text: cal::format_value(test, value),
        display: cal::display_value(test, value),
        change,
        change_error,
    })
}

/// The diagram for a setup, and the value and diff once there is a reading.
pub(crate) fn preview_for(
    ctx: &Ctx,
    run_id: i64,
    test: TestKind,
    setup: Setup,
    reading: Option<Reading>,
) -> Result<PreviewView, String> {
    check_test(test, &setup, reading.as_ref())?;
    let (diagram, labels) = checked_diagram(&setup)?;
    let run = load_run(ctx, run_id)?;
    if test == TestKind::FlowFine && !coarse_saved(&ctx.store, run.id)? {
        return Err(COARSE_FIRST.into());
    }
    Ok(PreviewView {
        labels,
        diagram,
        value: reading
            .map(|r| value_view(ctx, &run, &setup, &r))
            .transpose()?,
    })
}

fn checked_photo(ctx: &Ctx, photo: Option<String>) -> Result<Option<String>, String> {
    match photo {
        None => Ok(None),
        Some(name) => match ctx.store.photo_path(&name) {
            Some(_) => Ok(Some(name)),
            None => Err(PHOTO_GONE.to_string()),
        },
    }
}

/// Saves a reading into the run's preset, or answers the new diff when the
/// preset changed since the preview. Ledger notes go to `ledger` even when
/// the save fails: the caller replays them all.
pub(crate) fn save_for(
    ctx: &Ctx,
    req: SaveRequest,
    assigner: &dyn SlotAssigner,
    studio_running: bool,
    now: i64,
    ledger: &mut Vec<LedgerNote>,
) -> Result<SaveAnswer, String> {
    let SaveRequest {
        run_id,
        test,
        setup,
        reading,
        expected_old,
        photo,
    } = req;
    if test == TestKind::PressureAdvance {
        return Err(apply::RECORD_ONLY.into());
    }
    check_test(test, &setup, Some(&reading))?;
    let (diagram, labels) = checked_diagram(&setup)?;
    // Held from reading the run's target to recording the result: a second
    // save waits, then reads the file (and the copy) this one left.
    let _held = mutating();
    let run = reconciled_run(ctx, run_id, now)?;
    if test == TestKind::FlowFine && !coarse_saved(&ctx.store, run.id)? {
        return Err(COARSE_FIRST.into());
    }
    let value = cal::value_of(&setup, &reading)?;
    let photo = checked_photo(ctx, photo)?;
    let change = Change {
        test,
        value,
        variant: run.variant.as_deref(),
    };
    let target = PathBuf::from(run.target().1);
    let outcome = apply::save(
        &target,
        &ctx.folders,
        &ctx.presets,
        change,
        &expected_old,
        ledger,
    )?;
    let saved = match outcome {
        SaveOutcome::Changed(p) => {
            return Ok(SaveAnswer::Changed {
                preview: PreviewView {
                    labels,
                    diagram,
                    value: Some(ValueView {
                        value,
                        text: cal::format_value(test, value),
                        display: cal::display_value(test, value),
                        change: Some(change_view(&p)),
                        change_error: None,
                    }),
                },
            });
        }
        SaveOutcome::Saved(s) => s,
    };
    let main = &saved.changes[0];
    let target_text = saved.target_path.to_string_lossy().into_owned();
    let new = NewResult {
        run_id: run.id,
        setup,
        reading,
        value,
        preset_key: Some(main.key.clone()),
        old_value: Some(main.old.clone()),
        new_value: Some(main.new.clone()),
        target_path: Some(target_text.clone()),
        variant: saved.variant.clone(),
        backup_path: saved
            .backup_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        // Exactly as `save` returned them: `written`, `before` and
        // `copy_sha256` are what makes the undo exact.
        changes: saved.changes.clone(),
        created_copy: saved.created_copy,
        photo_path: photo,
        saved: true,
    };
    // The result and the run's copy in one transaction: both or neither.
    let copy = saved
        .created_copy
        .then_some((saved.target_name.as_str(), target_text.as_str()));
    let result_id = match ctx.store.add_result_with_copy(&new, copy, now) {
        Ok(id) => id,
        // A save the history doesn't hold could never be undone, and a
        // retry would make a second copy: put the preset back.
        Err(e) => {
            tracing::warn!("calibration: the save wasn't recorded ({e}); putting it back");
            return Err(
                match apply::undo(
                    &saved.target_path,
                    &saved.changes,
                    saved.created_copy,
                    &ctx.folders,
                    &ctx.presets,
                    ledger,
                ) {
                    Ok(()) => SAVE_NOT_RECORDED.to_string(),
                    Err(undo) => format!(
                        "The preset was saved, but the calibration history couldn't record it ({e}), and putting it back failed too: {undo}"
                    ),
                },
            );
        }
    };
    let mut assign_error = None;
    if saved.created_copy {
        if let (Some(ams), Some(tray)) = (run.ams_id, run.tray_id) {
            assign_error = assigner
                .assign(
                    ams,
                    tray,
                    &saved.target_name,
                    saved.filament_id.as_deref(),
                    &target_text,
                )
                .err();
        }
    }
    Ok(SaveAnswer::Saved {
        run: run_view(ctx, load_run(ctx, run.id)?)?,
        result_id,
        studio_running,
        assign_error,
    })
}

/// Records the K the user used. Nothing is written anywhere else.
pub(crate) fn record_for(
    ctx: &Ctx,
    run_id: i64,
    setup: Setup,
    reading: Reading,
    photo: Option<String>,
    now: i64,
) -> Result<RunView, String> {
    check_test(TestKind::PressureAdvance, &setup, Some(&reading))?;
    let _held = mutating();
    let run = load_run(ctx, run_id)?;
    let value = cal::value_of(&setup, &reading)?;
    let photo = checked_photo(ctx, photo)?;
    ctx.store.add_result(
        &NewResult {
            run_id: run.id,
            setup,
            reading,
            value,
            preset_key: None,
            old_value: None,
            new_value: Some(cal::format_value(TestKind::PressureAdvance, value)),
            target_path: None,
            variant: run.variant.clone(),
            backup_path: None,
            changes: Vec::new(),
            created_copy: false,
            photo_path: photo,
            saved: false,
        },
        now,
    )?;
    run_view(ctx, load_run(ctx, run.id)?)
}

/// Undoes the run's newest save. A created copy is moved away, and a slot
/// still assigned to it goes back to the original preset, or is cleared
/// when the preset was only detected. Ledger notes go to `ledger` even
/// when the undo fails.
///
/// The files are put back first, then the history in one transaction. If
/// that write fails ([`UNDO_NOT_RECORDED`]), a second Undo finds the copy
/// already gone, or the elements already back, and only records it.
pub(crate) fn undo_for(
    ctx: &Ctx,
    result_id: i64,
    assigner: &dyn SlotAssigner,
    now: i64,
    ledger: &mut Vec<LedgerNote>,
) -> Result<RunView, String> {
    let _held = mutating();
    let result = ctx
        .store
        .result(result_id)?
        .ok_or_else(|| RESULT_GONE.to_string())?;
    if !result.saved {
        return Err(NOTHING_TO_UNDO.into());
    }
    if result.undone {
        return Err(ALREADY_UNDONE.into());
    }
    let results = ctx.store.results(result.run_id)?;
    if newest_saved(&results).map(|r| r.id) != Some(result.id) {
        return Err(NEWER_FIRST.into());
    }
    let run = reconciled_run(ctx, result.run_id, now)?;
    // A save that made the copy, or edited it, is undone in the run's
    // copy: never in the preset it came from. A copy-making save whose run
    // lost the copy falls back to the path it recorded, which
    // `apply::undo` proves is that copy before moving it.
    let edited_copy = run.copy_path.is_some() && run.copy_path == result.target_path;
    let target = if result.created_copy || edited_copy {
        run.copy_path.as_deref().or(result.target_path.as_deref())
    } else {
        result.target_path.as_deref()
    }
    .ok_or_else(|| RESULT_GONE.to_string())?;
    let target = Path::new(target);
    // Only a copy that is really gone counts: a permission or I/O error
    // must not forget a copy that is still on disk.
    let copy_gone = result.created_copy
        && match std::fs::symlink_metadata(target) {
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                tracing::warn!("calibration: can't check result {}'s copy: {e}", result.id);
                return Err(READ_FAILED.into());
            }
        };
    if copy_gone {
        // Already moved away by an undo whose history write failed (or by
        // the user): only the history is left to put back.
        tracing::warn!(
            "calibration: result {}'s copy is already gone; recording its undo",
            result.id
        );
    } else {
        match apply::undo(
            target,
            &result.changes,
            result.created_copy,
            &ctx.folders,
            &ctx.presets,
            ledger,
        ) {
            Ok(()) => {}
            Err(e)
                if !result.created_copy
                    && e == apply::CHANGED_SINCE
                    && already_put_back(ctx, target, &result.changes) =>
            {
                tracing::warn!(
                    "calibration: result {}'s preset is already put back; recording its undo",
                    result.id
                );
            }
            Err(e) => return Err(e),
        }
    }
    if let Err(e) = ctx
        .store
        .mark_undone_with_copy(result.id, result.created_copy, now)
    {
        tracing::warn!(
            "calibration: undo of result {} not recorded: {e}",
            result.id
        );
        return Err(UNDO_NOT_RECORDED.into());
    }
    if result.created_copy {
        if let (Some(ams), Some(tray)) = (run.ams_id, run.tray_id) {
            if run.copy_name.is_some() && assigner.assigned_name(ams, tray) == run.copy_name {
                let restored = match run.slot_source {
                    Some(SlotSource::Assigned) => assigner.assign(
                        ams,
                        tray,
                        &run.preset_name,
                        run.filament_id.as_deref(),
                        &run.preset_path,
                    ),
                    _ => assigner.clear(ams, tray),
                };
                if let Err(e) = restored {
                    tracing::warn!("calibration: slot {ams}/{tray} not put back after undo: {e}");
                }
            }
        }
    }
    run_view(ctx, load_run(ctx, run.id)?)
}

pub(crate) fn skip_for(
    ctx: &Ctx,
    run_id: i64,
    step: StepKey,
    skipped: bool,
    now: i64,
) -> Result<RunView, String> {
    let _held = mutating();
    let run = load_run(ctx, run_id)?;
    ctx.store.set_skipped(run.id, step, skipped, now)?;
    run_view(ctx, load_run(ctx, run.id)?)
}

/// Stores the user's pick of nozzle variant, by the preset's own name.
pub(crate) fn set_variant_for(
    ctx: &Ctx,
    run_id: i64,
    variant: &str,
    now: i64,
) -> Result<RunView, String> {
    let _held = mutating();
    let view = run_view(ctx, load_run(ctx, run_id)?)?;
    let name = view
        .variants
        .iter()
        .find(|v| v.eq_ignore_ascii_case(variant.trim()))
        .ok_or(NOT_A_VARIANT)?;
    ctx.store.set_variant(view.id, name, now)?;
    run_view(ctx, load_run(ctx, view.id)?)
}

/// The prints a result may be linked to, from when its step was opened.
pub(crate) fn link_candidates_for(
    store: &CalibrationStore,
    journal: &JournalStore,
    result_id: i64,
) -> Result<LinkCandidates, String> {
    let result = store
        .result(result_id)?
        .ok_or_else(|| RESULT_GONE.to_string())?;
    let run = store
        .run(result.run_id)?
        .ok_or_else(|| RUN_GONE.to_string())?;
    let step = StepKey::of(result.test);
    let since = store
        .steps(run.id)?
        .into_iter()
        .find(|s| s.step == step)
        .map_or(run.created_at, |s| s.opened_at);
    journal.link_candidates(since)
}

/// Links a result to a journal print. The print linked before is unlinked
/// in the same journal transaction, when it still points at this result.
/// A print belongs to one result: one that named another result is taken
/// from it, and that result's link is cleared.
pub(crate) fn link_for(
    ctx: &Ctx,
    journal: &JournalStore,
    result_id: i64,
    journal_id: i64,
    now: i64,
) -> Result<RunView, String> {
    let _held = mutating();
    let result = ctx
        .store
        .result(result_id)?
        .ok_or_else(|| RESULT_GONE.to_string())?;
    if result.undone {
        return Err(LINK_UNDONE.into());
    }
    let entry = journal.get(journal_id)?.ok_or_else(|| GONE.to_string())?;
    if entry.outcome == Outcome::Printing {
        return Err(STILL_PRINTING.into());
    }
    let purpose = PrintPurpose {
        calibration_result_id: result.id,
        test: result.test,
    };
    let Linked::Linked { displaced } =
        journal.link_purpose(journal_id, &purpose, result.journal_entry_id, now)?
    else {
        return Err(GONE.into());
    };
    ctx.store
        .set_journal_entry(result.id, Some(journal_id), now)?;
    if let Some(other) = displaced {
        if let Some(r) = ctx.store.result(other)? {
            if r.journal_entry_id == Some(journal_id) {
                ctx.store.set_journal_entry(other, None, now)?;
            }
        }
    }
    run_view(ctx, load_run(ctx, result.run_id)?)
}

pub(crate) fn summaries(store: &CalibrationStore) -> Result<Vec<RunSummary>, String> {
    store
        .runs()?
        .into_iter()
        .map(|run| {
            Ok(RunSummary {
                result_ids: store.results(run.id)?.iter().map(|r| r.id).collect(),
                id: run.id,
                preset_name: run.preset_name,
                slot_label: run.slot_label,
                updated_at: run.updated_at,
            })
        })
        .collect()
}

/// Decodes, checks and re-encodes the page's photo, as the journal does,
/// and keeps it. `decode_upload` refuses over 25 MB, `prepare_photo`
/// accepts only JPEG, PNG and WebP within 64 MP and re-encodes to a JPEG
/// of at most 2048 px, and the store names it `<uuid>.jpg` in its photo
/// folder. Returns its file name and the JPEG.
pub(crate) fn keep_photo(
    store: &CalibrationStore,
    data_base64: &str,
) -> Result<(String, Vec<u8>), String> {
    let bytes = photos::decode_upload(data_base64)?;
    let jpeg = photos::prepare_photo(&bytes)?;
    let name = store.save_photo(&jpeg)?;
    Ok((name, jpeg))
}

/// The blocking half of a suggestion: checks the run, the setup and, for
/// the fine flow pass, a saved coarse pass, then keeps the photo. Returns its name, the JPEG the AI is shown, and the
/// labels it must pick from. A refused setup keeps no photo.
pub(crate) fn prepare_suggest(
    store: &CalibrationStore,
    run_id: i64,
    test: TestKind,
    setup: Setup,
    data_base64: &str,
) -> Result<(String, Vec<u8>, Vec<String>), String> {
    check_test(test, &setup, None)?;
    let (_, labels) = checked_diagram(&setup)?;
    store.run(run_id)?.ok_or_else(|| RUN_GONE.to_string())?;
    // The fine pass is refused until the coarse one is saved: no photo is
    // kept and no AI asked for a pass that can't be saved.
    if test == TestKind::FlowFine && !coarse_saved(store, run_id)? {
        return Err(COARSE_FIRST.into());
    }
    let (photo, jpeg) = keep_photo(store, data_base64)?;
    Ok((photo, jpeg, labels))
}

/// The suggestion for a kept photo, from `backend` when there is one. It
/// writes nothing: the page offers the pick and the user taps it.
pub(crate) async fn answer_suggest(
    backend: Option<&dyn VisionBackend>,
    photo: String,
    jpeg: &[u8],
    test: TestKind,
    labels: &[String],
) -> SuggestAnswer {
    let quiet = |photo: String, note: Option<&str>| SuggestAnswer {
        photo,
        suggestion: None,
        note: note.map(str::to_string),
    };
    if labels.is_empty() {
        return quiet(photo, None);
    }
    let Some(backend) = backend else {
        return quiet(photo, Some(NO_AI));
    };
    // `suggest` validates the reply against the labels: the provider's raw
    // JSON never reaches the page.
    match suggest::suggest(backend, jpeg, test, labels).await {
        Ok(s) => SuggestAnswer {
            photo,
            suggestion: Some(s),
            note: None,
        },
        Err(e) => SuggestAnswer {
            photo,
            suggestion: None,
            note: Some(e),
        },
    }
}

/// The configured AI provider, through the analyzer's vision path.
struct ConfiguredVision {
    provider: String,
    model: String,
    api_key: String,
}

#[async_trait::async_trait]
impl VisionBackend for ConfiguredVision {
    async fn complete(&self, image: &[u8], prompt: &str, schema: &Value) -> Result<Value, String> {
        crate::analyzer::vision::complete_vision_json(
            image,
            prompt,
            schema,
            &self.provider,
            &self.model,
            &self.api_key,
        )
        .await
    }
}

/// The vision backend `analyze_print` would use: only when the configured
/// model has vision, with the configured provider, model and key.
async fn configured_vision(app: &AppHandle) -> Option<ConfiguredVision> {
    if !crate::commands::models::get_ai_capabilities(app.clone())
        .await
        .vision
    {
        return None;
    }
    match crate::commands::analyzer::get_ai_settings(app) {
        Ok((provider, model, api_key)) => Some(ConfiguredVision {
            provider,
            model,
            api_key,
        }),
        Err(e) => {
            tracing::warn!("calibration: no AI settings: {e}");
            None
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| {
        tracing::warn!("calibration: no app data folder: {e}");
        READ_FAILED.to_string()
    })
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        tracing::warn!("calibration task failed: {e}");
        READ_FAILED.to_string()
    })?
}

fn open_store(dir: &Path) -> Result<CalibrationStore, String> {
    CalibrationStore::open(&CalibrationPaths::in_dir(dir))
}

/// The store, Bambu Studio's folders and the slicer's presets. The preset
/// index is checked against its folders on every call
/// (`SlicerService::presets` reloads it when any preset file changed), so
/// a preview or a save never reads a stale parent.
fn open_ctx(dir: &Path, slicer: &SlicerService) -> Result<Ctx, String> {
    let store = open_store(dir)?;
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {e}"))?;
    let presets = slicer.presets().map_err(|e| e.to_string())?;
    Ok(Ctx {
        store,
        presets,
        folders: PresetFolders::from_paths(&paths),
    })
}

/// Runs a preset-changing helper with a fresh ledger, then replays every
/// note it made, also when it returned an error or panicked partway: a
/// write the app's history should know about is never lost to a bug. A
/// panic is answered with [`INTERRUPTED`].
fn replaying<T>(
    f: impl FnOnce(&mut Vec<LedgerNote>) -> Result<T, String>,
    replay: impl FnOnce(&[LedgerNote]),
) -> Result<T, String> {
    let mut ledger = Vec::new();
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut ledger)));
    replay(&ledger);
    answer.unwrap_or_else(|_| {
        tracing::error!("calibration: a preset change panicked; its ledger notes were replayed");
        Err(INTERRUPTED.into())
    })
}

/// The app's history ledger, which tracks the presets BambuMate wrote new.
/// Every note is replayed, including those of a save or undo that failed
/// and put the file back.
fn note_ledger(notes: &[LedgerNote]) {
    for note in notes {
        match note {
            LedgerNote::Wrote(outcome, path) => {
                crate::history::ledger::note_new_preset_write(outcome, path)
            }
            LedgerNote::Forgot(path) => {
                crate::history::ledger::forget_preset(&crate::history::ledger::ledger_key(path))
            }
        }
    }
}

#[tauri::command]
pub async fn calibration_runs(app: AppHandle) -> Result<Vec<RunSummary>, String> {
    let dir = data_dir(&app)?;
    blocking(move || summaries(&open_store(&dir)?)).await
}

#[tauri::command]
pub async fn calibration_start(
    app: AppHandle,
    printer: State<'_, PrinterService>,
    slicer: State<'_, SlicerService>,
    slot: Option<String>,
    preset: Option<String>,
    run: Option<i64>,
) -> Result<RunView, String> {
    let (dir, printer, slicer) = (
        data_dir(&app)?,
        printer.inner().clone(),
        slicer.inner().clone(),
    );
    blocking(move || {
        let ctx = open_ctx(&dir, &slicer)?;
        start_run(
            &ctx,
            StartRequest { slot, preset, run },
            &printer.view(),
            now(),
        )
    })
    .await
}

#[tauri::command]
pub async fn calibration_set_variant(
    app: AppHandle,
    slicer: State<'_, SlicerService>,
    run_id: i64,
    variant: String,
) -> Result<RunView, String> {
    let (dir, slicer) = (data_dir(&app)?, slicer.inner().clone());
    blocking(move || set_variant_for(&open_ctx(&dir, &slicer)?, run_id, &variant, now())).await
}

#[tauri::command]
pub async fn calibration_defaults(
    app: AppHandle,
    slicer: State<'_, SlicerService>,
    run_id: i64,
    test: TestKind,
) -> Result<Defaults, String> {
    let (dir, slicer) = (data_dir(&app)?, slicer.inner().clone());
    blocking(move || defaults_for(&open_ctx(&dir, &slicer)?, run_id, test, now())).await
}

#[tauri::command]
pub async fn calibration_preview(
    app: AppHandle,
    slicer: State<'_, SlicerService>,
    run_id: i64,
    test: TestKind,
    setup: Setup,
    reading: Option<Reading>,
) -> Result<PreviewView, String> {
    let (dir, slicer) = (data_dir(&app)?, slicer.inner().clone());
    blocking(move || preview_for(&open_ctx(&dir, &slicer)?, run_id, test, setup, reading)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn calibration_save(
    app: AppHandle,
    printer: State<'_, PrinterService>,
    slicer: State<'_, SlicerService>,
    run_id: i64,
    test: TestKind,
    setup: Setup,
    reading: Reading,
    expected_old: String,
    photo: Option<String>,
) -> Result<SaveAnswer, String> {
    let (dir, printer, slicer) = (
        data_dir(&app)?,
        printer.inner().clone(),
        slicer.inner().clone(),
    );
    blocking(move || {
        let ctx = open_ctx(&dir, &slicer)?;
        let req = SaveRequest {
            run_id,
            test,
            setup,
            reading,
            expected_old,
            photo,
        };
        let studio_running = crate::profile::is_bambu_studio_running();
        replaying(
            |ledger| save_for(&ctx, req, &printer, studio_running, now(), ledger),
            note_ledger,
        )
    })
    .await
}

#[tauri::command]
pub async fn calibration_record(
    app: AppHandle,
    slicer: State<'_, SlicerService>,
    run_id: i64,
    setup: Setup,
    reading: Reading,
    photo: Option<String>,
) -> Result<RunView, String> {
    let (dir, slicer) = (data_dir(&app)?, slicer.inner().clone());
    blocking(move || {
        record_for(
            &open_ctx(&dir, &slicer)?,
            run_id,
            setup,
            reading,
            photo,
            now(),
        )
    })
    .await
}

#[tauri::command]
pub async fn calibration_undo(
    app: AppHandle,
    printer: State<'_, PrinterService>,
    slicer: State<'_, SlicerService>,
    result_id: i64,
) -> Result<RunView, String> {
    let (dir, printer, slicer) = (
        data_dir(&app)?,
        printer.inner().clone(),
        slicer.inner().clone(),
    );
    blocking(move || {
        let ctx = open_ctx(&dir, &slicer)?;
        replaying(
            |ledger| undo_for(&ctx, result_id, &printer, now(), ledger),
            note_ledger,
        )
    })
    .await
}

#[tauri::command]
pub async fn calibration_skip(
    app: AppHandle,
    slicer: State<'_, SlicerService>,
    run_id: i64,
    step: StepKey,
    skipped: bool,
) -> Result<RunView, String> {
    let (dir, slicer) = (data_dir(&app)?, slicer.inner().clone());
    blocking(move || skip_for(&open_ctx(&dir, &slicer)?, run_id, step, skipped, now())).await
}

/// Keeps the photo, then asks the configured vision backend for a pick
/// when there is one. Nothing else is written: the page offers the pick.
#[tauri::command]
pub async fn calibration_suggest(
    app: AppHandle,
    run_id: i64,
    test: TestKind,
    setup: Setup,
    photo_base64: String,
) -> Result<SuggestAnswer, String> {
    let dir = data_dir(&app)?;
    // A decoder that panics on a bad file is a photo that couldn't be saved.
    let (photo, jpeg, labels) = tauri::async_runtime::spawn_blocking(move || {
        prepare_suggest(&open_store(&dir)?, run_id, test, setup, &photo_base64)
    })
    .await
    .map_err(|e| {
        tracing::warn!("calibration photo task failed: {e}");
        PHOTO_FAILED.to_string()
    })??;
    // Pressure advance has no labels: no AI is asked, or even looked up.
    let backend = if labels.is_empty() {
        None
    } else {
        configured_vision(&app).await
    };
    Ok(answer_suggest(
        backend.as_ref().map(|b| b as &dyn VisionBackend),
        photo,
        &jpeg,
        test,
        &labels,
    )
    .await)
}

/// A kept photo as a `data:` URL, for the history's thumbnails.
#[tauri::command]
pub async fn calibration_image(app: AppHandle, file_name: String) -> Result<String, String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        open_store(&dir)?
            .photo_path(&file_name)
            .and_then(|p| photos::image_data_url(&p))
            .ok_or_else(|| PHOTO_UNAVAILABLE.to_string())
    })
    .await
}

#[tauri::command]
pub async fn calibration_link_candidates(
    app: AppHandle,
    result_id: i64,
) -> Result<LinkCandidates, String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        let journal = JournalStore::open(&JournalPaths::in_dir(&dir))?;
        link_candidates_for(&open_store(&dir)?, &journal, result_id)
    })
    .await
}

#[tauri::command]
pub async fn calibration_link_print(
    app: AppHandle,
    slicer: State<'_, SlicerService>,
    result_id: i64,
    journal_id: i64,
) -> Result<RunView, String> {
    let (dir, slicer) = (data_dir(&app)?, slicer.inner().clone());
    let run = blocking(move || {
        let journal = JournalStore::open(&JournalPaths::in_dir(&dir))?;
        link_for(
            &open_ctx(&dir, &slicer)?,
            &journal,
            result_id,
            journal_id,
            now(),
        )
    })
    .await?;
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(run)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use base64::Engine;
    use serde_json::json;

    use super::*;
    use crate::calibration::apply::testing::*;
    use crate::calibration::store::StepStatus;
    use crate::journal::types::{NewPrint, Outcome, PrintEnd};
    use crate::printer::filament_index::Detected;
    use crate::printer::service::PrinterSummary;
    use crate::printer::state::{Nozzle, Tray};

    #[derive(Default)]
    struct FakeAssigner {
        calls: Mutex<Vec<String>>,
        assigned: Mutex<Option<String>>,
    }

    impl SlotAssigner for FakeAssigner {
        fn assign(
            &self,
            ams_id: u32,
            tray_id: u32,
            name: &str,
            _filament_id: Option<&str>,
            _path: &str,
        ) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("assign {ams_id}/{tray_id} {name}"));
            *self.assigned.lock().unwrap() = Some(name.to_string());
            Ok(())
        }

        fn clear(&self, ams_id: u32, tray_id: u32) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("clear {ams_id}/{tray_id}"));
            *self.assigned.lock().unwrap() = None;
            Ok(())
        }

        fn assigned_name(&self, _ams_id: u32, _tray_id: u32) -> Option<String> {
            self.assigned.lock().unwrap().clone()
        }
    }

    fn ctx_in(root: &Path, data: &Path) -> Ctx {
        Ctx {
            store: CalibrationStore::open(&CalibrationPaths::in_dir(data)).unwrap(),
            presets: Arc::new(presets(root)),
            folders: folders(root),
        }
    }

    fn slot(label: &str, ams_id: u32, tray_id: u32, detected: Option<PathBuf>) -> SlotView {
        SlotView {
            ams_id,
            tray_id,
            label: label.into(),
            tray: Tray {
                id: tray_id,
                tray_type: "PLA".into(),
                tray_info_idx: "GFA00".into(),
                ..Default::default()
            },
            rfid: true,
            assigned_preset: None,
            assigned_filament_id: None,
            status: SlotStatus::Rfid,
            needs_cloud_sync: false,
            preset_has_no_id: false,
            detected: detected.map(|p| Detected {
                preset_name: SYSTEM_H2D.into(),
                path: p.to_string_lossy().into_owned(),
                is_user: false,
                variants: vec![SYSTEM_H2D.into()],
            }),
            not_installed: false,
            other_nozzle: None,
            material: None,
        }
    }

    /// An H2D with a standard right nozzle feeding A2 and a high-flow left one.
    fn printer(root: &Path, connected: bool) -> PrinterView {
        let nozzle = |id: u32, t: &str, tray_now: u32| Nozzle {
            id,
            diameter: Some(0.4),
            nozzle_type: Some(t.into()),
            tray_now: Some(tray_now),
            ..Default::default()
        };
        PrinterView {
            configured: true,
            printer: Some(PrinterSummary {
                ip: "192.168.1.20".into(),
                serial: "SN1".into(),
                name: "Workshop".into(),
                model: "H2D".into(),
                firmware: None,
                ca_verified: true,
            }),
            connection: if connected {
                ConnectionState::Connected
            } else {
                ConnectionState::Disconnected
            },
            state: Some(PrinterState {
                nozzles: vec![nozzle(0, "HS01", 1), nozzle(1, "HH01", 0xFFFF)],
                active_nozzle: Some(0),
                ..Default::default()
            }),
            slots: vec![
                slot("A2", 0, 1, Some(system_path(root, SYSTEM_H2D))),
                SlotView {
                    tray: Tray {
                        empty: true,
                        ..Default::default()
                    },
                    rfid: false,
                    status: SlotStatus::Empty,
                    ..slot("B4", 1, 3, None)
                },
            ],
            errors: Vec::new(),
        }
    }

    fn start_slot(ctx: &Ctx, view: &PrinterView, label: &str) -> Result<RunView, String> {
        start_run(
            ctx,
            StartRequest {
                slot: Some(label.into()),
                preset: None,
                run: None,
            },
            view,
            100,
        )
    }

    fn start_preset(ctx: &Ctx, view: &PrinterView, path: &Path) -> RunView {
        start_run(
            ctx,
            StartRequest {
                slot: None,
                preset: Some(path.to_string_lossy().into_owned()),
                run: None,
            },
            view,
            100,
        )
        .unwrap()
    }

    fn save_req(run: &RunView, setup: Setup, reading: Reading, expected_old: &str) -> SaveRequest {
        SaveRequest {
            run_id: run.id,
            test: setup.test(),
            setup,
            reading,
            expected_old: expected_old.into(),
            photo: None,
        }
    }

    fn temp_225(run: &RunView) -> SaveRequest {
        save_req(
            run,
            Setup::Temperature {
                start: 230,
                end: 190,
            },
            Reading::Temperature { band: 1 },
            "220",
        )
    }

    fn saved(answer: SaveAnswer) -> (RunView, i64) {
        match answer {
            SaveAnswer::Saved { run, result_id, .. } => (run, result_id),
            SaveAnswer::Changed { preview } => panic!("expected a save: {preview:?}"),
        }
    }

    #[test]
    fn a_run_starts_from_a_slot_with_the_printers_variant_and_is_reused() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let view = printer(root.path(), true);
        let run = start_slot(&ctx, &view, "A2").unwrap();
        assert_eq!(run.slot_label.as_deref(), Some("A2"));
        assert_eq!(
            run.variant.as_deref(),
            Some("Direct Drive Standard"),
            "nozzle 0 feeds A2"
        );
        assert!(run.is_system && !run.needs_variant);
        assert_eq!(
            run.will_save_as.as_deref(),
            Some("Bambu PLA Basic (calibrated) @BBL H2D")
        );
        assert_eq!(run.material, "PLA");
        assert_eq!(run.filament_id.as_deref(), Some("GFA00"));
        assert_eq!(
            (run.printer_model.as_str(), run.nozzle.as_str()),
            ("H2D", "0.4")
        );
        assert_eq!(run.variants.len(), 3);
        assert_eq!(run.steps.len(), 5);
        let again = start_preset(&ctx, &view, &system_path(root.path(), SYSTEM_H2D));
        assert_eq!(
            again.id, run.id,
            "the same preset on the same printer is the same run"
        );
        let reopened = start_run(
            &ctx,
            StartRequest {
                slot: None,
                preset: None,
                run: Some(run.id),
            },
            &view,
            300,
        )
        .unwrap();
        assert_eq!(reopened.id, run.id);
    }

    #[test]
    fn a_slot_needs_a_connected_printer_and_a_preset() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        assert_eq!(
            start_slot(&ctx, &printer(root.path(), false), "A2"),
            Err(NOT_CONNECTED.to_string())
        );
        assert_eq!(
            start_slot(&ctx, &printer(root.path(), true), "B4"),
            Err(NO_PRESET.to_string())
        );
        assert_eq!(
            start_run(
                &ctx,
                StartRequest {
                    slot: None,
                    preset: None,
                    run: None
                },
                &PrinterView::unconfigured(),
                100
            ),
            Err(PICK_ONE.to_string())
        );
        let no_printer = start_preset(
            &ctx,
            &PrinterView::unconfigured(),
            &user_path(root.path(), MY_PETG),
        );
        assert_eq!(no_printer.variant, None);
        assert!(
            no_printer.needs_variant,
            "two variants and no printer: the user picks"
        );
        assert_eq!(
            (
                no_printer.printer_model.as_str(),
                no_printer.nozzle.as_str()
            ),
            ("", "")
        );
    }

    #[test]
    fn saving_a_system_preset_makes_the_copy_assigns_the_slot_and_later_saves_edit_it() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let slots = FakeAssigner::default();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let mut ledger = Vec::new();
        let (run, first) =
            saved(save_for(&ctx, temp_225(&run), &slots, false, 200, &mut ledger).unwrap());
        assert_eq!(run.target_name, "Bambu PLA Basic (calibrated) @BBL H2D");
        assert!(!run.is_system && run.will_save_as.is_none());
        assert_eq!(
            *slots.calls.lock().unwrap(),
            vec!["assign 0/1 Bambu PLA Basic (calibrated) @BBL H2D".to_string()]
        );
        assert_eq!(
            run.steps[0].status,
            StepStatus::Saved {
                result_id: first,
                value: "225 °C".into(),
                at: 200
            }
        );
        assert!(run.results[0].can_undo);
        assert!(!ledger.is_empty());

        let flow = save_req(
            &run,
            Setup::FlowCoarse { base: 0.98 },
            Reading::FlowCoarse { modifier: 5 },
            "0.98",
        );
        let (run, _) = saved(save_for(&ctx, flow, &slots, false, 300, &mut ledger).unwrap());
        assert_eq!(
            run.target_name, "Bambu PLA Basic (calibrated) @BBL H2D",
            "no second copy"
        );
        let copy = raw(&user_path(
            root.path(),
            "Bambu PLA Basic (calibrated) @BBL H2D",
        ));
        assert_eq!(
            copy["filament_flow_ratio"],
            json!(["1.029", "0.98", "0.98"])
        );
        assert_eq!(copy["nozzle_temperature"], json!(["225", "220", "220"]));
        assert_eq!(slots.calls.lock().unwrap().len(), 1);
        assert!(
            !run.results[1].can_undo && run.results[0].can_undo,
            "newest first, only it undoes"
        );
    }

    #[test]
    fn a_stale_preview_answers_the_new_diff() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let mut req = temp_225(&run);
        req.expected_old = "999".into();
        let answer = save_for(
            &ctx,
            req,
            &FakeAssigner::default(),
            false,
            200,
            &mut Vec::new(),
        )
        .unwrap();
        let SaveAnswer::Changed { preview } = answer else {
            panic!("expected the new diff");
        };
        let change = preview.value.unwrap().change.unwrap();
        assert_eq!((change.old.as_str(), change.new.as_str()), ("220", "225"));
        assert_eq!(
            change.extra,
            Some(KeyDiff {
                key: "nozzle_temperature_initial_layer".into(),
                old: "220".into(),
                new: "225".into()
            })
        );
        assert!(change.will_copy);
    }

    #[test]
    fn the_fine_pass_waits_for_a_saved_coarse_pass() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let run = start_preset(
            &ctx,
            &printer(root.path(), true),
            &user_path(root.path(), MY_PETG),
        );
        assert!(
            run.needs_variant,
            "the nozzles differ and no slot was picked"
        );
        assert_eq!(
            defaults_for(&ctx, run.id, TestKind::FlowCoarse, 150),
            Err(crate::calibration::apply::PICK_VARIANT.to_string())
        );
        assert_eq!(
            set_variant_for(&ctx, run.id, "Bowden", 150),
            Err(NOT_A_VARIANT.to_string())
        );
        let run = set_variant_for(&ctx, run.id, "direct drive high flow", 160).unwrap();
        assert_eq!(run.variant.as_deref(), Some("Direct Drive High Flow"));
        let fine = Setup::FlowFine { base: 0.95 };
        assert_eq!(
            preview_for(&ctx, run.id, TestKind::FlowFine, fine, None),
            Err(COARSE_FIRST.to_string())
        );
        let d = defaults_for(&ctx, run.id, TestKind::FlowCoarse, 170).unwrap();
        assert_eq!(d.setup, Setup::FlowCoarse { base: 0.95 });
        assert_eq!(d.current.as_deref(), Some("0.95"));
        let coarse = save_req(&run, d.setup, Reading::FlowCoarse { modifier: -5 }, "0.95");
        let (run, _) = saved(
            save_for(
                &ctx,
                coarse,
                &FakeAssigner::default(),
                false,
                200,
                &mut Vec::new(),
            )
            .unwrap(),
        );
        assert!(run.steps[1].fine_ready);
        let d = defaults_for(&ctx, run.id, TestKind::FlowFine, 210).unwrap();
        assert_eq!(
            d.setup,
            Setup::FlowFine { base: 0.9025 },
            "the saved coarse result"
        );
        let p = preview_for(
            &ctx,
            run.id,
            TestKind::FlowFine,
            d.setup,
            Some(Reading::FlowFine { modifier: -2 }),
        )
        .unwrap();
        assert_eq!(p.labels.len(), 10);
        assert_eq!(p.value.unwrap().text, "0.88445");
    }

    #[test]
    fn pressure_advance_is_recorded_and_never_saved() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let system = system_path(root.path(), SYSTEM_H2D);
        let before = std::fs::read_to_string(&system).unwrap();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let pa = save_req(
            &run,
            Setup::PressureAdvance {},
            Reading::PressureAdvance { k: 0.025 },
            "",
        );
        assert_eq!(
            save_for(
                &ctx,
                pa,
                &FakeAssigner::default(),
                false,
                200,
                &mut Vec::new()
            ),
            Err(crate::calibration::apply::RECORD_ONLY.to_string())
        );
        let run = record_for(
            &ctx,
            run.id,
            Setup::PressureAdvance {},
            Reading::PressureAdvance { k: 0.025 },
            None,
            300,
        )
        .unwrap();
        assert!(
            matches!(&run.steps[4].status, StepStatus::Recorded { value, .. } if value == "K 0.025")
        );
        assert!(!run.results[0].saved && !run.results[0].can_undo);
        assert_eq!(std::fs::read_to_string(&system).unwrap(), before);
        assert!(!user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D").exists());
    }

    #[test]
    fn undo_goes_newest_first_and_puts_the_slot_back() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let slots = FakeAssigner::default();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let mut ledger = Vec::new();
        let (run, temp) =
            saved(save_for(&ctx, temp_225(&run), &slots, false, 200, &mut ledger).unwrap());
        let flow = save_req(
            &run,
            Setup::FlowCoarse { base: 0.98 },
            Reading::FlowCoarse { modifier: 5 },
            "0.98",
        );
        let (_, flow) = saved(save_for(&ctx, flow, &slots, false, 300, &mut ledger).unwrap());
        assert_eq!(
            undo_for(&ctx, temp, &slots, 400, &mut ledger),
            Err(NEWER_FIRST.to_string())
        );
        let run = undo_for(&ctx, flow, &slots, 410, &mut ledger).unwrap();
        assert_eq!(run.steps[1].status, StepStatus::NotRun);
        let run = undo_for(&ctx, temp, &slots, 420, &mut ledger).unwrap();
        assert!(!user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D").exists());
        assert_eq!(run.target_name, SYSTEM_H2D);
        assert!(run.is_system);
        assert_eq!(
            slots.calls.lock().unwrap().last().map(String::as_str),
            Some("clear 0/1")
        );
        assert!(run.results.iter().all(|r| r.undone));
        assert_eq!(
            undo_for(&ctx, temp, &slots, 430, &mut ledger),
            Err(ALREADY_UNDONE.to_string())
        );
    }

    #[test]
    fn linking_a_print_sets_its_purpose_and_moves_it() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let journal = JournalStore::open(&JournalPaths::in_dir(data.path())).unwrap();
        let print = |name: &str, start: i64| {
            let id = journal
                .start(
                    &NewPrint {
                        job_key: name.into(),
                        name: name.into(),
                        gcode_file: None,
                        plate: None,
                        printer_model: None,
                        started_at: start,
                        start_approx: false,
                    },
                    start,
                )
                .unwrap();
            journal
                .end(
                    id,
                    &PrintEnd {
                        outcome: Outcome::Finished,
                        ended_at: Some(start + 100),
                        error_code: None,
                        hms: Vec::new(),
                    },
                    start + 100,
                )
                .unwrap();
            id
        };
        let older = print("Older", 500);
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        defaults_for(&ctx, run.id, TestKind::PressureAdvance, 1_000).unwrap();
        let first = print("PA lines", 1_100);
        let second = print("PA pattern", 1_300);
        let run = record_for(
            &ctx,
            run.id,
            Setup::PressureAdvance {},
            Reading::PressureAdvance { k: 0.02 },
            None,
            1_500,
        )
        .unwrap();
        let result = run.results[0].id;
        let found = link_candidates_for(&ctx.store, &journal, result).unwrap();
        assert_eq!(found.suggested, Some(second));
        let ids: Vec<i64> = found.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![second, first, older]);

        let run = link_for(&ctx, &journal, result, first, 1_600).unwrap();
        assert_eq!(run.results[0].journal_entry_id, Some(first));
        let purpose = journal.get(first).unwrap().unwrap().purpose.unwrap();
        assert_eq!(
            (purpose.calibration_result_id, purpose.test),
            (result, TestKind::PressureAdvance)
        );
        link_for(&ctx, &journal, result, second, 1_700).unwrap();
        assert_eq!(
            journal.get(first).unwrap().unwrap().purpose,
            None,
            "moved to the other print"
        );
        assert!(journal.get(second).unwrap().unwrap().purpose.is_some());
        assert_eq!(
            link_for(&ctx, &journal, result, 999, 1_800)
                .err()
                .as_deref(),
            Some(GONE)
        );
        // The refused link changed nothing: the result still points at its print.
        let run = start_run(
            &ctx,
            StartRequest {
                slot: None,
                preset: None,
                run: Some(run.id),
            },
            &PrinterView::unconfigured(),
            1_900,
        )
        .unwrap();
        assert_eq!(run.results[0].journal_entry_id, Some(second));
        assert!(journal.get(second).unwrap().unwrap().purpose.is_some());
    }

    #[test]
    fn a_photo_is_kept_and_only_its_own_name_is_accepted() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(64, 64)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png.get_ref());
        let (name, jpeg) = keep_photo(&ctx.store, &b64).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]), "re-encoded as JPEG");
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let pa = |photo: &str| {
            record_for(
                &ctx,
                run.id,
                Setup::PressureAdvance {},
                Reading::PressureAdvance { k: 0.02 },
                Some(photo.into()),
                200,
            )
        };
        assert_eq!(pa("../calibration.db").err().as_deref(), Some(PHOTO_GONE));
        let run = pa(&name).unwrap();
        assert_eq!(run.results[0].photo.as_deref(), Some(name.as_str()));
        assert_eq!(
            keep_photo(&ctx.store, "not base64!").err().as_deref(),
            Some(crate::journal::photos::NOT_AN_IMAGE)
        );
    }

    #[test]
    fn summaries_list_each_run_with_its_results() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let run = record_for(
            &ctx,
            run.id,
            Setup::PressureAdvance {},
            Reading::PressureAdvance { k: 0.02 },
            None,
            200,
        )
        .unwrap();
        let list = summaries(&ctx.store).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].slot_label.as_deref(), Some("A2"));
        assert_eq!(list[0].result_ids, vec![run.results[0].id]);
    }

    // Beyond the plan: the review requirements for Tasks 1, 3 and 4.

    fn png_base64(width: u32, height: u32) -> String {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(png.get_ref())
    }

    fn photo_count(data: &Path) -> usize {
        std::fs::read_dir(data.join("calibration/photos"))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    #[test]
    fn a_tower_too_tall_to_print_is_refused_before_its_labels_are_built() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        // 1.4 + 2 / 0.0001 mm: 20,001 labels.
        let tall = Setup::Retraction {
            start: 0.0,
            end: 2.0,
            step: 0.0001,
        };
        assert_eq!(
            preview_for(&ctx, run.id, TestKind::Retraction, tall, None),
            Err(TOO_TALL.to_string())
        );
        let req = save_req(&run, tall, Reading::Retraction { height_mm: 5.0 }, "0.4");
        assert_eq!(
            save_for(
                &ctx,
                req,
                &FakeAssigner::default(),
                false,
                200,
                &mut Vec::new()
            ),
            Err(TOO_TALL.to_string())
        );
        let speed = Setup::MaxVolumetricSpeed {
            start: 5.0,
            end: 200.0,
            step: 0.1,
        };
        assert_eq!(
            prepare_suggest(
                &ctx.store,
                run.id,
                TestKind::MaxVolumetricSpeed,
                speed,
                &png_base64(8, 8)
            )
            .err()
            .as_deref(),
            Some(TOO_TALL)
        );
        assert_eq!(
            photo_count(data.path()),
            0,
            "a refused setup keeps no photo"
        );
        // The tallest tower that fits: (200 - 5 + 1) / 0.5 = 392 mm, 393 labels.
        let fits = Setup::MaxVolumetricSpeed {
            start: 5.0,
            end: 200.0,
            step: 0.5,
        };
        let p = preview_for(&ctx, run.id, TestKind::MaxVolumetricSpeed, fits, None).unwrap();
        assert_eq!(p.labels.len(), 393);
    }

    #[test]
    fn a_retraction_left_to_the_printer_reads_as_printer_default() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let run = start_preset(
            &ctx,
            &PrinterView::unconfigured(),
            &user_path(root.path(), MY_PETG),
        );
        let run = set_variant_for(&ctx, run.id, "Direct Drive Standard", 110).unwrap();
        let setup = Setup::Retraction {
            start: 0.0,
            end: 2.0,
            step: 0.1,
        };
        let reading = Reading::Retraction { height_mm: 5.0 };
        let p = preview_for(&ctx, run.id, TestKind::Retraction, setup, Some(reading)).unwrap();
        let change = p.value.unwrap().change.unwrap();
        assert_eq!(
            (
                change.old.as_str(),
                change.old_display.as_str(),
                change.new.as_str()
            ),
            ("nil", "printer default", "0.4"),
            "the raw old value is what a save must be handed back"
        );
        let req = save_req(&run, setup, reading, &change.old);
        let (run, _) = saved(
            save_for(
                &ctx,
                req,
                &FakeAssigner::default(),
                false,
                200,
                &mut Vec::new(),
            )
            .unwrap(),
        );
        assert_eq!(run.results[0].old_value.as_deref(), Some("nil"));
        assert_eq!(
            run.results[0].old_display.as_deref(),
            Some("printer default")
        );
        // Only an override key's "nil" is the printer's: any other key keeps its text.
        let temp = Setup::Temperature {
            start: 250,
            end: 230,
        };
        let p = preview_for(
            &ctx,
            run.id,
            TestKind::Temperature,
            temp,
            Some(Reading::Temperature { band: 0 }),
        )
        .unwrap();
        let change = p.value.unwrap().change.unwrap();
        assert_eq!(
            (change.old.as_str(), change.old_display.as_str()),
            ("240", "240")
        );
        assert_eq!(old_display("filament_flow_ratio", "nil"), "nil");
        assert_eq!(
            old_display("filament_retraction_length", "nil"),
            "printer default"
        );
    }

    struct Stub {
        reply: serde_json::Value,
        images: Mutex<Vec<Vec<u8>>>,
    }

    #[async_trait::async_trait]
    impl VisionBackend for Stub {
        async fn complete(
            &self,
            image: &[u8],
            _prompt: &str,
            _schema: &Value,
        ) -> Result<Value, String> {
            self.images.lock().unwrap().push(image.to_vec());
            Ok(self.reply.clone())
        }
    }

    #[tokio::test]
    async fn a_suggestion_is_offered_and_only_the_photo_is_written() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let system = system_path(root.path(), SYSTEM_H2D);
        let before = std::fs::read_to_string(&system).unwrap();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let setup = Setup::Temperature {
            start: 230,
            end: 190,
        };
        let (photo, jpeg, labels) = prepare_suggest(
            &ctx.store,
            run.id,
            TestKind::Temperature,
            setup,
            &png_base64(64, 48),
        )
        .unwrap();
        assert!(
            ctx.store.photo_path(&photo).is_some(),
            "kept in the photo folder"
        );
        assert_eq!(labels.len(), 9);
        let stub = Stub {
            reply: json!({"pick":"225 °C","confidence":0.8,"reason":"Least stringing."}),
            images: Mutex::new(Vec::new()),
        };
        let answer = answer_suggest(
            Some(&stub),
            photo.clone(),
            &jpeg,
            TestKind::Temperature,
            &labels,
        )
        .await;
        assert_eq!(answer.photo, photo);
        assert_eq!(answer.suggestion.map(|s| s.pick).as_deref(), Some("225 °C"));
        assert_eq!(answer.note, None);
        assert_eq!(
            *stub.images.lock().unwrap(),
            vec![jpeg.clone()],
            "the re-encoded JPEG"
        );
        // A reply naming no label is the plain failure, never a pick.
        let wrong = Stub {
            reply: json!({"pick":"999 °C","confidence":0.8,"reason":""}),
            images: Mutex::new(Vec::new()),
        };
        let answer = answer_suggest(
            Some(&wrong),
            photo.clone(),
            &jpeg,
            TestKind::Temperature,
            &labels,
        )
        .await;
        assert_eq!(
            (answer.suggestion, answer.note.as_deref()),
            (None, Some(crate::calibration::suggest::SUGGEST_FAILED))
        );
        let none = answer_suggest(None, photo.clone(), &jpeg, TestKind::Temperature, &labels).await;
        assert_eq!((none.suggestion, none.note.as_deref()), (None, Some(NO_AI)));
        // Nothing but the photo was written.
        assert!(ctx.store.results(run.id).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&system).unwrap(), before);
        assert!(!user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D").exists());
        assert_eq!(photo_count(data.path()), 1);
        // Pressure advance has nothing to pick: the photo is kept, no AI asked.
        let (photo, jpeg, labels) = prepare_suggest(
            &ctx.store,
            run.id,
            TestKind::PressureAdvance,
            Setup::PressureAdvance {},
            &png_base64(8, 8),
        )
        .unwrap();
        let quiet = answer_suggest(
            Some(&stub),
            photo,
            &jpeg,
            TestKind::PressureAdvance,
            &labels,
        )
        .await;
        assert_eq!((quiet.suggestion, quiet.note), (None, None));
        assert_eq!(stub.images.lock().unwrap().len(), 1, "not asked again");
    }

    #[test]
    fn a_suggestion_needs_a_run_a_matching_setup_and_a_photo() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let setup = Setup::Temperature {
            start: 230,
            end: 190,
        };
        assert_eq!(
            prepare_suggest(
                &ctx.store,
                42,
                TestKind::Temperature,
                setup,
                &png_base64(8, 8)
            )
            .err()
            .as_deref(),
            Some(RUN_GONE)
        );
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        assert_eq!(
            prepare_suggest(
                &ctx.store,
                run.id,
                TestKind::FlowCoarse,
                setup,
                &png_base64(8, 8)
            )
            .err()
            .as_deref(),
            Some(cal::OTHER_TEST)
        );
        assert_eq!(
            prepare_suggest(
                &ctx.store,
                run.id,
                TestKind::Temperature,
                setup,
                "bm90IGEgcGhvdG8="
            )
            .err()
            .as_deref(),
            Some(crate::journal::photos::NOT_AN_IMAGE)
        );
        // The fine pass waits for a saved coarse one, before any photo or AI.
        let fine = Setup::FlowFine { base: 0.98 };
        assert_eq!(
            prepare_suggest(
                &ctx.store,
                run.id,
                TestKind::FlowFine,
                fine,
                &png_base64(8, 8)
            )
            .err()
            .as_deref(),
            Some(COARSE_FIRST)
        );
        assert_eq!(photo_count(data.path()), 0);
    }

    // Fix round 1: serialised writes, recoverable history writes, links.

    fn json_count(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.contains("(calibrated)") && name.ends_with(".json")
            })
            .count()
    }

    /// A trigger, from a second connection, that makes the store's writes
    /// `on` fail.
    fn refuse(data: &Path, name: &str, on: &str) {
        rusqlite::Connection::open(data.join("calibration.db"))
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER {name} BEFORE {on} BEGIN SELECT RAISE(ABORT, 'test'); END;"
            ))
            .unwrap();
    }

    fn allow(data: &Path, name: &str) {
        rusqlite::Connection::open(data.join("calibration.db"))
            .unwrap()
            .execute_batch(&format!("DROP TRIGGER {name};"))
            .unwrap();
    }

    /// MY_PETG's run at its high-flow variant, with no printer.
    fn petg_run(ctx: &Ctx, root: &Path) -> RunView {
        let run = start_preset(ctx, &PrinterView::unconfigured(), &user_path(root, MY_PETG));
        set_variant_for(ctx, run.id, "Direct Drive High Flow", 110).unwrap()
    }

    fn petg_temp(run: &RunView) -> SaveRequest {
        save_req(
            run,
            Setup::Temperature {
                start: 250,
                end: 230,
            },
            Reading::Temperature { band: 0 },
            "245",
        )
    }

    #[test]
    fn two_saves_at_once_to_one_preset_both_reach_the_file() {
        // Several rounds: without the lock the two writes overlap in most.
        for _ in 0..8 {
            let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
            let run = petg_run(&ctx_in(root.path(), data.path()), root.path());
            let flow = save_req(
                &run,
                Setup::FlowCoarse { base: 0.95 },
                Reading::FlowCoarse { modifier: 5 },
                "0.95",
            );
            let barrier = std::sync::Barrier::new(2);
            let save = |req: SaveRequest| {
                let ctx = ctx_in(root.path(), data.path());
                barrier.wait();
                save_for(
                    &ctx,
                    req,
                    &FakeAssigner::default(),
                    false,
                    200,
                    &mut Vec::new(),
                )
            };
            let (a, b) = std::thread::scope(|s| {
                let a = s.spawn(|| save(petg_temp(&run)));
                let b = s.spawn(|| save(flow));
                (a.join().unwrap(), b.join().unwrap())
            });
            saved(a.unwrap());
            saved(b.unwrap());
            let file = raw(&user_path(root.path(), MY_PETG));
            assert_eq!(file["nozzle_temperature"], json!(["240", "250"]));
            assert_eq!(file["filament_flow_ratio"], json!(["0.97", "0.9975"]));
        }
    }

    #[test]
    fn two_rapid_saves_on_a_system_preset_make_one_copy() {
        for _ in 0..5 {
            let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
            let run = start_slot(
                &ctx_in(root.path(), data.path()),
                &printer(root.path(), true),
                "A2",
            )
            .unwrap();
            let slots = FakeAssigner::default();
            let barrier = std::sync::Barrier::new(2);
            let save = || {
                let ctx = ctx_in(root.path(), data.path());
                barrier.wait();
                save_for(&ctx, temp_225(&run), &slots, false, 200, &mut Vec::new()).unwrap()
            };
            let answers = std::thread::scope(|s| {
                let a = s.spawn(save);
                let b = s.spawn(save);
                [a.join().unwrap(), b.join().unwrap()]
            });
            let saves = answers
                .iter()
                .filter(|a| matches!(a, SaveAnswer::Saved { .. }))
                .count();
            assert_eq!(
                saves, 1,
                "the second reads the copy the first made: {answers:?}"
            );
            let base = user_path(root.path(), "x").parent().unwrap().to_path_buf();
            assert_eq!(json_count(&base), 1, "exactly one copy");
            assert_eq!(slots.calls.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn a_run_that_lost_its_copy_takes_it_back_and_makes_no_second_one() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let slots = FakeAssigner::default();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let (run, _) =
            saved(save_for(&ctx, temp_225(&run), &slots, false, 200, &mut Vec::new()).unwrap());
        // The run forgets its copy, as a failed history write once could leave it.
        ctx.store.set_copy(run.id, None, 250).unwrap();
        let flow = save_req(
            &run,
            Setup::FlowCoarse { base: 0.98 },
            Reading::FlowCoarse { modifier: 5 },
            "0.98",
        );
        let (run, _) = saved(save_for(&ctx, flow, &slots, false, 300, &mut Vec::new()).unwrap());
        assert_eq!(run.target_name, "Bambu PLA Basic (calibrated) @BBL H2D");
        let copy = user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D");
        assert_eq!(json_count(copy.parent().unwrap()), 1, "no \"(2)\"");
        assert_eq!(
            raw(&copy)["filament_flow_ratio"],
            json!(["1.029", "0.98", "0.98"])
        );
    }

    #[test]
    fn a_save_the_history_cant_record_is_put_back() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let slots = FakeAssigner::default();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let petg = petg_run(&ctx, root.path());
        let petg_before = raw(&user_path(root.path(), MY_PETG));
        refuse(data.path(), "no_results", "INSERT ON results");
        // A system preset: the copy goes again.
        let mut ledger = Vec::new();
        assert_eq!(
            save_for(&ctx, temp_225(&run), &slots, false, 200, &mut ledger),
            Err(SAVE_NOT_RECORDED.to_string())
        );
        let copy = user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D");
        assert!(!copy.exists());
        assert!(
            matches!(ledger.last(), Some(LedgerNote::Forgot(_))),
            "{ledger:?}"
        );
        assert!(slots.calls.lock().unwrap().is_empty());
        // A user preset: every element back as it was.
        assert_eq!(
            save_for(&ctx, petg_temp(&petg), &slots, false, 200, &mut Vec::new()),
            Err(SAVE_NOT_RECORDED.to_string())
        );
        let now = raw(&user_path(root.path(), MY_PETG));
        for key in ["nozzle_temperature", "nozzle_temperature_initial_layer"] {
            assert_eq!(now[key], petg_before[key], "{key}");
        }
        // Once the history works again, the save makes the copy with its own name.
        allow(data.path(), "no_results");
        let (run, _) =
            saved(save_for(&ctx, temp_225(&run), &slots, false, 300, &mut Vec::new()).unwrap());
        assert_eq!(run.target_name, "Bambu PLA Basic (calibrated) @BBL H2D");
        assert_eq!(json_count(copy.parent().unwrap()), 1);
    }

    #[test]
    fn an_undo_the_history_cant_record_is_finished_by_the_next_undo() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let slots = FakeAssigner::default();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let (_, temp) =
            saved(save_for(&ctx, temp_225(&run), &slots, false, 200, &mut Vec::new()).unwrap());
        let petg = petg_run(&ctx, root.path());
        let (_, petg_temp_id) =
            saved(save_for(&ctx, petg_temp(&petg), &slots, false, 210, &mut Vec::new()).unwrap());
        refuse(data.path(), "no_undo", "UPDATE OF undone ON results");
        // The copy is moved away, but the history still has it.
        assert_eq!(
            undo_for(&ctx, temp, &slots, 300, &mut Vec::new()),
            Err(UNDO_NOT_RECORDED.to_string())
        );
        let copy = user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D");
        assert!(!copy.exists());
        // A user preset: the elements are back, the history not yet.
        assert_eq!(
            undo_for(&ctx, petg_temp_id, &slots, 300, &mut Vec::new()),
            Err(UNDO_NOT_RECORDED.to_string())
        );
        assert_eq!(
            raw(&user_path(root.path(), MY_PETG))["nozzle_temperature"],
            json!(["240", "245"])
        );
        allow(data.path(), "no_undo");
        let run = undo_for(&ctx, temp, &slots, 400, &mut Vec::new()).unwrap();
        assert!(
            run.is_system && run.target_name == SYSTEM_H2D,
            "back on the system preset"
        );
        assert!(run.results.iter().all(|r| r.undone));
        assert_eq!(
            slots.calls.lock().unwrap().last().map(String::as_str),
            Some("clear 0/1")
        );
        let petg = undo_for(&ctx, petg_temp_id, &slots, 400, &mut Vec::new()).unwrap();
        assert!(petg.results[0].undone);
        assert_eq!(
            raw(&user_path(root.path(), MY_PETG))["nozzle_temperature"],
            json!(["240", "245"])
        );
    }

    #[test]
    fn a_print_links_to_one_result_and_only_once_it_has_ended() {
        let (root, data) = (fake_root(), tempfile::tempdir().unwrap());
        let ctx = ctx_in(root.path(), data.path());
        let journal = JournalStore::open(&JournalPaths::in_dir(data.path())).unwrap();
        let new_print = |name: &str, start: i64| NewPrint {
            job_key: name.into(),
            name: name.into(),
            gcode_file: None,
            plate: None,
            printer_model: None,
            started_at: start,
            start_approx: false,
        };
        let done = journal.start(&new_print("Done", 1_000), 1_000).unwrap();
        journal
            .end(
                done,
                &PrintEnd {
                    outcome: Outcome::Failed,
                    ended_at: Some(1_100),
                    error_code: None,
                    hms: Vec::new(),
                },
                1_100,
            )
            .unwrap();
        let running = journal.start(&new_print("Running", 1_200), 1_200).unwrap();
        let run = start_slot(&ctx, &printer(root.path(), true), "A2").unwrap();
        let k = |k: f64, at: i64| {
            record_for(
                &ctx,
                run.id,
                Setup::PressureAdvance {},
                Reading::PressureAdvance { k },
                None,
                at,
            )
            .unwrap()
            .results[0]
                .id
        };
        let (first, second) = (k(0.02, 1_300), k(0.03, 1_310));
        link_for(&ctx, &journal, first, done, 1_400).unwrap();
        let view = link_for(&ctx, &journal, second, done, 1_500).unwrap();
        let link = |id: i64| {
            view.results
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .journal_entry_id
        };
        assert_eq!(
            (link(first), link(second)),
            (None, Some(done)),
            "taken from the first"
        );
        assert_eq!(
            journal
                .get(done)
                .unwrap()
                .unwrap()
                .purpose
                .unwrap()
                .calibration_result_id,
            second
        );
        assert_eq!(
            link_for(&ctx, &journal, second, running, 1_600)
                .err()
                .as_deref(),
            Some(STILL_PRINTING)
        );
        assert_eq!(journal.get(running).unwrap().unwrap().purpose, None);
        // An undone save has nothing to link.
        let slots = FakeAssigner::default();
        let (_, temp) =
            saved(save_for(&ctx, temp_225(&run), &slots, false, 1_700, &mut Vec::new()).unwrap());
        undo_for(&ctx, temp, &slots, 1_800, &mut Vec::new()).unwrap();
        assert_eq!(
            link_for(&ctx, &journal, temp, done, 1_900).err().as_deref(),
            Some(LINK_UNDONE)
        );
        assert_eq!(
            journal
                .get(done)
                .unwrap()
                .unwrap()
                .purpose
                .unwrap()
                .calibration_result_id,
            second
        );
    }

    #[test]
    fn a_panic_still_replays_the_ledger_and_leaves_the_lock_usable() {
        let note = LedgerNote::Forgot(PathBuf::from("/tmp/x.json"));
        let mut replayed = Vec::new();
        let answer: Result<(), String> = replaying(
            |ledger| {
                let _held = mutating();
                ledger.push(note.clone());
                panic!("a bug mid-save");
            },
            |notes| replayed = notes.to_vec(),
        );
        assert_eq!(answer, Err(INTERRUPTED.to_string()));
        assert_eq!(replayed, vec![note.clone()]);
        drop(mutating());
        let mut replayed = Vec::new();
        let answer: Result<(), String> = replaying(
            |ledger| {
                ledger.push(note.clone());
                Err("refused".into())
            },
            |notes| replayed = notes.to_vec(),
        );
        assert_eq!((answer, replayed), (Err("refused".to_string()), vec![note]));
    }

    // The frontend's mirrors (`src/calibration/types.rs`) are tested against
    // this file: it holds what the real types above serialize. Run with
    // `UPDATE_WIRE_FIXTURE=1` to rewrite it after a deliberate wire change.
    #[test]
    fn the_wire_fixture_holds_what_the_real_types_serialize() {
        use crate::calibration::store::{StepKey, StepView};
        use crate::journal::types::{Photo, TrayUse, Verdict};

        fn to<T: serde::Serialize>(v: &T) -> Value {
            serde_json::to_value(v).unwrap()
        }
        let setups = json!({
            "temperature": to(&Setup::Temperature { start: 230, end: 190 }),
            "flow_coarse": to(&Setup::FlowCoarse { base: 0.98 }),
            "flow_fine": to(&Setup::FlowFine { base: 1.029 }),
            "retraction": to(&Setup::Retraction { start: 0.0, end: 2.0, step: 0.1 }),
            "max_volumetric_speed": to(&Setup::MaxVolumetricSpeed { start: 5.0, end: 20.0, step: 0.5 }),
            "pressure_advance": to(&Setup::PressureAdvance {}),
        });
        let readings = json!({
            "temperature": to(&Reading::Temperature { band: 1 }),
            "flow_coarse": to(&Reading::FlowCoarse { modifier: 5 }),
            "flow_fine": to(&Reading::FlowFine { modifier: -6 }),
            "retraction": to(&Reading::Retraction { height_mm: 5.0 }),
            "max_volumetric_speed": to(&Reading::MaxVolumetricSpeed { height_mm: 14.0 }),
            "pressure_advance": to(&Reading::PressureAdvance { k: 0.025 }),
        });
        let change = ChangeView {
            preset_key: "filament_retraction_length".into(),
            old: "nil".into(),
            old_display: PRINTER_DEFAULT.into(),
            new: "0.6".into(),
            extra: Some(KeyDiff {
                key: "nozzle_temperature_initial_layer".into(),
                old: "225".into(),
                new: "230".into(),
            }),
            target_name: "Bambu PLA Basic (calibrated) @BBL H2D".into(),
            will_copy: true,
            variant: Some("Direct Drive Standard".into()),
        };
        let value = ValueView {
            value: 225.0,
            text: "225".into(),
            display: "225 °C".into(),
            change: Some(change.clone()),
            change_error: None,
        };
        let preview = |diagram: Diagram, value: Option<ValueView>| PreviewView {
            labels: cal::labels(&diagram),
            diagram,
            value,
        };
        let result = ResultView {
            id: 11,
            test: TestKind::Retraction,
            value: "0.6".into(),
            preset_key: Some("filament_retraction_length".into()),
            old_value: Some("nil".into()),
            old_display: Some(PRINTER_DEFAULT.into()),
            new_value: Some("0.6".into()),
            target_name: "Bambu PLA Basic (calibrated) @BBL H2D".into(),
            variant: Some("Direct Drive Standard".into()),
            saved: true,
            undone: false,
            created_at: 300,
            photo: Some("3f2a.jpg".into()),
            journal_entry_id: Some(42),
            can_undo: true,
        };
        let run = RunView {
            id: 1,
            preset_name: "Bambu PLA Basic @BBL H2D".into(),
            preset_path: "/sys/Bambu PLA Basic @BBL H2D.json".into(),
            target_name: "Bambu PLA Basic (calibrated) @BBL H2D".into(),
            target_path: "/user/Bambu PLA Basic (calibrated) @BBL H2D.json".into(),
            is_system: false,
            will_save_as: None,
            filament_id: Some("GFA00".into()),
            material: "PLA".into(),
            printer_model: "H2D".into(),
            nozzle: "0.4".into(),
            slot_label: Some("A2".into()),
            variant: Some("Direct Drive Standard".into()),
            variants: vec![
                "Direct Drive Standard".into(),
                "Direct Drive High Flow".into(),
            ],
            needs_variant: false,
            preset_error: None,
            steps: vec![
                StepView {
                    step: StepKey::Temperature,
                    status: StepStatus::Saved {
                        result_id: 10,
                        value: "225 °C".into(),
                        at: 200,
                    },
                    fine_ready: false,
                    opened_at: Some(150),
                },
                StepView {
                    step: StepKey::Flow,
                    status: StepStatus::Skipped,
                    fine_ready: true,
                    opened_at: None,
                },
                StepView {
                    step: StepKey::Retraction,
                    status: StepStatus::Recorded {
                        result_id: 11,
                        value: "0.6 mm".into(),
                        at: 300,
                    },
                    fine_ready: false,
                    opened_at: Some(170),
                },
                StepView {
                    step: StepKey::MaxSpeed,
                    status: StepStatus::NotRun,
                    fine_ready: false,
                    opened_at: None,
                },
            ],
            results: vec![result],
            created_at: 100,
            updated_at: 300,
        };
        let entry = crate::journal::types::PrintEntry {
            id: 42,
            job_key: "local:cube|".into(),
            name: "Gear housing".into(),
            gcode_file: Some("gear.gcode.3mf".into()),
            plate: Some(1),
            printer_model: Some("H2D".into()),
            started_at: 1_000,
            start_approx: false,
            ended_at: Some(2_000),
            outcome: Outcome::Finished,
            error_code: None,
            hms: vec![],
            verdict: Some(Verdict::Usable),
            tags: vec!["stringing".into()],
            notes: "ok".into(),
            slice: None,
            trays: vec![TrayUse {
                ams_id: 0,
                tray_id: 1,
                label: "A2".into(),
                tray_type: "PLA".into(),
                color: "FFFFFFFF".into(),
                brand: "PLA Basic".into(),
            }],
            photos: vec![Photo {
                id: 7,
                file_name: "a.jpg".into(),
                added_at: 1_500,
            }],
            analyses: vec![3],
            purpose: Some(PrintPurpose {
                calibration_result_id: 11,
                test: TestKind::Retraction,
            }),
            updated_at: 2_000,
        };
        let fixture = json!({
            "setups": setups,
            "readings": readings,
            "runs": to(&vec![RunSummary {
                id: 1,
                preset_name: "Bambu PLA Basic @BBL H2D".into(),
                slot_label: Some("A2".into()),
                updated_at: 300,
                result_ids: vec![10, 11],
            }]),
            "run": to(&run),
            "defaults": to(&Defaults {
                setup: Setup::Temperature { start: 230, end: 190 },
                current: Some("220".into()),
                material_label: Some("PLA".into()),
            }),
            "preview_bands": to(&preview(cal::diagram(&Setup::Temperature { start: 230, end: 220 }), Some(value))),
            "preview_patches": to(&preview(cal::diagram(&Setup::FlowFine { base: 1.0 }), None)),
            "preview_ruler": to(&preview(cal::diagram(&Setup::Retraction { start: 0.0, end: 1.0, step: 0.5 }), None)),
            "preview_none": to(&preview(cal::diagram(&Setup::PressureAdvance {}), None)),
            "saved": to(&SaveAnswer::Saved {
                run: run.clone(),
                result_id: 11,
                studio_running: true,
                assign_error: Some("No printer configured".into()),
            }),
            "changed": to(&SaveAnswer::Changed { preview: preview(Diagram::None {}, None) }),
            "suggest": to(&SuggestAnswer {
                photo: "3f2a.jpg".into(),
                suggestion: Some(Suggestion {
                    pick: "225 °C".into(),
                    confidence: 0.8,
                    reason: "Smoothest band.\nSlight stringing above.".into(),
                }),
                note: None,
            }),
            "link_candidates": to(&LinkCandidates { suggested: Some(42), entries: vec![entry] }),
        });

        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/calibration/wire.json");
        if std::env::var_os("UPDATE_WIRE_FIXTURE").is_some() {
            let text = serde_json::to_string_pretty(&fixture).unwrap() + "\n";
            std::fs::write(&path, text).unwrap();
        }
        let on_disk: Value = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("src/calibration/wire.json is missing"),
        )
        .unwrap();
        assert_eq!(
            on_disk, fixture,
            "the wire changed: rerun with UPDATE_WIRE_FIXTURE=1 and update the frontend mirrors"
        );
    }
}
