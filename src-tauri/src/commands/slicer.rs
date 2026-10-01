//! Tauri commands for slicing with Bambu Studio. Nothing here prints or
//! uploads; "Open in Bambu Studio" hands the sliced file to the user.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_store::StoreExt;
use tokio::sync::Semaphore;

use crate::slicer::auto::{auto_request, wait_until_stable, STABLE_INTERVAL, STABLE_MAX};
use crate::slicer::binary::{self, SlicerStatus};
use crate::slicer::cache::{SliceCache, DEFAULT_CAP_BYTES};
use crate::slicer::jobs::{
    BambuStudioEnv, JobEvents, JobOrigin, JobRequest, JobState, JobView, SlicerEnv, SlicerService,
    WORK_DIR_NAME,
};
use crate::slicer::settings::{
    bambu_studio_selection, normalize_bed_type, BambuStudioSelection, PresetChoice, PresetLists,
    SlicerSettings, BED_TYPES, SETTINGS_KEY,
};
use crate::slicer::SlicerError;
use crate::stl_watcher::StlFile;

/// The event every job update is emitted on.
pub const JOB_EVENT: &str = "slicer://job";
/// Dropped models are staged here, one `<uuid>` folder each. Emptied at
/// launch and by "Clear slice cache"; staging a new model removes the older
/// folders no waiting or running job uses.
pub const INPUTS_DIR: &str = "slice-inputs";
/// Largest model the Slice page accepts by drag and drop.
const MAX_STAGED_BYTES: usize = 512 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;
/// How long quitting waits for a cancelled job to stop. Its process group
/// is killed at once; this only covers reaping it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
/// A staged folder younger than this is never pruned. Staging and
/// enqueueing don't share the queue's lock, so this covers a slice of an
/// older staged model that is on its way in, and the other files of a
/// multi-file drop being staged at the same time.
const STAGE_GRACE: Duration = Duration::from_secs(30 * 60);

const JOB_GONE: &str = "That slicing job is no longer available.";
const NOT_FINISHED: &str = "That job hasn't finished slicing.";
const FILES_CLEARED: &str = "That slice's files were cleared; slice it again.";
const NOT_A_MODEL: &str = "Drop an .stl or .3mf file.";
const ALREADY_SLICED: &str =
    "That file is already sliced. Drop the .stl or .3mf model it was made from.";
const TOO_LARGE: &str = "That file is too large to slice from BambuMate.";

/// Emits every job update to the webview. The service calls this with its
/// lock held, so it only emits: it never waits and never calls back into
/// the [`SlicerService`].
pub struct TauriJobEvents(pub AppHandle);

impl JobEvents for TauriJobEvents {
    fn job(&self, view: &JobView) {
        let _ = self.0.emit(JOB_EVENT, view);
    }
}

/// Saved settings, plus what jobs will actually use.
#[derive(Debug, Clone, Serialize)]
pub struct SlicerSettingsView {
    pub saved: SlicerSettings,
    pub effective: SlicerSettings,
    pub bed_types: Vec<String>,
}

/// The saved slicing settings. Reads the store: call it off the async
/// runtime.
pub fn read_settings(app: &AppHandle) -> SlicerSettings {
    app.store("preferences.json")
        .ok()
        .and_then(|s| s.get(SETTINGS_KEY))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// The defaults auto-slice and the agent use when no preset is given. Reads
/// the store and `BambuStudio.conf`: call it off the async runtime.
pub fn effective_settings(app: &AppHandle) -> SlicerSettings {
    read_settings(app).effective(&bambu_selection())
}

/// Reads `BambuStudio.conf`.
fn bambu_selection() -> BambuStudioSelection {
    crate::profile::BambuPaths::detect()
        .map(|p| bambu_studio_selection(&p.config_root))
        .unwrap_or_default()
}

/// Reads `BambuStudio.conf`: call it off the async runtime.
fn settings_view(saved: SlicerSettings) -> SlicerSettingsView {
    let effective = saved.effective(&bambu_selection());
    SlicerSettingsView {
        saved,
        effective,
        bed_types: BED_TYPES.iter().map(|s| s.to_string()).collect(),
    }
}

/// Runs blocking file work off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn slicer_status() -> SlicerStatus {
    SlicerStatus::from_detect(&binary::detect().await)
}

