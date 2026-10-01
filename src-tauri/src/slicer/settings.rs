//! Finds Bambu Studio's presets and writes the fully flattened config files
//! its command line needs.
//!
//! The CLI does not resolve `inherits` or `include` itself: given a stub it
//! silently slices with built-in defaults (a filament density of 0, so 0 g).
//! Every config handed to it is flattened here with
//! [`resolve_with_includes`], which mirrors Bambu Studio's own loader.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use walkdir::WalkDir;

use super::SlicerError;
use crate::profile::inheritance::{resolve_with_includes, ResolveError, MAX_INHERITANCE_DEPTH};
use crate::profile::types::FilamentProfile;
use crate::profile::{reader, ProfileRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresetKind {
    Machine,
    Process,
    Filament,
}

impl PresetKind {
    pub const ALL: [PresetKind; 3] = [
        PresetKind::Machine,
        PresetKind::Process,
        PresetKind::Filament,
    ];

    /// Folder name under `system/BBL/` and `user/<id>/`, and the JSON `type`.
    pub fn as_str(self) -> &'static str {
        match self {
            PresetKind::Machine => "machine",
            PresetKind::Process => "process",
            PresetKind::Filament => "filament",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetSource {
    User,
    System,
}

/// One entry in a preset picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetOption {
    pub name: String,
    pub source: PresetSource,
}

/// The three pickers on the Slice page, user presets first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetLists {
    pub printers: Vec<PresetOption>,
    pub processes: Vec<PresetOption>,
    pub filaments: Vec<PresetOption>,
}

/// The plate types Bambu Studio offers (`curr_bed_type`).
pub const BED_TYPES: [&str; 5] = [
    "Cool Plate",
    "Engineering Plate",
    "High Temp Plate",
    "Textured PEI Plate",
    "Supertack Plate",
];
pub const DEFAULT_BED_TYPE: &str = "Textured PEI Plate";

/// A valid plate type, falling back to [`DEFAULT_BED_TYPE`].
pub fn normalize_bed_type(raw: Option<&str>) -> String {
    raw.and_then(|r| BED_TYPES.iter().find(|b| b.eq_ignore_ascii_case(r.trim())))
        .unwrap_or(&DEFAULT_BED_TYPE)
        .to_string()
}

struct KindIndex {
    registry: ProfileRegistry,
    sources: HashMap<String, PresetSource>,
    selectable: HashSet<String>,
}

/// Every machine, process and filament preset Bambu Studio has: system
/// presets from `system/BBL/<kind>` and the user's from `user/<id>/<kind>`.
pub struct PresetIndex {
    kinds: HashMap<PresetKind, KindIndex>,
}

/// A flattened preset, ready to be written for the CLI.
#[derive(Debug, Clone, PartialEq)]
pub struct Flattened {
    pub config: Map<String, Value>,
    pub source: PresetSource,
    /// The system preset this one is, or descends from. For a user preset
    /// with no system ancestor, its own name.
    pub system_name: String,
}

/// What to slice with, by preset name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetChoice {
    pub printer: String,
    pub process: String,
    pub filaments: Vec<String>,
    pub bed_type: String,
}

/// The config files' contents, in CLI order. The cache key hashes these
/// exact bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedConfigs {
    pub machine: String,
    pub process: String,
    pub filaments: Vec<String>,
}

/// Where [`write_configs`] put the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub machine: PathBuf,
    pub process: PathBuf,
    pub filaments: Vec<PathBuf>,
}

impl PresetIndex {
    /// Indexes `config_root` (Bambu Studio's data folder). User presets come
    /// from `user/<preset_folder>` when it is known, else from every folder
    /// under `user/`. Unreadable files are skipped, and so is a user preset
    /// named like a system preset (Bambu Studio refuses those too).
    ///
    /// Blocking: it reads every preset file, so call it from
    /// `spawn_blocking`, and cache the index rather than reloading per job.
    pub fn load(config_root: &Path, preset_folder: Option<&str>) -> Self {
        let user_folders = user_folders(config_root, preset_folder);
        let mut kinds = HashMap::new();
        for kind in PresetKind::ALL {
            let mut idx = KindIndex {
                registry: ProfileRegistry::new(),
                sources: HashMap::new(),
                selectable: HashSet::new(),
            };
            let system_dir = config_root.join("system").join("BBL").join(kind.as_str());
            add_dir(&mut idx, &system_dir, PresetSource::System, kind);
            for folder in &user_folders {
                add_dir(
                    &mut idx,
                    &folder.join(kind.as_str()),
                    PresetSource::User,
                    kind,
                );
            }
            kinds.insert(kind, idx);
        }
        Self { kinds }
    }

    fn kind(&self, kind: PresetKind) -> &KindIndex {
        &self.kinds[&kind]
    }

    pub fn contains(&self, kind: PresetKind, name: &str) -> bool {
        self.kind(kind).registry.get_by_name(name).is_some()
    }

    /// Flattens one preset by name.
    pub fn flatten(&self, kind: PresetKind, name: &str) -> Result<Flattened, SlicerError> {
        let idx = self.kind(kind);
        let profile = idx
            .registry
            .get_by_name(name)
            .ok_or_else(|| SlicerError::UnknownPreset {
                name: name.to_string(),
            })?;
        let resolved = resolve_with_includes(profile, &idx.registry).map_err(|e| match e {
            ResolveError::MissingParent(parent) => SlicerError::MissingParent {
                name: name.to_string(),
                parent,
            },
            ResolveError::Circular(_) | ResolveError::TooDeep(_) => {
                SlicerError::io(format!("preset '{name}' has a broken inheritance chain"))
            }
        })?;
        let source = idx.sources.get(name).copied().unwrap_or(PresetSource::User);
        let system_name = std::iter::once(name.to_string())
            .chain(resolved.ancestors.iter().cloned())
            .find(|n| idx.sources.get(n) == Some(&PresetSource::System))
            .unwrap_or_else(|| name.to_string());
        Ok(Flattened {
            config: resolved.config,
            source,
            system_name,
        })
    }

