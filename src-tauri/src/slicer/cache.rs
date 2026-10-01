//! Slice results cached under `app_data/slices/<key>/`, capped in size with
//! least-recently-used eviction.
//!
//! An entry holds `output.gcode.3mf`, the `plate_N.png` thumbnails and
//! `summary.json` (the parsed [`SliceResult`]). The summary's modification
//! time is the entry's "last used" time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use super::command::OUTPUT_FILE;
use super::result::SliceResult;
use super::settings::PreparedConfigs;
use super::{ModelKind, SlicerError};

/// 1 GB.
pub const DEFAULT_CAP_BYTES: u64 = 1024 * 1024 * 1024;
pub const SUMMARY_FILE: &str = "summary.json";
const STAGING_PREFIX: &str = ".staging-";

/// SHA-256 over everything that changes the output: the model's bytes, the
/// exact config files (in CLI order) and the Bambu Studio version.
pub fn cache_key(
    model: &Path,
    kind: ModelKind,
    configs: &PreparedConfigs,
    bambu_studio_version: &str,
) -> std::io::Result<String> {
    let mut h = Sha256::new();
    h.update(b"bambumate-slice-v1\0");
    h.update(bambu_studio_version.as_bytes());
    h.update(b"\0");
    h.update(match kind {
        ModelKind::Stl => b"stl\0" as &[u8],
        ModelKind::ThreeMf => b"3mf\0",
    });
    let mut file = std::fs::File::open(model)?;
    let mut model_hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        model_hash.update(&buf[..n]);
    }
    h.update(model_hash.finalize());
    for part in std::iter::once(&configs.machine)
        .chain(std::iter::once(&configs.process))
        .chain(configs.filaments.iter())
    {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn is_key(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn dir_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn touch(path: &Path, at: SystemTime) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(at);
    }
}

pub struct SliceCache {
    root: PathBuf,
    cap: u64,
}

impl SliceCache {
    pub fn new(root: PathBuf, cap: u64) -> Self {
        Self { root, cap }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder for `key`, or `None` when `key` isn't a cache key (64
    /// hex characters), so no other path can be reached through it.
    pub fn entry_dir(&self, key: &str) -> Option<PathBuf> {
        is_key(key).then(|| self.root.join(key))
    }

    /// A cached result, marked as just used. Its `output_path` points at
    /// the cached file.
    pub fn get(&self, key: &str) -> Option<SliceResult> {
        let dir = self.entry_dir(key)?;
        let summary = dir.join(SUMMARY_FILE);
        let output = dir.join(OUTPUT_FILE);
        if !output.is_file() {
            return None;
        }
        let mut result: SliceResult =
            serde_json::from_str(&std::fs::read_to_string(&summary).ok()?).ok()?;
        result.output_path = output.to_string_lossy().into_owned();
        touch(&summary, SystemTime::now());
        Some(result)
    }

    /// A fresh staging folder inside the cache, so finishing an entry is a
    /// rename on the same disk.
    pub fn staging_dir(&self) -> Result<PathBuf, SlicerError> {
        let dir = self
            .root
            .join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| SlicerError::io(e.to_string()))?;
        Ok(dir)
    }