#[tauri::command]
pub async fn slicer_presets(printer: Option<String>) -> Result<PresetLists, String> {
    blocking(move || {
        BambuStudioEnv
            .presets()
            .map(|idx| idx.list(printer.as_deref()))
            .map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn slicer_get_settings(app: AppHandle) -> Result<SlicerSettingsView, String> {
    blocking(move || settings_view(read_settings(&app))).await
}

#[tauri::command]
pub async fn slicer_set_settings(
    app: AppHandle,
    settings: SlicerSettings,
) -> Result<SlicerSettingsView, String> {
    blocking(move || {
        let store = app.store("preferences.json").map_err(|e| e.to_string())?;
        store.set(
            SETTINGS_KEY,
            serde_json::to_value(&settings).map_err(|e| e.to_string())?,
        );
        store.save().map_err(|e| e.to_string())?;
        Ok(settings_view(settings))
    })
    .await?
}

/// Queues a slice the user asked for. The model path is checked by
/// [`SlicerService::enqueue`] (`validate_model_path`) before anything is
/// queued.
#[tauri::command]
pub async fn slicer_slice(
    svc: State<'_, SlicerService>,
    model_path: String,
    printer: String,
    process: String,
    filament: String,
    bed_type: Option<String>,
) -> Result<JobView, String> {
    let svc = svc.inner().clone();
    blocking(move || {
        svc.enqueue(JobRequest {
            source_path: model_path,
            choice: PresetChoice {
                printer,
                process,
                filaments: vec![filament],
                bed_type: normalize_bed_type(bed_type.as_deref()),
            },
            origin: JobOrigin::Manual,
        })
        .map_err(|e| e.to_string())
    })
    .await?
}

/// `true` when the cancel was taken (see [`SlicerService::cancel`]); the
/// job's final state arrives as a `slicer://job` event.
#[tauri::command]
pub fn slicer_cancel(svc: State<'_, SlicerService>, job_id: u64) -> bool {
    svc.cancel(job_id)
}

#[tauri::command]
pub fn slicer_jobs(svc: State<'_, SlicerService>) -> Vec<JobView> {
    svc.jobs()
}

/// The finished job's sliced file, which must still be on disk.
fn finished_output(view: &JobView) -> Result<PathBuf, &'static str> {
    let JobState::Done { result, .. } = &view.state else {
        return Err(NOT_FINISHED);
    };
    let path = PathBuf::from(&result.output_path);
    if result.output_path.is_empty() || !path.is_file() {
        return Err(FILES_CLEARED);
    }
    Ok(path)
}

/// Plate `plate`'s thumbnail as a `data:` URL. Only `plate_<plate>.png` next
/// to the job's own output is read, and only up to `max` bytes; `None`
/// when the plate has no thumbnail (or it is too large).
fn plate_thumbnail(view: &JobView, plate: u32, max: u64) -> Result<Option<String>, &'static str> {
    let output = finished_output(view)?;
    let JobState::Done { result, .. } = &view.state else {
        return Err(NOT_FINISHED);
    };
    let expected = format!("plate_{plate}.png");
    let listed = result
        .plates
        .iter()
        .find(|p| p.index == plate)
        .and_then(|p| p.thumbnail.as_deref());
    if listed != Some(expected.as_str()) {
        return Ok(None);
    }
    let Ok(file) = std::fs::File::open(output.with_file_name(&expected)) else {
        return Ok(None);
    };
    match file.metadata() {
        Ok(meta) if meta.is_file() && meta.len() <= max => {}
        _ => return Ok(None),
    }
    // The size is checked again while reading, in case the file grew.
    let mut bytes = Vec::new();
    if file.take(max + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > max {
        return Ok(None);
    }
    Ok(Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

/// A plate thumbnail as a `data:` URL, or `None` when the plate has none.
#[tauri::command]
pub async fn slicer_thumbnail(
    svc: State<'_, SlicerService>,
    job_id: u64,
    plate: u32,
) -> Result<Option<String>, String> {
    let view = svc.job(job_id).ok_or(JOB_GONE)?;
    blocking(move || plate_thumbnail(&view, plate, MAX_THUMBNAIL_BYTES).map_err(String::from))
        .await?
}

/// Opens the sliced file in Bambu Studio for the user to check and print.
/// BambuMate itself never prints or uploads.
#[tauri::command]
pub async fn slicer_open_in_bambu_studio(
    app: AppHandle,
    svc: State<'_, SlicerService>,
    job_id: u64,
) -> Result<crate::commands::launcher::LaunchResult, String> {
    let view = svc.job(job_id).ok_or(JOB_GONE)?;
    let output = finished_output(&view)?;
    crate::commands::launcher::launch_bambu_studio(
        app,
        Some(output.to_string_lossy().into_owned()),
        None,
    )
    .await
}

fn inputs_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(INPUTS_DIR))
}

/// Removes the staged models; returns the bytes freed.
fn remove_inputs(dir: &Path) -> u64 {
    let size = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum::<u64>();
    match std::fs::remove_dir_all(dir) {
        Ok(()) => size,
        Err(_) => 0,
    }
}

/// Empties the cache and the staged models. Returns bytes freed. Refused
/// while a job is running or waiting.
#[tauri::command]
pub async fn slicer_clear_cache(
    app: AppHandle,
    svc: State<'_, SlicerService>,
) -> Result<u64, String> {
    let svc = svc.inner().clone();
    let inputs = inputs_dir(&app).ok();
    blocking(move || {
        svc.clear_cache(|| inputs.as_deref().map(remove_inputs).unwrap_or(0))
            .map_err(|e| e.to_string())
    })
    .await?
}

/// Native file picker limited to STL and 3MF.
#[tauri::command]
pub async fn slicer_pick_model(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a model to slice")
        .add_filter("3D models", &["stl", "3mf"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let path = rx
        .await
        .map_err(|_| "Dialog closed unexpectedly".to_string())?;
    Ok(path.map(|p| p.to_string()))
}

/// A safe file name for a dropped model: its last path component, which
/// must end in `.stl` or `.3mf` (not a sliced `.gcode.3mf`) and not start
/// with a dot.
pub fn staged_name(file_name: &str) -> Option<String> {
    // Split on both separators so this is the same on every platform.
    let base = file_name
        .trim()
        .rsplit(['/', '\\'])
        .next()?
        .replace([':', '\0'], "_");
    let lower = base.to_ascii_lowercase();
    if base.starts_with('.')
        || lower.ends_with(".gcode.3mf")
        || !(lower.ends_with(".stl") || lower.ends_with(".3mf"))
    {
        return None;
    }
    Some(base)
}

/// Decodes a dropped file, refusing more than `max` bytes before and after
/// decoding.
fn decode_model(data_base64: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let data = data_base64.trim();
    // Every 4 characters hold 3 bytes, less up to 2 of padding.
    if (data.len() / 4 * 3).saturating_sub(2) > max {
        return Err(TOO_LARGE);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "That file couldn't be read.")?;
    if bytes.len() > max {
        return Err(TOO_LARGE);
    }
    Ok(bytes)
}

/// Writes a dropped model to `<inputs>/<uuid>/<name>` and returns its path.
/// `name` must already be a bare model file name ([`staged_name`]); a new
/// folder per file means nothing is ever overwritten.
fn stage_bytes(inputs: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if staged_name(name).as_deref() != Some(name) {
        return Err(NOT_A_MODEL.into());
    }
    let dir = inputs.join(uuid::Uuid::new_v4().to_string());
    let path = dir.join(name);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        f.write_all(bytes)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_dir_all(&dir);
        format!("BambuMate couldn't save the dropped file: {e}")
    })?;
    Ok(path)
}

/// The `<uuid>` folder name when `source` is a staged model
/// (`<inputs>/<uuid>/…`); `None` for any other path.
fn staged_folder(inputs: &Path, source: &str) -> Option<String> {
    let rest = Path::new(source).strip_prefix(inputs).ok()?;
    let std::path::Component::Normal(name) = rest.components().next()? else {
        return None;
    };
    let name = name.to_str()?;
    let id = uuid::Uuid::parse_str(name).ok()?;
    (id.hyphenated().to_string() == name).then(|| name.to_string())
}

/// When a staged folder last changed: its own mtime or its files', whichever
/// is newer.
fn last_modified(dir: &Path) -> Option<std::time::SystemTime> {
    let own = std::fs::symlink_metadata(dir)
        .and_then(|m| m.modified())
        .ok();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
        .chain(own)
        .max()
}

/// Removes the staged `<uuid>` folders directly in `inputs` that aren't in
/// `keep` and haven't changed for `grace`. Anything else there (other
/// names, files, symlinks) is left alone. Returns the bytes freed.
fn prune_staged(inputs: &Path, keep: &[String], grace: Duration) -> u64 {
    let Ok(read) = std::fs::read_dir(inputs) else {
        return 0;
    };
    let mut freed = 0;
    for e in read.flatten() {
        let path = e.path();
        let Some(name) = staged_folder(inputs, &path.to_string_lossy()) else {
            continue;
        };
        if keep.contains(&name) {
            continue;
        }
        // Not following a symlink: only a real folder is removed.
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            _ => continue,
        }
        // A folder from the future (clock change) counts as young.
        let old_enough = last_modified(&path)
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= grace);
        if !old_enough {
            continue;
        }
        let size = remove_inputs(&path);
        freed += size;
    }
    freed
}

