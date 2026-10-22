//! Tauri command behind the model viewport: read a model file and return
//! its dots, edges and risks. Read-only; nothing is written.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::UNIX_EPOCH;

use crate::model::view::{build_view, open_model, ModelView, OpenModel};
use crate::model::ModelError;

/// Views kept so reopening a model is instant.
const CACHE_SIZE: usize = 4;

/// (path, modified-time nanos, length).
type Key = (PathBuf, u128, u64);

/// Most-recently-used first.
struct ViewCache {
    entries: VecDeque<(Key, ModelView)>,
    cap: usize,
}

impl ViewCache {
    fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            cap,
        }
    }

    /// The view for `key`, which becomes the most recently used.
    fn get(&mut self, key: &Key) -> Option<ModelView> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(i)?;
        let view = entry.1.clone();
        self.entries.push_front(entry);
        Some(view)
    }

    /// Store `view` as the most recently used, replacing any entry with the
    /// same key and dropping the least recently used beyond the cap.
    fn put(&mut self, key: Key, view: ModelView) {
        self.entries.retain(|(k, _)| *k != key);
        self.entries.push_front((key, view));
        self.entries.truncate(self.cap);
    }
}

static CACHE: OnceLock<Mutex<ViewCache>> = OnceLock::new();

fn new_cache() -> Mutex<ViewCache> {
    Mutex::new(ViewCache::new(CACHE_SIZE))
}

fn lock(cache: &Mutex<ViewCache>) -> MutexGuard<'_, ViewCache> {
    cache.lock().unwrap_or_else(|e| e.into_inner())
}

/// The cache key for an opened file, or `None` when it can't be trusted:
/// without a usable modified time, a same-size rewrite would look unchanged.
fn key_for(path: &Path, open: &OpenModel) -> Option<Key> {
    let nanos = open.modified?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some((path.to_path_buf(), nanos, open.len))
}

/// The view for `path`: from `cache` when the file is unchanged, otherwise
/// from `build`, which says whether its result may be cached (false when the
/// file changed while it was being read). The lock is never held while
/// `build` runs, and only cacheable successes are stored.
fn cached_view(
    path: &Path,
    cache: &Mutex<ViewCache>,
    open: impl FnOnce(&Path) -> Result<OpenModel, ModelError>,
    build: impl FnOnce(OpenModel) -> Result<(ModelView, bool), ModelError>,
) -> Result<ModelView, ModelError> {
    let open = open(path)?;
    let key = key_for(path, &open);
    if let Some(key) = &key {
        if let Some(view) = lock(cache).get(key) {
            return Ok(view);
        }
    }
    let (view, cacheable) = build(open)?;
    if let (Some(key), true) = (key, cacheable) {
        lock(cache).put(key, view.clone());
    }
    Ok(view)
}

fn model_view_blocking(path: &Path) -> Result<ModelView, ModelError> {
    let cache = CACHE.get_or_init(new_cache);
    cached_view(path, cache, open_model, |open| {
        let (mesh, unchanged) = open.parse_checked()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok((build_view(&name, &mesh), unchanged))
    })
}