    /// `compatible_printers` of a preset, from the nearest level that sets a
    /// non-empty list. Empty means "every printer".
    fn compatible_printers(&self, kind: PresetKind, name: &str) -> Vec<String> {
        let reg = &self.kind(kind).registry;
        let mut current = reg.get_by_name(name);
        let mut depth = 0;
        while let Some(p) = current {
            if let Some(list) = p.get_string_array("compatible_printers") {
                if !list.is_empty() {
                    return list.into_iter().map(str::to_string).collect();
                }
            }
            depth += 1;
            if depth >= MAX_INHERITANCE_DEPTH {
                break;
            }
            current = p
                .inherits()
                .filter(|i| !i.is_empty())
                .and_then(|i| reg.get_by_name(i));
        }
        Vec::new()
    }

    /// The pickers. With a printer, processes and filaments are limited to
    /// presets compatible with it.
    pub fn list(&self, printer: Option<&str>) -> PresetLists {
        let printer_names: Option<Vec<String>> = printer
            .filter(|p| self.contains(PresetKind::Machine, p))
            .map(|p| {
                let system = self.flatten(PresetKind::Machine, p).ok();
                printer_aliases(p, system.as_ref().map(|f| f.system_name.as_str()))
            });
        let options = |kind: PresetKind| -> Vec<PresetOption> {
            let idx = self.kind(kind);
            let mut out: Vec<PresetOption> = idx
                .selectable
                .iter()
                .filter(|name| match (&printer_names, kind) {
                    (Some(printers), PresetKind::Process | PresetKind::Filament) => {
                        made_for(&self.compatible_printers(kind, name), printers)
                    }
                    _ => true,
                })
                .map(|name| PresetOption {
                    name: name.clone(),
                    source: idx.sources[name],
                })
                .collect();
            out.sort_by(|a, b| {
                (
                    a.source != PresetSource::User,
                    a.name.to_lowercase(),
                    &a.name,
                )
                    .cmp(&(
                        b.source != PresetSource::User,
                        b.name.to_lowercase(),
                        &b.name,
                    ))
            });
            out
        };
        PresetLists {
            printers: options(PresetKind::Machine),
            processes: options(PresetKind::Process),
            filaments: options(PresetKind::Filament),
        }
    }