/// Stages a model ([`stage_bytes`]), then prunes older staged folders except
/// those `in_use` (the model paths of waiting and running jobs) and those
/// younger than `grace`.
fn stage_and_prune(
    inputs: &Path,
    name: &str,
    bytes: &[u8],
    in_use: &[String],
    grace: Duration,
) -> Result<PathBuf, String> {
    let path = stage_bytes(inputs, name, bytes)?;
    let new = path.to_string_lossy();
    let keep: Vec<String> = in_use
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(new.as_ref()))
        .filter_map(|p| staged_folder(inputs, p))
        .collect();
    prune_staged(inputs, &keep, grace);
    Ok(path)
}

/// Saves a model dropped on the Slice page (the webview gives bytes, not a
/// path) and returns its path.
///
/// The page keeps one staged model at a time and may slice it again (other
/// presets, "Slice and compare"), so a staged model is kept until the next
/// one is staged; then older ones that no waiting or running job uses, and
/// that are older than [`STAGE_GRACE`], are removed.
#[tauri::command]
pub async fn slicer_stage_model(
    app: AppHandle,
    svc: State<'_, SlicerService>,
    file_name: String,
    data_base64: String,
) -> Result<String, String> {
    let name = staged_name(&file_name).ok_or_else(|| {
        if file_name
            .trim()
            .to_ascii_lowercase()
            .ends_with(".gcode.3mf")
        {
            ALREADY_SLICED
        } else {
            NOT_A_MODEL
        }
    })?;
    let inputs = inputs_dir(&app)?;
    let svc = svc.inner().clone();
    blocking(move || {
        let bytes = decode_model(&data_base64, MAX_STAGED_BYTES)?;
        drop(data_base64);
        let in_use: Vec<String> = svc
            .jobs()
            .into_iter()
            .filter(|j| !j.state.is_terminal())
            .map(|j| j.source_path)
            .collect();
        stage_and_prune(&inputs, &name, &bytes, &in_use, STAGE_GRACE)
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await?
}

/// Builds the queue, starts its worker and manages it. Called once from
/// `setup`.
pub fn start(app: &AppHandle) {
    let app_data = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bambumate"));
    let _ = std::fs::remove_dir_all(app_data.join(INPUTS_DIR));
    let (svc, worker) = SlicerService::new(
        Arc::new(BambuStudioEnv),
        Arc::new(TauriJobEvents(app.clone())),
        SliceCache::new(app_data.join("slices"), DEFAULT_CAP_BYTES),
        std::env::temp_dir().join(WORK_DIR_NAME),
        crate::slicer::TIMEOUT,
    );
    tauri::async_runtime::spawn(worker);
    app.manage(svc);
}

/// On app exit: cancels every job and waits (briefly) for the running one
/// to end, so no Bambu Studio process outlives BambuMate. Called from the
/// main thread, outside the async runtime.
pub fn stop(app: &AppHandle) {
    let Some(svc) = app.try_state::<SlicerService>() else {
        return;
    };
    let svc = svc.inner().clone();
    if let Some(id) = svc.shutdown() {
        let last = tauri::async_runtime::block_on(async move { svc.wait(id, SHUTDOWN_WAIT).await });
        if !last.is_some_and(|v| v.state.is_terminal()) {
            tracing::warn!(
                "slicing job {id} hadn't stopped {}s after quitting; exiting anyway",
                SHUTDOWN_WAIT.as_secs()
            );
        }
    }
}

/// How many new STLs are waited on (and queued) at once. Others wait for a
/// turn, so a bulk copy into the watch folder can't pile up polling tasks.
const AUTO_SLICE_CONCURRENCY: usize = 4;

/// Queues one new STL from the watch folder, when "Slice new STLs
/// automatically" is on. Every way out is quiet: nothing here retries, and
/// the wait for the file to settle is bounded by [`STABLE_MAX`].
async fn auto_slice(app: AppHandle, file: StlFile, turns: Arc<Semaphore>) {
    let name = file.filename.as_str();
    let full = |app: &AppHandle| {
        app.try_state::<SlicerService>()
            .is_none_or(|svc| svc.queue_is_full(JobOrigin::Auto))
    };
    // Skip at once when the watch folder's queue is full anyway.
    if full(&app) {
        tracing::debug!("auto-slice skipped {name}: the watch-folder queue is full");
        return;
    }
    let Ok(_turn) = turns.acquire().await else {
        return;
    };
    // Cheap first: the saved flag. `BambuStudio.conf` is read only when on.
    let settings = {
        let app = app.clone();
        blocking(move || {
            read_settings(&app)
                .auto_slice
                .then(|| effective_settings(&app))
        })
        .await
    };
    let request = match settings {
        Ok(Some(settings)) => match auto_request(&settings, &file.path) {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                tracing::warn!("auto-slice skipped {name}: {e}");
                return;
            }
            None => return,
        },
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("auto-slice skipped {name}: {e}");
            return;
        }
    };
    if !wait_until_stable(Path::new(&file.path), STABLE_INTERVAL, STABLE_MAX).await {
        if Path::new(&file.path).exists() {
            tracing::warn!("auto-slice skipped {name}: the file never settled");
        } else {
            tracing::debug!("auto-slice dropped {name}: it was removed or renamed");
        }
        return;
    }
    // The wait can take a minute: honour a switch turned off meanwhile.
    let still_on = {
        let app = app.clone();
        blocking(move || read_settings(&app).auto_slice).await
    };
    if !still_on.unwrap_or(false) {
        return;
    }
    // Absent only while the app is starting or has already torn down.
    let Some(svc) = app.try_state::<SlicerService>() else {
        return;
    };
    match svc.enqueue(request) {
        Ok(_) => {}
        Err(SlicerError::Closing) => {
            tracing::debug!("auto-slice of {name} not queued: BambuMate is closing");
        }
        // Includes the cap on waiting automatic jobs: the file is left alone.
        Err(e) => tracing::warn!("auto-slice of {name} not queued: {e}"),
    }
}