    /// Turns a staging folder (holding the output and thumbnails) into the
    /// entry for `key`, then evicts old entries over the cap.
    pub fn put(
        &self,
        key: &str,
        staging: &Path,
        result: &SliceResult,
    ) -> Result<SliceResult, SlicerError> {
        let io = |e: std::io::Error| SlicerError::io(e.to_string());
        let dir = self
            .entry_dir(key)
            .ok_or_else(|| SlicerError::io(format!("bad cache key {key}")))?;
        let mut stored = result.clone();
        stored.output_path = String::new();
        std::fs::write(
            staging.join(SUMMARY_FILE),
            serde_json::to_vec(&stored).map_err(|e| SlicerError::io(e.to_string()))?,
        )
        .map_err(io)?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(io)?;
        }
        std::fs::rename(staging, &dir).map_err(io)?;
        stored.output_path = dir.join(OUTPUT_FILE).to_string_lossy().into_owned();
        self.evict(Some(key));
        Ok(stored)
    }

    /// Removes least recently used entries until the cache fits its cap.
    /// `keep` is never removed. Returns the bytes freed.
    pub fn evict(&self, keep: Option<&str>) -> u64 {
        let Ok(read) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        let mut entries: Vec<(SystemTime, u64, PathBuf, String)> = read
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !is_key(&name) {
                    return None;
                }
                let path = e.path();
                let used = std::fs::metadata(path.join(SUMMARY_FILE))
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                Some((used, dir_size(&path), path, name))
            })
            .collect();
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        entries.sort_by_key(|e| e.0);
        let mut freed = 0;
        for (_, size, path, name) in entries {
            if total <= self.cap {
                break;
            }
            if Some(name.as_str()) == keep {
                continue;
            }
            if std::fs::remove_dir_all(&path).is_ok() {
                total -= size;
                freed += size;
            }
        }
        freed
    }

    /// Empties the cache, including leftover staging folders. Returns the
    /// bytes freed.
    pub fn clear(&self) -> u64 {
        self.remove_where(|name| is_key(name) || name.starts_with(STAGING_PREFIX))
    }

    /// Removes staging folders a crash left behind. Only call it when no
    /// job can be writing one (at startup). Returns the bytes freed.
    pub fn remove_staging(&self) -> u64 {
        self.remove_where(|name| name.starts_with(STAGING_PREFIX))
    }

    fn remove_where(&self, matches: impl Fn(&str) -> bool) -> u64 {
        let Ok(read) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        let mut freed = 0;
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if matches(&name) {
                let size = dir_size(&e.path());
                if std::fs::remove_dir_all(e.path()).is_ok() {
                    freed += size;
                }
            }
        }
        freed
    }

    /// Bytes used by finished entries.
    pub fn size(&self) -> u64 {
        std::fs::read_dir(&self.root)
            .map(|r| {
                r.flatten()
                    .filter(|e| is_key(&e.file_name().to_string_lossy()))
                    .map(|e| dir_size(&e.path()))
                    .sum()
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn configs(filament: &str) -> PreparedConfigs {
        PreparedConfigs {
            machine: "{\"name\":\"m\"}".into(),
            process: "{\"name\":\"p\"}".into(),
            filaments: vec![filament.to_string()],
        }
    }

    fn sample_result() -> SliceResult {
        SliceResult {
            plates: vec![],
            printer: "Bambu Lab H2C 0.4 nozzle".into(),
            printer_model: "Bambu Lab H2C".into(),
            process: "0.20mm Standard @BBL H2C".into(),
            filaments: vec![],
            bed_type: None,
            bambu_studio_version: "02.08.02.61".into(),
            output_path: String::new(),
        }
    }

    #[test]
    fn key_changes_when_any_input_changes() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("cube.stl");
        std::fs::write(&model, b"solid a").unwrap();
        let base = cache_key(&model, ModelKind::Stl, &configs("{\"f\":1}"), "02.08.02.61").unwrap();
        assert_eq!(base.len(), 64);
        assert_eq!(
            base,
            cache_key(&model, ModelKind::Stl, &configs("{\"f\":1}"), "02.08.02.61").unwrap(),
            "stable for the same inputs"
        );
        let other_filament =
            cache_key(&model, ModelKind::Stl, &configs("{\"f\":2}"), "02.08.02.61").unwrap();
        let other_version =
            cache_key(&model, ModelKind::Stl, &configs("{\"f\":1}"), "02.09.00.00").unwrap();
        let mut machine = configs("{\"f\":1}");
        machine.machine.push(' ');
        let other_machine = cache_key(&model, ModelKind::Stl, &machine, "02.08.02.61").unwrap();
        let mut process = configs("{\"f\":1}");
        process.process.push(' ');
        let other_process = cache_key(&model, ModelKind::Stl, &process, "02.08.02.61").unwrap();
        std::fs::write(&model, b"solid b").unwrap();
        let other_kind = cache_key(
            &model,
            ModelKind::ThreeMf,
            &configs("{\"f\":1}"),
            "02.08.02.61",
        )
        .unwrap();
        let other_model =
            cache_key(&model, ModelKind::Stl, &configs("{\"f\":1}"), "02.08.02.61").unwrap();
        let all = [
            &base,
            &other_filament,
            &other_version,
            &other_machine,
            &other_process,
            &other_kind,
            &other_model,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    fn put_entry(cache: &SliceCache, key: &str, bytes: usize) {
        let staging = cache.staging_dir().unwrap();
        std::fs::write(staging.join(OUTPUT_FILE), vec![0u8; bytes]).unwrap();
        cache.put(key, &staging, &sample_result()).unwrap();
    }

    fn key(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    #[test]
    fn put_then_get_round_trips_with_the_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SliceCache::new(dir.path().to_path_buf(), DEFAULT_CAP_BYTES);
        put_entry(&cache, &key(1), 10);
        let got = cache.get(&key(1)).unwrap();
        assert_eq!(got.printer, "Bambu Lab H2C 0.4 nozzle");
        assert_eq!(
            PathBuf::from(&got.output_path),
            dir.path().join(key(1)).join(OUTPUT_FILE)
        );
        assert!(cache.get(&key(2)).is_none());
        assert!(cache.get("../../etc").is_none());
        assert_eq!(cache.entry_dir(&key(1)), Some(dir.path().join(key(1))));
        assert_eq!(cache.entry_dir("../../etc"), None);
        assert_eq!(cache.entry_dir(&"g".repeat(64)), None);
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SliceCache::new(dir.path().to_path_buf(), 2800);
        let t0 = SystemTime::now() - Duration::from_secs(1000);
        put_entry(&cache, &key(1), 1000);
        touch(&dir.path().join(key(1)).join(SUMMARY_FILE), t0);
        put_entry(&cache, &key(2), 1000);
        touch(
            &dir.path().join(key(2)).join(SUMMARY_FILE),
            t0 + Duration::from_secs(10),
        );
        // Using entry 1 makes entry 2 the least recently used.
        assert!(cache.get(&key(1)).is_some());
        put_entry(&cache, &key(3), 1000);
        assert!(cache.get(&key(1)).is_some(), "recently used survives");
        assert!(cache.get(&key(2)).is_none(), "LRU entry evicted");
        assert!(cache.get(&key(3)).is_some(), "the new entry is kept");
        assert!(cache.size() <= 2800);
    }

    #[test]
    fn the_newest_entry_is_kept_even_when_it_alone_exceeds_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SliceCache::new(dir.path().to_path_buf(), 100);
        put_entry(&cache, &key(1), 500);
        assert!(cache.get(&key(1)).is_some());
    }

    #[test]
    fn clear_removes_entries_and_staging_but_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SliceCache::new(dir.path().to_path_buf(), DEFAULT_CAP_BYTES);
        put_entry(&cache, &key(1), 100);
        let _staging = cache.staging_dir().unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), b"keep").unwrap();
        assert!(cache.clear() >= 100);
        assert!(cache.get(&key(1)).is_none());
        assert_eq!(cache.size(), 0);
        assert!(dir.path().join("unrelated.txt").exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_half_written_entry_is_never_read_and_is_swept_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SliceCache::new(dir.path().to_path_buf(), DEFAULT_CAP_BYTES);
        put_entry(&cache, &key(1), 100);
        let before = cache.size();
        // A crash between staging and the rename leaves only staging.
        let staging = cache.staging_dir().unwrap();
        std::fs::write(staging.join(OUTPUT_FILE), vec![0u8; 50]).unwrap();
        assert_eq!(cache.evict(None), 0);
        assert_eq!(cache.size(), before, "staging isn't counted as an entry");
        assert_eq!(cache.remove_staging(), 50);
        assert!(!staging.exists());
        assert!(cache.get(&key(1)).is_some(), "entries stay");
    }
}