    /// Flattens the chosen presets into the exact text of the config files.
    ///
    /// The process must be made for the printer: its `compatible_printers`
    /// (from the nearest level that sets a non-empty one, as the pickers
    /// use) is empty or names the printer
    /// or the printer's system preset. Otherwise this is
    /// [`SlicerError::IncompatibleProcess`].
    pub fn prepare(&self, choice: &PresetChoice) -> Result<PreparedConfigs, SlicerError> {
        if choice.filaments.is_empty() {
            return Err(SlicerError::io(not_chosen("filament")));
        }
        let machine = self.flatten(PresetKind::Machine, &choice.printer)?;
        let printer_system = machine.system_name.clone();
        let mut process = self.flatten(PresetKind::Process, &choice.process)?;
        let printers = printer_aliases(&choice.printer, Some(&printer_system));
        if !made_for(
            &self.compatible_printers(PresetKind::Process, &choice.process),
            &printers,
        ) {
            return Err(SlicerError::IncompatibleProcess {
                process: choice.process.clone(),
                printer: choice.printer.clone(),
            });
        }
        process.config.insert(
            "curr_bed_type".into(),
            Value::String(normalize_bed_type(Some(&choice.bed_type))),
        );
        // The CLI refuses a process whose own compatible_printers lacks the
        // printer's system name, even when the list is empty or only
        // inherited. The check above established they go together, so make
        // sure the flattened list says so.
        let mut compat: Vec<Value> = process
            .config
            .get("compatible_printers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !compat
            .iter()
            .any(|v| v.as_str() == Some(printer_system.as_str()))
        {
            compat.push(Value::String(printer_system.clone()));
        }
        process
            .config
            .insert("compatible_printers".into(), Value::Array(compat));
        let filaments = choice
            .filaments
            .iter()
            .map(|name| {
                self.flatten(PresetKind::Filament, name)
                    .map(|f| cli_text(PresetKind::Filament, name, f))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PreparedConfigs {
            machine: cli_text(PresetKind::Machine, &choice.printer, machine),
            process: cli_text(PresetKind::Process, &choice.process, process),
            filaments,
        })
    }
}

/// The names a preset's `compatible_printers` may use for a printer: its own
/// and, for a user printer, its system preset's.
fn printer_aliases(printer: &str, system_name: Option<&str>) -> Vec<String> {
    let mut names = vec![printer.to_string()];
    if let Some(system) = system_name.filter(|s| *s != printer) {
        names.push(system.to_string());
    }
    names
}

/// Whether a `compatible_printers` list admits one of `printers`. Empty
/// means "every printer".
fn made_for(compatible: &[String], printers: &[String]) -> bool {
    compatible.is_empty() || compatible.iter().any(|c| printers.contains(c))
}

/// The user preset folders [`PresetIndex::load`] reads: `user/<preset_folder>`
/// when it exists, else every folder under `user/`.
fn user_folders(config_root: &Path, preset_folder: Option<&str>) -> Vec<PathBuf> {
    let user_root = config_root.join("user");
    match preset_folder {
        Some(f) if user_root.join(f).is_dir() => vec![user_root.join(f)],
        _ => std::fs::read_dir(&user_root)
            .map(|it| {
                it.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A summary of everything [`PresetIndex::load`] would read: per folder, how
/// many entries it holds and the newest modification time among them and the
/// folder itself. Adding, removing, renaming or editing a preset changes it.
/// Only metadata is read; no preset is parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetStamp(Vec<(PathBuf, usize, Option<SystemTime>)>);

impl PresetStamp {
    /// Blocking: it walks the preset folders.
    pub fn read(config_root: &Path, preset_folder: Option<&str>) -> Self {
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        // `user/` itself, so a new user folder is noticed.
        let user_root = config_root.join("user");
        let mut out = vec![(user_root.clone(), 0, modified(&user_root))];
        let mut dirs: Vec<PathBuf> = PresetKind::ALL
            .iter()
            .map(|k| config_root.join("system").join("BBL").join(k.as_str()))
            .collect();
        for folder in user_folders(config_root, preset_folder) {
            dirs.extend(PresetKind::ALL.iter().map(|k| folder.join(k.as_str())));
        }
        for dir in dirs {
            let mut count = 0;
            let mut newest = None;
            for entry in WalkDir::new(&dir).into_iter().filter_map(Result::ok) {
                count += 1;
                let t = entry.metadata().ok().and_then(|m| m.modified().ok());
                newest = newest.max(t);
            }
            out.push((dir, count, newest));
        }
        Self(out)
    }
}

/// The last [`PresetIndex`] loaded, reused until its folders change
/// ([`PresetStamp`]). Loading parses thousands of preset files; checking
/// the stamp only reads their metadata.
#[derive(Default)]
pub struct PresetCache(Mutex<Option<CachedIndex>>);

struct CachedIndex {
    config_root: PathBuf,
    preset_folder: Option<String>,
    stamp: PresetStamp,
    index: Arc<PresetIndex>,
}

impl PresetCache {
    /// The index for `config_root`, loaded again only when something in its
    /// preset folders changed. Blocking: call it from `spawn_blocking`.
    pub fn load(&self, config_root: &Path, preset_folder: Option<&str>) -> Arc<PresetIndex> {
        // Read before loading: a change made during the load leaves the
        // stamp older than the index, so the next call loads again.
        let stamp = PresetStamp::read(config_root, preset_folder);
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = slot.as_ref() {
            if c.config_root == config_root
                && c.preset_folder.as_deref() == preset_folder
                && c.stamp == stamp
            {
                return c.index.clone();
            }
        }
        let index = Arc::new(PresetIndex::load(config_root, preset_folder));
        *slot = Some(CachedIndex {
            config_root: config_root.to_path_buf(),
            preset_folder: preset_folder.map(str::to_string),
            stamp,
            index: index.clone(),
        });
        index
    }
}

fn add_dir(idx: &mut KindIndex, dir: &Path, source: PresetSource, kind: PresetKind) {
    if !dir.is_dir() {
        return;
    }
    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".backups")
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(profile) = reader::read_profile(path) else {
            continue;
        };
        let Some(name) = profile.name().map(str::to_string) else {
            continue;
        };
        // Bambu Studio skips a user preset named like a system one
        // ("Preset already present, not loading" in
        // PresetCollection::load_presets, src/libslic3r/Preset.cpp,
        // v02.08.02.61). Letting it replace the system preset would also
        // take over every system preset inheriting from it.
        if source == PresetSource::User && idx.sources.get(&name) == Some(&PresetSource::System) {
            tracing::warn!("user preset {name:?} has a system preset's name; skipped");
            continue;
        }
        if is_selectable(&profile, source, kind) {
            idx.selectable.insert(name.clone());
        }
        idx.sources.insert(name, source);
        idx.registry.insert(profile);
    }
}

/// System presets are pickable only when `instantiation` is `"true"` (the
/// rest are bases and templates). User presets are, unless marked `"false"`.
fn is_selectable(p: &FilamentProfile, source: PresetSource, kind: PresetKind) -> bool {
    let inst = p.raw().get("instantiation").and_then(Value::as_str);
    let type_ok = p
        .raw()
        .get("type")
        .and_then(Value::as_str)
        .is_none_or(|t| t == kind.as_str());
    type_ok
        && match source {
            PresetSource::System => inst == Some("true"),
            PresetSource::User => inst != Some("false"),
        }
}

/// The CLI requires `type`, accepts `from` of `system` or `User` only, and
/// for a user preset reads `inherits` as its system preset's name.
fn cli_text(kind: PresetKind, name: &str, flat: Flattened) -> String {
    let mut config = flat.config;
    config.insert("type".into(), Value::String(kind.as_str().into()));
    config.insert("name".into(), Value::String(name.into()));
    match flat.source {
        PresetSource::System => {
            config.insert("from".into(), Value::String("system".into()));
            config.remove("inherits");
        }
        PresetSource::User => {
            config.insert("from".into(), Value::String("User".into()));
            let inherits = if flat.system_name == name {
                String::new()
            } else {
                flat.system_name
            };
            let inherits = if kind == PresetKind::Machine && inherits.is_empty() {
                name.to_string()
            } else {
                inherits
            };
            config.insert("inherits".into(), Value::String(inherits));
        }
    }
    serde_json::to_string_pretty(&Value::Object(config)).expect("JSON maps always serialize")
}

/// Writes the prepared configs into `dir` as `machine.json`,
/// `process.json` and `filament_<n>.json`.
pub fn write_configs(dir: &Path, prepared: &PreparedConfigs) -> Result<ConfigPaths, SlicerError> {
    let io = |e: std::io::Error| SlicerError::io(e.to_string());
    std::fs::create_dir_all(dir).map_err(io)?;
    let machine = dir.join("machine.json");
    let process = dir.join("process.json");
    std::fs::write(&machine, &prepared.machine).map_err(io)?;
    std::fs::write(&process, &prepared.process).map_err(io)?;
    let mut filaments = Vec::new();
    for (i, text) in prepared.filaments.iter().enumerate() {
        let path = dir.join(format!("filament_{}.json", i + 1));
        std::fs::write(&path, text).map_err(io)?;
        filaments.push(path);
    }
    Ok(ConfigPaths {
        machine,
        process,
        filaments,
    })
}

/// Bambu Studio's own current selection, used when the user hasn't chosen
/// defaults in BambuMate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BambuStudioSelection {
    pub printer: Option<String>,
    pub process: Option<String>,
    pub filament: Option<String>,
    pub bed_type: Option<String>,
}

/// The only parts of `BambuStudio.conf` ever deserialized. Every other key,
/// including the printer access codes, is skipped by the parser without
/// being stored. Leaves are lenient: a value of the wrong type reads as
/// unset instead of failing the whole file.
#[derive(Deserialize, Default)]
#[serde(default)]
struct ConfSelection {
    app: ConfApp,
    presets: ConfPresets,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ConfApp {
    curr_bed_type: Option<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ConfPresets {
    machine: Option<Value>,
    process: Option<Value>,
    filaments: Option<Value>,
}

/// Reads `presets.machine`, `presets.process`, `presets.filaments[0]` and
/// `app.curr_bed_type` from `BambuStudio.conf`. Nothing else in that file is
/// read; it also holds printer access codes.
pub fn bambu_studio_selection(config_root: &Path) -> BambuStudioSelection {
    let Ok(text) = std::fs::read_to_string(config_root.join("BambuStudio.conf")) else {
        return BambuStudioSelection::default();
    };
    let Ok(conf) =
        serde_json::from_str::<ConfSelection>(crate::profile::writer::strip_md5_checksum(&text))
    else {
        return BambuStudioSelection::default();
    };
    drop(text);
    let s = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    };
    // Bambu Studio stores the plate as the number of its `BedType` enum
    // (`app_config->set("curr_bed_type", std::to_string(int(bed_type)))` in
    // src/slic3r/GUI/Plater.cpp). The enum, in src/libslic3r/PrintConfig.hpp
    // at v02.08.02.61, is btDefault = 0, btPC (Cool Plate), btEP
    // (Engineering Plate), btPEI (High Temp Plate), btPTE (Textured PEI
    // Plate), btSuperTack (Supertack Plate): 1 to 5 in `BED_TYPES` order.
    // 0 ("Default Plate") is not a real plate and reads as unset.
    let bed_raw = match conf.app.curr_bed_type.as_ref() {
        Some(Value::Number(n)) => Some(n.to_string()),
        other => s(other),
    };
    let bed_type = bed_raw.and_then(|raw| match raw.trim() {
        "1" => Some(BED_TYPES[0].to_string()),
        "2" => Some(BED_TYPES[1].to_string()),
        "3" => Some(BED_TYPES[2].to_string()),
        "4" => Some(BED_TYPES[3].to_string()),
        "5" => Some(BED_TYPES[4].to_string()),
        other => BED_TYPES
            .iter()
            .find(|b| b.eq_ignore_ascii_case(other))
            .map(|b| b.to_string()),
    });
    BambuStudioSelection {
        printer: s(conf.presets.machine.as_ref()),
        process: s(conf.presets.process.as_ref()),
        filament: s(conf
            .presets
            .filaments
            .as_ref()
            .and_then(|f| f.as_array())
            .and_then(|f| f.first())),
        bed_type,
    }
}

/// BambuMate's slicing preferences, stored as one JSON object under
/// [`SETTINGS_KEY`] in `preferences.json`.
pub const SETTINGS_KEY: &str = "slicer";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlicerSettings {
    pub printer: Option<String>,
    pub process: Option<String>,
    pub filament: Option<String>,
    pub bed_type: Option<String>,
    /// "Slice new STLs automatically". Off by default.
    pub auto_slice: bool,
}

impl SlicerSettings {
    /// The defaults a job uses: BambuMate's saved choice, else Bambu
    /// Studio's current selection.
    pub fn effective(&self, bs: &BambuStudioSelection) -> SlicerSettings {
        SlicerSettings {
            printer: self.printer.clone().or_else(|| bs.printer.clone()),
            process: self.process.clone().or_else(|| bs.process.clone()),
            filament: self.filament.clone().or_else(|| bs.filament.clone()),
            bed_type: Some(normalize_bed_type(
                self.bed_type.as_deref().or(bs.bed_type.as_deref()),
            )),
            auto_slice: self.auto_slice,
        }
    }
}

impl SlicerSettings {
    /// The presets for one job: the overrides given, else these settings
    /// (normally [`SlicerSettings::effective`]). Names what is missing.
    pub fn choice(
        &self,
        printer: Option<String>,
        process: Option<String>,
        filament: Option<String>,
    ) -> Result<PresetChoice, String> {
        let need = |v: Option<String>, what: &str| {
            v.filter(|s| !s.trim().is_empty())
                .ok_or_else(|| not_chosen(what))
        };
        Ok(PresetChoice {
            printer: need(printer.or_else(|| self.printer.clone()), "printer")?,
            process: need(process.or_else(|| self.process.clone()), "process")?,
            filaments: vec![need(
                filament.or_else(|| self.filament.clone()),
                "filament",
            )?],
            bed_type: normalize_bed_type(self.bed_type.as_deref()),
        })
    }
}

/// The copy for a job with no preset of one kind.
fn not_chosen(what: &str) -> String {
    format!("No {what} preset chosen. Pick one on the Slice page.")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_preset_cache_reloads_only_when_a_preset_folder_changes() {
        let root = fake_bambu_root();
        let cache = PresetCache::default();
        let first = cache.load(root.path(), Some("1881310893"));
        let again = cache.load(root.path(), Some("1881310893"));
        assert!(Arc::ptr_eq(&first, &again), "nothing changed: reused");

        // A new user filament (in a nested folder, as Bambu Studio keeps
        // them) is picked up.
        let name = "My New PLA";
        assert!(!first.contains(PresetKind::Filament, name));
        let dir = root.path().join("user/1881310893/filament/base");
        std::fs::write(
            dir.join("My New PLA.json"),
            serde_json::to_string(&json!({
                "type":"filament","name":name,"from":"User","inherits":"Bambu PLA Basic @BBL H2C"
            }))
            .unwrap(),
        )
        .unwrap();
        let touched = cache.load(root.path(), Some("1881310893"));
        assert!(!Arc::ptr_eq(&first, &touched), "a touched folder reloads");
        assert!(touched.contains(PresetKind::Filament, name));
        let settled = cache.load(root.path(), Some("1881310893"));
        assert!(Arc::ptr_eq(&touched, &settled));

        // Another root or user folder is another index.
        let other = cache.load(root.path(), None);
        assert!(!Arc::ptr_eq(&settled, &other));
    }

    #[test]
    fn the_preset_stamp_sees_an_edited_preset() {
        let root = fake_bambu_root();
        let before = PresetStamp::read(root.path(), Some("1881310893"));
        assert_eq!(before, PresetStamp::read(root.path(), Some("1881310893")));
        let file = root.path().join("user/1881310893/process/My Fine.json");
        // Moved forward explicitly: some file systems keep coarse times.
        let later = std::fs::metadata(&file).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(10);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_ne!(before, PresetStamp::read(root.path(), Some("1881310893")));
    }

    fn write(root: &Path, rel: &str, v: Value) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }

    /// A miniature Bambu Studio data folder shaped like the real one
    /// (02.08.02.61): includes, @base filaments, a user folder.
    pub(crate) fn fake_bambu_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "system/BBL/machine/fdm_machine_common.json",
            json!({
                "type":"machine","name":"fdm_machine_common","from":"system","instantiation":"false",
                "machine_start_gcode":"G28 ; generic","nozzle_diameter":["0.4"]
            }),
        );
        write(
            r,
            "system/BBL/machine/Bambu Lab H2C 0.4 nozzle template machine_start_gcode.json",
            json!({
                "name":"Bambu Lab H2C 0.4 nozzle template machine_start_gcode","instantiation":"false",
                "machine_start_gcode":";===== machine: H2C"
            }),
        );
        write(
            r,
            "system/BBL/machine/Bambu Lab H2C 0.4 nozzle.json",
            json!({
                "type":"machine","name":"Bambu Lab H2C 0.4 nozzle","inherits":"fdm_machine_common",
                "from":"system","setting_id":"GM040","instantiation":"true","printer_model":"Bambu Lab H2C",
                "nozzle_diameter":["0.4","0.4"],
                "include":["Bambu Lab H2C 0.4 nozzle template machine_start_gcode"]
            }),
        );
        write(
            r,
            "system/BBL/machine/Bambu Lab H2S 0.4 nozzle.json",
            json!({
                "type":"machine","name":"Bambu Lab H2S 0.4 nozzle","inherits":"fdm_machine_common",
                "from":"system","instantiation":"true","printer_model":"Bambu Lab H2S"
            }),
        );
        write(
            r,
            "system/BBL/machine/Bambu Lab H2C.json",
            json!({
                "type":"machine_model","name":"Bambu Lab H2C","model_id":"O1C2"
            }),
        );
        write(
            r,
            "system/BBL/process/fdm_process_common.json",
            json!({
                "type":"process","name":"fdm_process_common","from":"system","instantiation":"false",
                "layer_height":"0.2","sparse_infill_density":"15%"
            }),
        );
        write(
            r,
            "system/BBL/process/0.20mm Standard @BBL H2C.json",
            json!({
                "type":"process","name":"0.20mm Standard @BBL H2C","inherits":"fdm_process_common",
                "from":"system","instantiation":"true","compatible_printers":["Bambu Lab H2C 0.4 nozzle"]
            }),
        );
        write(
            r,
            "system/BBL/process/0.20mm Standard @BBL H2S.json",
            json!({
                "type":"process","name":"0.20mm Standard @BBL H2S","inherits":"fdm_process_common",
                "from":"system","instantiation":"true","compatible_printers":["Bambu Lab H2S 0.4 nozzle"]
            }),
        );
        write(
            r,
            "system/BBL/filament/fdm_filament_template_direct_dual.json",
            json!({
                "type":"filament","name":"fdm_filament_template_direct_dual","from":"system",
                "instantiation":"false","filament_flow_ratio":["0.95","0.95"]
            }),
        );
        write(
            r,
            "system/BBL/filament/Bambu PLA Basic @base.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @base","from":"system","instantiation":"false",
                "filament_id":"GFA00","filament_type":["PLA"],"filament_density":["1.26"],
                "filament_cost":["19.99"],"filament_flow_ratio":["0.98"]
            }),
        );
        write(
            r,
            "system/BBL/filament/Bambu PLA Basic @BBL H2C.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @BBL H2C","inherits":"Bambu PLA Basic @base",
                "from":"system","setting_id":"GFSA04_10","instantiation":"true",
                "compatible_printers":["Bambu Lab H2C 0.4 nozzle"],
                "include":["fdm_filament_template_direct_dual"]
            }),
        );
        write(
            r,
            "system/BBL/filament/Bambu PLA Basic @BBL H2S.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @BBL H2S","inherits":"Bambu PLA Basic @base",
                "from":"system","instantiation":"true","compatible_printers":["Bambu Lab H2S 0.4 nozzle"]
            }),
        );
        // A BambuMate-generated user filament: flattened, no `type`.
        write(
            r,
            "user/1881310893/filament/base/SUNLU PETG @Bambu Lab H2C 0.4 nozzle.json",
            json!({
                "name":"SUNLU PETG @Bambu Lab H2C 0.4 nozzle","inherits":"","from":"User",
                "filament_id":"HSPETG04","filament_type":["PETG"],"filament_cost":["12.99"],
                "compatible_printers":["Bambu Lab H2C 0.4 nozzle"]
            }),
        );
        // A Bambu Studio user process with an empty compatible_printers.
        write(
            r,
            "user/1881310893/process/My Fine.json",
            json!({
                "type":"process","name":"My Fine","inherits":"0.20mm Standard @BBL H2C","from":"User",
                "layer_height":"0.12","compatible_printers":[]
            }),
        );
        write(
            r,
            "user/1881310893/filament/base/.backups/old.json",
            json!({
                "name":"Stale backup","filament_type":["PLA"]
            }),
        );
        write(
            r,
            "BambuStudio.conf",
            json!({
                "app":{"curr_bed_type":"4"},
                "presets":{"machine":"Bambu Lab H2C 0.4 nozzle","process":"0.20mm Standard @BBL H2C",
                           "filaments":["Bambu PLA Basic @BBL H2C"]},
                "access_code":{"SERIAL":"12345678"}
            }),
        );
        dir
    }

    fn names(opts: &[PresetOption]) -> Vec<&str> {
        opts.iter().map(|o| o.name.as_str()).collect()
    }

    #[test]
    fn lists_only_selectable_presets_user_first() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let lists = idx.list(None);
        assert_eq!(
            names(&lists.printers),
            vec!["Bambu Lab H2C 0.4 nozzle", "Bambu Lab H2S 0.4 nozzle"]
        );
        assert_eq!(
            names(&lists.processes),
            vec![
                "My Fine",
                "0.20mm Standard @BBL H2C",
                "0.20mm Standard @BBL H2S"
            ]
        );
        assert_eq!(lists.processes[0].source, PresetSource::User);
        assert_eq!(
            names(&lists.filaments),
            vec![
                "SUNLU PETG @Bambu Lab H2C 0.4 nozzle",
                "Bambu PLA Basic @BBL H2C",
                "Bambu PLA Basic @BBL H2S"
            ]
        );
    }

    #[test]
    fn listing_for_a_printer_keeps_compatible_presets() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let lists = idx.list(Some("Bambu Lab H2C 0.4 nozzle"));
        assert_eq!(
            names(&lists.processes),
            vec!["My Fine", "0.20mm Standard @BBL H2C"],
            "My Fine inherits the H2C process's compatible_printers"
        );
        assert_eq!(
            names(&lists.filaments),
            vec![
                "SUNLU PETG @Bambu Lab H2C 0.4 nozzle",
                "Bambu PLA Basic @BBL H2C"
            ]
        );
    }

    #[test]
    fn system_presets_flatten_with_includes_and_cli_metadata() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), None);
        let prepared = idx
            .prepare(&PresetChoice {
                printer: "Bambu Lab H2C 0.4 nozzle".into(),
                process: "0.20mm Standard @BBL H2C".into(),
                filaments: vec!["Bambu PLA Basic @BBL H2C".into()],
                bed_type: "High Temp Plate".into(),
            })
            .unwrap();
        let m: Value = serde_json::from_str(&prepared.machine).unwrap();
        assert_eq!(m["type"], "machine");
        assert_eq!(m["from"], "system");
        assert_eq!(m["machine_start_gcode"], ";===== machine: H2C");
        assert_eq!(m["nozzle_diameter"], json!(["0.4", "0.4"]));
        assert!(m.get("inherits").is_none() && m.get("include").is_none());
        let p: Value = serde_json::from_str(&prepared.process).unwrap();
        assert_eq!(p["layer_height"], "0.2");
        assert_eq!(p["curr_bed_type"], "High Temp Plate");
        assert_eq!(
            p["compatible_printers"],
            json!(["Bambu Lab H2C 0.4 nozzle"])
        );
        let f: Value = serde_json::from_str(&prepared.filaments[0]).unwrap();
        assert_eq!(f["type"], "filament");
        assert_eq!(f["filament_density"], json!(["1.26"]));
        assert_eq!(f["filament_id"], "GFA00");
        assert_eq!(
            f["filament_flow_ratio"],
            json!(["0.95", "0.95"]),
            "include beats @base"
        );
    }

    #[test]
    fn user_presets_get_type_from_and_their_system_ancestor() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let prepared = idx
            .prepare(&PresetChoice {
                printer: "Bambu Lab H2C 0.4 nozzle".into(),
                process: "My Fine".into(),
                filaments: vec!["SUNLU PETG @Bambu Lab H2C 0.4 nozzle".into()],
                bed_type: "nonsense".into(),
            })
            .unwrap();
        let p: Value = serde_json::from_str(&prepared.process).unwrap();
        assert_eq!(p["from"], "User");
        assert_eq!(p["inherits"], "0.20mm Standard @BBL H2C");
        assert_eq!(p["layer_height"], "0.12");
        assert_eq!(p["sparse_infill_density"], "15%");
        assert_eq!(p["curr_bed_type"], "Textured PEI Plate");
        assert_eq!(
            p["compatible_printers"],
            json!(["Bambu Lab H2C 0.4 nozzle"]),
            "an empty list would make the CLI refuse the process"
        );
        let f: Value = serde_json::from_str(&prepared.filaments[0]).unwrap();
        assert_eq!(f["type"], "filament");
        assert_eq!(f["from"], "User");
        assert_eq!(f["inherits"], "");
    }

    #[test]
    fn unknown_presets_are_named() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), None);
        let err = idx
            .prepare(&PresetChoice {
                printer: "Bambu Lab H2C 0.4 nozzle".into(),
                process: "0.20mm Standard @BBL H2C".into(),
                filaments: vec!["Nope PLA".into()],
                bed_type: DEFAULT_BED_TYPE.into(),
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "Preset 'Nope PLA' wasn't found.");
        write(
            root.path(),
            "system/BBL/filament/Orphan.json",
            json!({
                "type":"filament","name":"Orphan","inherits":"Missing @base","instantiation":"true"
            }),
        );
        let idx = PresetIndex::load(root.path(), None);
        let err = idx.flatten(PresetKind::Filament, "Orphan").unwrap_err();
        assert_eq!(
            err,
            SlicerError::MissingParent {
                name: "Orphan".into(),
                parent: "Missing @base".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "Preset 'Orphan' is missing its parent 'Missing @base'."
        );
    }

    fn h2c_choice(process: &str, filament: &str) -> PresetChoice {
        PresetChoice {
            printer: "Bambu Lab H2C 0.4 nozzle".into(),
            process: process.into(),
            filaments: vec![filament.into()],
            bed_type: DEFAULT_BED_TYPE.into(),
        }
    }

    #[test]
    fn a_user_preset_cannot_take_a_system_presets_name() {
        let root = fake_bambu_root();
        // Named like the system @base, and inheriting its own child: if it
        // replaced the system preset it would hijack the chain and loop.
        write(
            root.path(),
            "user/1881310893/filament/base/Bambu PLA Basic @base.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @base","from":"User",
                "inherits":"Bambu PLA Basic @BBL H2C","filament_density":["9.99"]
            }),
        );
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let flat = idx
            .flatten(PresetKind::Filament, "Bambu PLA Basic @base")
            .unwrap();
        assert_eq!(flat.source, PresetSource::System);
        let prepared = idx
            .prepare(&h2c_choice(
                "0.20mm Standard @BBL H2C",
                "Bambu PLA Basic @BBL H2C",
            ))
            .unwrap();
        let f: Value = serde_json::from_str(&prepared.filaments[0]).unwrap();
        assert_eq!(f["filament_density"], json!(["1.26"]));
        assert_eq!(f["from"], "system");
        assert!(!names(&idx.list(None).filaments).contains(&"Bambu PLA Basic @base"));
    }

    #[test]
    fn a_process_made_for_another_printer_is_refused() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let err = idx
            .prepare(&h2c_choice(
                "0.20mm Standard @BBL H2S",
                "Bambu PLA Basic @BBL H2C",
            ))
            .unwrap_err();
        assert_eq!(
            err,
            SlicerError::IncompatibleProcess {
                process: "0.20mm Standard @BBL H2S".into(),
                printer: "Bambu Lab H2C 0.4 nozzle".into(),
            }
        );
        assert_eq!(
            err.to_string(),
            "Process '0.20mm Standard @BBL H2S' isn't made for printer 'Bambu Lab H2C 0.4 nozzle'."
        );
        // An empty list of its own doesn't free a user process from its
        // parent's: My Fine descends from the H2C process.
        let err = idx
            .prepare(&PresetChoice {
                printer: "Bambu Lab H2S 0.4 nozzle".into(),
                process: "My Fine".into(),
                filaments: vec!["Bambu PLA Basic @BBL H2S".into()],
                bed_type: DEFAULT_BED_TYPE.into(),
            })
            .unwrap_err();
        assert_eq!(err.kind(), "incompatible_process");
    }

    #[test]
    fn a_process_for_every_printer_or_the_printers_system_preset_is_accepted() {
        let root = fake_bambu_root();
        write(
            root.path(),
            "system/BBL/process/0.20mm Any.json",
            json!({
                "type":"process","name":"0.20mm Any","inherits":"fdm_process_common",
                "from":"system","instantiation":"true"
            }),
        );
        write(
            root.path(),
            "user/1881310893/machine/My H2C.json",
            json!({
                "type":"machine","name":"My H2C","inherits":"Bambu Lab H2C 0.4 nozzle",
                "from":"User","nozzle_diameter":["0.4","0.4"]
            }),
        );
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let prepared = idx
            .prepare(&PresetChoice {
                printer: "Bambu Lab H2S 0.4 nozzle".into(),
                process: "0.20mm Any".into(),
                filaments: vec!["Bambu PLA Basic @BBL H2S".into()],
                bed_type: DEFAULT_BED_TYPE.into(),
            })
            .unwrap();
        let p: Value = serde_json::from_str(&prepared.process).unwrap();
        assert_eq!(
            p["compatible_printers"],
            json!(["Bambu Lab H2S 0.4 nozzle"]),
            "an empty list means every printer; the CLI still needs the name"
        );
        // A user printer matches a process made for its system preset.
        let prepared = idx
            .prepare(&PresetChoice {
                printer: "My H2C".into(),
                process: "0.20mm Standard @BBL H2C".into(),
                filaments: vec!["Bambu PLA Basic @BBL H2C".into()],
                bed_type: DEFAULT_BED_TYPE.into(),
            })
            .unwrap();
        let m: Value = serde_json::from_str(&prepared.machine).unwrap();
        assert_eq!(m["inherits"], "Bambu Lab H2C 0.4 nozzle");
        let p: Value = serde_json::from_str(&prepared.process).unwrap();
        assert_eq!(
            p["compatible_printers"],
            json!(["Bambu Lab H2C 0.4 nozzle"])
        );
    }

    #[test]
    fn no_filament_is_reported_before_anything_else() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), None);
        let err = idx
            .prepare(&PresetChoice {
                printer: "No Such Printer".into(),
                process: "No Such Process".into(),
                filaments: vec![],
                bed_type: DEFAULT_BED_TYPE.into(),
            })
            .unwrap_err();
        assert_eq!(
            err,
            SlicerError::io("No filament preset chosen. Pick one on the Slice page.")
        );
    }

    #[test]
    fn a_user_filament_inherits_from_its_system_ancestor() {
        let root = fake_bambu_root();
        write(
            root.path(),
            "user/1881310893/filament/My PLA.json",
            json!({
                "type":"filament","name":"My PLA","inherits":"Bambu PLA Basic @BBL H2C",
                "from":"User","filament_cost":["25"]
            }),
        );
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let prepared = idx
            .prepare(&h2c_choice("0.20mm Standard @BBL H2C", "My PLA"))
            .unwrap();
        let f: Value = serde_json::from_str(&prepared.filaments[0]).unwrap();
        assert_eq!(f["from"], "User");
        assert_eq!(f["inherits"], "Bambu PLA Basic @BBL H2C");
        assert_eq!(f["name"], "My PLA");
        assert_eq!(f["filament_cost"], json!(["25"]));
        assert_eq!(f["filament_density"], json!(["1.26"]));
        assert_eq!(f["filament_id"], "GFA00");
    }

    #[test]
    fn write_configs_puts_files_in_cli_order() {
        let dir = tempfile::tempdir().unwrap();
        let prepared = PreparedConfigs {
            machine: "{\"type\":\"machine\"}".into(),
            process: "{\"type\":\"process\"}".into(),
            filaments: vec!["{\"a\":1}".into(), "{\"b\":2}".into()],
        };
        let paths = write_configs(dir.path(), &prepared).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.machine).unwrap(),
            prepared.machine
        );
        assert_eq!(paths.filaments.len(), 2);
        assert!(paths.filaments[1].ends_with("filament_2.json"));
    }

    #[test]
    fn reads_bambu_studios_current_selection_only() {
        let root = fake_bambu_root();
        let sel = bambu_studio_selection(root.path());
        assert_eq!(
            sel,
            BambuStudioSelection {
                printer: Some("Bambu Lab H2C 0.4 nozzle".into()),
                process: Some("0.20mm Standard @BBL H2C".into()),
                filament: Some("Bambu PLA Basic @BBL H2C".into()),
                bed_type: Some("Textured PEI Plate".into()),
            }
        );
        assert_eq!(
            bambu_studio_selection(&root.path().join("missing")),
            BambuStudioSelection::default()
        );
    }

    /// Bed numbers follow Bambu Studio's `BedType` enum (PrintConfig.hpp,
    /// v02.08.02.61): 1 Cool, 2 Engineering, 3 High Temp, 4 Textured PEI,
    /// 5 Supertack; 0 is "Default Plate", not a real plate.
    #[test]
    fn bed_numbers_follow_bambu_studios_enum_and_bad_leaves_read_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let bed = |raw: Value| {
            write(
                dir.path(),
                "BambuStudio.conf",
                json!({"app":{"curr_bed_type":raw},"presets":{"machine":42,"process":"Q"}}),
            );
            bambu_studio_selection(dir.path())
        };
        let expected = [
            (json!("1"), Some("Cool Plate")),
            (json!("2"), Some("Engineering Plate")),
            (json!("3"), Some("High Temp Plate")),
            (json!("4"), Some("Textured PEI Plate")),
            (json!("5"), Some("Supertack Plate")),
            (json!(1), Some("Cool Plate")),
            (json!("0"), None),
            (json!("6"), None),
            (json!("high temp plate"), Some("High Temp Plate")),
            (json!(["4"]), None),
        ];
        for (raw, want) in expected {
            let sel = bed(raw.clone());
            assert_eq!(sel.bed_type.as_deref(), want, "{raw}");
            assert_eq!(sel.printer, None, "a non-string machine reads as unset");
            assert_eq!(sel.process.as_deref(), Some("Q"));
        }
    }

    #[test]
    fn effective_settings_prefer_bambumate_then_bambu_studio() {
        let bs = BambuStudioSelection {
            printer: Some("P".into()),
            process: Some("Q".into()),
            filament: Some("F".into()),
            bed_type: Some("Cool Plate".into()),
        };
        let mine = SlicerSettings {
            filament: Some("Mine".into()),
            auto_slice: true,
            ..Default::default()
        };
        let e = mine.effective(&bs);
        assert_eq!(e.printer.as_deref(), Some("P"));
        assert_eq!(e.filament.as_deref(), Some("Mine"));
        assert_eq!(e.bed_type.as_deref(), Some("Cool Plate"));
        assert!(e.auto_slice);
        assert_eq!(
            SlicerSettings::default()
                .effective(&BambuStudioSelection::default())
                .bed_type
                .as_deref(),
            Some(DEFAULT_BED_TYPE)
        );
    }

    #[test]
    fn choices_fill_from_settings_and_name_what_is_missing() {
        let eff = SlicerSettings {
            printer: Some("P".into()),
            process: Some("Q".into()),
            filament: None,
            bed_type: Some("Cool Plate".into()),
            auto_slice: false,
        };
        assert_eq!(
            eff.choice(None, None, Some("F".into())).unwrap(),
            PresetChoice {
                printer: "P".into(),
                process: "Q".into(),
                filaments: vec!["F".into()],
                bed_type: "Cool Plate".into(),
            }
        );
        assert_eq!(
            eff.choice(None, None, None).unwrap_err(),
            "No filament preset chosen. Pick one on the Slice page."
        );
    }

    /// Runs against the real Bambu Studio data folder on a developer Mac.
    #[test]
    #[ignore = "needs a local Bambu Studio data folder"]
    fn flattens_the_local_h2c_presets() {
        let paths = crate::profile::BambuPaths::detect().unwrap();
        let idx = PresetIndex::load(&paths.config_root, paths.preset_folder.as_deref());
        let prepared = idx
            .prepare(&PresetChoice {
                printer: "Bambu Lab H2C 0.4 nozzle".into(),
                process: "0.20mm Standard @BBL H2C".into(),
                filaments: vec!["Bambu PLA Basic @BBL H2C".into()],
                bed_type: DEFAULT_BED_TYPE.into(),
            })
            .unwrap();
        let m: Value = serde_json::from_str(&prepared.machine).unwrap();
        assert!(m["machine_start_gcode"]
            .as_str()
            .unwrap()
            .contains("machine: H2C"));
        let f: Value = serde_json::from_str(&prepared.filaments[0]).unwrap();
        assert_eq!(f["filament_density"], json!(["1.26"]));
    }
}