/// Queues each new STL from the watch folder when "Slice new STLs
/// automatically" is on. Called once from `setup`, after [`start`].
pub fn install_auto_slice(app: &AppHandle) {
    let handle = app.clone();
    let turns = Arc::new(Semaphore::new(AUTO_SLICE_CONCURRENCY));
    let hook: crate::stl_watcher::NewStlHook = Arc::new(move |file| {
        tauri::async_runtime::spawn(auto_slice(handle.clone(), file, turns.clone()));
    });
    app.state::<crate::stl_watcher::StlWatcherState>()
        .set_on_new(hook);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::command::OUTPUT_FILE;
    use crate::slicer::result::{parse_output, tests::fixture};

    #[test]
    fn staged_names_keep_only_the_file_name_of_models() {
        assert_eq!(staged_name("cube.STL").as_deref(), Some("cube.STL"));
        assert_eq!(
            staged_name("../../etc/plate.3mf").as_deref(),
            Some("plate.3mf")
        );
        assert_eq!(staged_name("C:\\x\\..\\a.stl").as_deref(), Some("a.stl"));
        assert_eq!(staged_name("odd:name.stl").as_deref(), Some("odd_name.stl"));
        assert_eq!(staged_name("notes.txt"), None);
        assert_eq!(staged_name(".stl"), None);
        assert_eq!(staged_name(""), None);
    }

    #[test]
    fn staged_names_refuse_sliced_files_and_dot_names() {
        assert_eq!(staged_name("cube.gcode.3mf"), None);
        assert_eq!(staged_name("dir/Cube.GCODE.3MF"), None);
        assert_eq!(staged_name("a/.."), None);
        assert_eq!(staged_name("a/"), None);
        assert_eq!(staged_name("..stl"), None);
    }

    #[test]
    fn staged_models_get_their_own_folder_and_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let a = stage_bytes(dir.path(), "cube.stl", b"solid a").unwrap();
        let b = stage_bytes(dir.path(), "cube.stl", b"solid b").unwrap();
        assert_ne!(a, b);
        for (p, body) in [(&a, b"solid a"), (&b, b"solid b")] {
            assert_eq!(p.file_name().unwrap(), "cube.stl");
            assert_eq!(p.parent().unwrap().parent().unwrap(), dir.path());
            assert_eq!(std::fs::read(p).unwrap(), body);
        }
        // A name that isn't a bare model file name is refused outright.
        assert!(stage_bytes(dir.path(), "../cube.stl", b"x").is_err());
        assert!(stage_bytes(dir.path(), "notes.txt", b"x").is_err());
    }

    #[test]
    fn staged_data_is_size_capped_before_and_after_decoding() {
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        assert_eq!(decode_model(&b64(b"solid"), 16).unwrap(), b"solid");
        assert_eq!(decode_model(&b64(&[7u8; 17]), 16).unwrap_err(), TOO_LARGE);
        assert!(decode_model("%%% not base64", 16).is_err());
    }

    #[test]
    fn oversized_data_is_refused_before_it_is_decoded() {
        assert_eq!(decode_model(&"A".repeat(400), 16).unwrap_err(), TOO_LARGE);
        // Not even valid base64: only the length was looked at.
        assert_eq!(decode_model(&"%".repeat(400), 16).unwrap_err(), TOO_LARGE);
    }

    #[test]
    fn staged_folders_are_recognised_only_directly_under_the_inputs_folder() {
        let inputs = Path::new("/data/slice-inputs");
        let id = uuid::Uuid::new_v4().to_string();
        let staged = format!("/data/slice-inputs/{id}/cube.stl");
        assert_eq!(staged_folder(inputs, &staged), Some(id.clone()));
        assert_eq!(
            staged_folder(inputs, "/data/slice-inputs/not-a-uuid/c.stl"),
            None
        );
        assert_eq!(
            staged_folder(inputs, &format!("/elsewhere/{id}/c.stl")),
            None
        );
        assert_eq!(staged_folder(inputs, "/data/slice-inputs"), None);
        assert_eq!(staged_folder(inputs, "relative/cube.stl"), None);
    }

    #[test]
    fn pruning_removes_only_unused_staged_folders() {
        let root = tempfile::tempdir().unwrap();
        let inputs = root.path().join(INPUTS_DIR);
        let old = stage_bytes(&inputs, "old.stl", b"old").unwrap();
        let queued = stage_bytes(&inputs, "queued.stl", b"queued").unwrap();
        let new = stage_bytes(&inputs, "new.stl", b"new").unwrap();
        // Things that aren't staged folders are never touched.
        let other = inputs.join("not-a-uuid");
        std::fs::create_dir_all(&other).unwrap();
        let loose = inputs.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&loose, b"a file, not a folder").unwrap();
        #[cfg(unix)]
        let outside = {
            let outside = root.path().join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("keep.stl"), b"x").unwrap();
            std::os::unix::fs::symlink(&outside, inputs.join(uuid::Uuid::new_v4().to_string()))
                .unwrap();
            outside
        };
        let folder = |p: &Path| staged_folder(&inputs, &p.to_string_lossy()).unwrap();
        let keep = [folder(&queued), folder(&new)];
        let freed = prune_staged(&inputs, &keep, Duration::ZERO);
        assert_eq!(freed, 3);
        assert!(!old.exists() && !old.parent().unwrap().exists());
        assert!(queued.is_file() && new.is_file());
        assert!(other.is_dir() && loose.is_file());
        #[cfg(unix)]
        assert!(outside.join("keep.stl").is_file());
        // A missing inputs folder is fine.
        assert_eq!(
            prune_staged(&root.path().join("nope"), &keep, Duration::ZERO),
            0
        );
    }

    #[test]
    fn recently_staged_folders_survive_a_prune() {
        let root = tempfile::tempdir().unwrap();
        let inputs = root.path().join(INPUTS_DIR);
        let young = stage_bytes(&inputs, "young.stl", b"young").unwrap();
        // Not kept, not in use, but staged moments ago.
        assert_eq!(prune_staged(&inputs, &[], STAGE_GRACE), 0);
        assert!(young.is_file());
        // A grace shorter than its age lets it go.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(prune_staged(&inputs, &[], Duration::from_millis(10)), 5);
        assert!(!young.exists());
    }

    #[test]
    fn back_to_back_stages_both_keep_their_files() {
        let root = tempfile::tempdir().unwrap();
        let inputs = root.path().join(INPUTS_DIR);
        // A multi-file drop: the second stage must not prune the first.
        let a = stage_and_prune(&inputs, "a.stl", b"a", &[], STAGE_GRACE).unwrap();
        let b = stage_and_prune(&inputs, "b.3mf", b"b", &[], STAGE_GRACE).unwrap();
        assert!(a.is_file() && b.is_file());
        // Without the grace period the older one would go, unless a job
        // still uses it.
        let c = stage_and_prune(
            &inputs,
            "c.stl",
            b"c",
            &[a.to_string_lossy().into_owned()],
            Duration::ZERO,
        )
        .unwrap();
        assert!(a.is_file() && !b.exists() && c.is_file());
    }

    /// A finished job whose result lives in `dir` (output and plate_1.png).
    fn done_job(dir: &Path) -> JobView {
        let out = dir.join(OUTPUT_FILE);
        std::fs::copy(fixture("cube_h2c.gcode.3mf"), &out).unwrap();
        let mut result = parse_output(&out, None, Some(dir)).unwrap();
        result.output_path = out.to_string_lossy().into_owned();
        JobView {
            id: 7,
            origin: JobOrigin::Manual,
            source_path: "/m/cube.stl".into(),
            model_name: "cube.stl".into(),
            printer: "p".into(),
            process: "q".into(),
            filament: "f".into(),
            bed_type: "Textured PEI Plate".into(),
            state: JobState::Done {
                result,
                cached: false,
            },
        }
    }

    #[test]
    fn thumbnails_come_only_from_the_jobs_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let job = done_job(dir.path());
        let url = plate_thumbnail(&job, 1, MAX_THUMBNAIL_BYTES)
            .unwrap()
            .unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        // No such plate.
        assert_eq!(plate_thumbnail(&job, 9, MAX_THUMBNAIL_BYTES).unwrap(), None);
        // Too large.
        assert_eq!(plate_thumbnail(&job, 1, 8).unwrap(), None);
        // A result naming another file is not followed.
        let mut odd = job.clone();
        if let JobState::Done { result, .. } = &mut odd.state {
            result.plates[0].thumbnail = Some("../secret.png".into());
        }
        assert_eq!(plate_thumbnail(&odd, 1, MAX_THUMBNAIL_BYTES).unwrap(), None);
    }

    #[test]
    fn cleared_or_unfinished_jobs_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let job = done_job(dir.path());
        assert_eq!(finished_output(&job).unwrap(), dir.path().join(OUTPUT_FILE));
        std::fs::remove_file(dir.path().join(OUTPUT_FILE)).unwrap();
        assert_eq!(finished_output(&job).unwrap_err(), FILES_CLEARED);
        assert_eq!(
            plate_thumbnail(&job, 1, MAX_THUMBNAIL_BYTES).unwrap_err(),
            FILES_CLEARED
        );
        let mut queued = job.clone();
        queued.state = JobState::Queued { position: 0 };
        assert_eq!(finished_output(&queued).unwrap_err(), NOT_FINISHED);
    }
}