/// Read a `.stl` or `.3mf` and return what the viewport draws.
#[tauri::command]
pub async fn model_view(path: String) -> Result<ModelView, String> {
    tauri::async_runtime::spawn_blocking(move || model_view_blocking(Path::new(&path)))
        .await
        .map_err(|_| "The model couldn't be read.".to_string())?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::stl::tests_support::{binary_stl, cube_stl, cube_tris};
    use std::cell::Cell;
    use std::time::{Duration, SystemTime};

    fn key(n: &str) -> Key {
        (PathBuf::from(n), 1, 1)
    }

    fn view(name: &str) -> ModelView {
        ModelView {
            name: name.into(),
            triangle_count: 0,
            bounds_min: [0.0; 3],
            bounds_max: [0.0; 3],
            dots: String::new(),
            dot_normals: String::new(),
            edges: String::new(),
            risks: Vec::new(),
        }
    }

    fn has(c: &ViewCache, n: &str) -> bool {
        c.entries.iter().any(|(k, _)| *k == key(n))
    }

    #[test]
    fn cache_hit_and_miss() {
        let mut c = ViewCache::new(4);
        assert!(c.get(&key("a")).is_none());
        c.put(key("a"), view("a"));
        assert_eq!(c.get(&key("a")).unwrap().name, "a");
        assert!(c.get(&key("b")).is_none());
        // Same path with a different stamp is a different entry.
        assert!(c.get(&(PathBuf::from("a"), 2, 1)).is_none());
    }

    #[test]
    fn cache_evicts_the_oldest_past_the_cap() {
        let mut c = ViewCache::new(3);
        for n in ["a", "b", "c", "d"] {
            c.put(key(n), view(n));
        }
        assert!(!has(&c, "a"));
        assert!(has(&c, "b") && has(&c, "c") && has(&c, "d"));
        assert_eq!(c.entries.len(), 3);
    }

    #[test]
    fn cache_hit_moves_to_front_so_it_outlives_newer_entries() {
        let mut c = ViewCache::new(3);
        for n in ["a", "b", "c"] {
            c.put(key(n), view(n));
        }
        assert!(c.get(&key("a")).is_some());
        c.put(key("d"), view("d"));
        assert!(has(&c, "a"), "a was just used, so it must survive");
        assert!(!has(&c, "b"), "b is now the least recently used");
    }

    #[test]
    fn cache_put_replaces_the_same_key() {
        let mut c = ViewCache::new(3);
        c.put(key("a"), view("old"));
        c.put(key("a"), view("new"));
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.get(&key("a")).unwrap().name, "new");
    }

    fn build(open: OpenModel) -> Result<(ModelView, bool), ModelError> {
        let (mesh, unchanged) = open.parse_checked()?;
        Ok((build_view("m.stl", &mesh), unchanged))
    }

    fn write_and_stamp(p: &Path, bytes: &[u8], t: SystemTime) {
        std::fs::write(p, bytes).unwrap();
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    /// `cached_view` with a build counter.
    fn run(p: &Path, cache: &Mutex<ViewCache>, builds: &Cell<u32>) -> ModelView {
        cached_view(p, cache, open_model, |o| {
            builds.set(builds.get() + 1);
            build(o)
        })
        .unwrap()
    }

    #[test]
    fn unchanged_file_is_not_rebuilt_but_a_changed_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let cache = new_cache();
        let builds = Cell::new(0);
        let a = run(&p, &cache, &builds);
        let b = run(&p, &cache, &builds);
        assert_eq!(builds.get(), 1, "second call must come from the cache");
        assert_eq!(a, b);

        let mut tris = cube_tris();
        tris.extend(cube_tris());
        std::fs::write(&p, binary_stl(&tris)).unwrap();
        let c = run(&p, &cache, &builds);
        assert_eq!(builds.get(), 2);
        assert_eq!(c.triangle_count, 24);
    }

    #[test]
    fn same_length_rewrite_with_a_newer_mtime_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        write_and_stamp(&p, &cube_stl(), t);
        let cache = new_cache();
        let builds = Cell::new(0);
        run(&p, &cache, &builds);
        // Same triangle count, so the same length; the cube is twice as big.
        let big: Vec<_> = cube_tris()
            .into_iter()
            .map(|t| t.map(|v| v.map(|x| x * 2.0)))
            .collect();
        let bytes = binary_stl(&big);
        assert_eq!(bytes.len(), cube_stl().len());
        write_and_stamp(&p, &bytes, t + Duration::from_secs(2));
        let v = run(&p, &cache, &builds);
        assert_eq!(builds.get(), 2, "the mtime is part of the key");
        assert_eq!(v.bounds_max, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn two_files_with_the_same_length_and_mtime_do_not_share_an_entry() {
        let dir = tempfile::tempdir().unwrap();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let (a, b) = (dir.path().join("a.stl"), dir.path().join("b.stl"));
        write_and_stamp(&a, &cube_stl(), t);
        write_and_stamp(&b, &cube_stl(), t);
        let cache = new_cache();
        let builds = Cell::new(0);
        run(&a, &cache, &builds);
        run(&b, &cache, &builds);
        assert_eq!(builds.get(), 2, "the path is part of the key");
        run(&a, &cache, &builds);
        run(&b, &cache, &builds);
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn a_file_with_no_modified_time_is_neither_looked_up_nor_cached() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let cache = new_cache();
        let builds = Cell::new(0);
        let no_mtime = |p: &Path| {
            let mut o = open_model(p)?;
            o.modified = None;
            Ok(o)
        };
        for _ in 0..2 {
            cached_view(&p, &cache, no_mtime, |o| {
                builds.set(builds.get() + 1);
                build(o)
            })
            .unwrap();
        }
        assert_eq!(builds.get(), 2);
        assert!(lock(&cache).entries.is_empty());
    }

    #[test]
    fn a_file_rewritten_while_being_read_is_returned_but_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let cache = new_cache();
        let builds = Cell::new(0);
        let mut tris = cube_tris();
        tris.extend(cube_tris());
        let v = cached_view(&p, &cache, open_model, |o| {
            builds.set(builds.get() + 1);
            // Rewritten between open and read.
            std::fs::write(&p, binary_stl(&tris)).unwrap();
            build(o)
        })
        .unwrap();
        assert_eq!(v.triangle_count, 24);
        assert!(lock(&cache).entries.is_empty());
        // The next call sees the new file and caches it.
        run(&p, &cache, &builds);
        run(&p, &cache, &builds);
        assert_eq!(builds.get(), 2);
        assert_eq!(lock(&cache).entries.len(), 1);
    }

    #[test]
    fn failures_are_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let cache = new_cache();
        let r = cached_view(&p, &cache, open_model, |_| Err(ModelError::Truncated));
        assert_eq!(r, Err(ModelError::Truncated));
        assert!(lock(&cache).entries.is_empty());
        assert!(cached_view(&p, &cache, open_model, build).is_ok());
    }

    #[test]
    fn the_global_cache_is_built_with_the_cache_size() {
        assert_eq!(CACHE_SIZE, 4);
        assert_eq!(lock(&new_cache()).cap, CACHE_SIZE);
    }

    #[test]
    fn no_modified_time_means_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let mut open = open_model(&p).unwrap();
        assert!(key_for(&p, &open).is_some());
        open.modified = None;
        assert!(key_for(&p, &open).is_none());
        open.modified = Some(SystemTime::UNIX_EPOCH - Duration::from_secs(1));
        assert!(key_for(&p, &open).is_none());
    }

    #[test]
    fn wrong_or_missing_files_give_user_facing_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            model_view_blocking(&dir.path().join("a.obj")),
            Err(ModelError::NotAModel)
        );
        assert!(matches!(
            model_view_blocking(&dir.path().join("nope.stl")),
            Err(ModelError::Io(_))
        ));
    }

    #[test]
    fn command_path_returns_the_cube() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let v = model_view_blocking(&p).unwrap();
        assert_eq!((v.name.as_str(), v.triangle_count), ("cube.stl", 12));
        assert_eq!(model_view_blocking(&p).unwrap(), v);
    }
}
