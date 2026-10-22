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
/// from `build`. The lock is never held while `build` runs, and only
/// successes are cached.
fn cached_view(
    path: &Path,
    cache: &Mutex<ViewCache>,
    build: impl FnOnce(OpenModel) -> Result<ModelView, ModelError>,
) -> Result<ModelView, ModelError> {
    let open = open_model(path)?;
    let key = key_for(path, &open);
    if let Some(key) = &key {
        if let Some(view) = lock(cache).get(key) {
            return Ok(view);
        }
    }
    let view = build(open)?;
    if let Some(key) = key {
        lock(cache).put(key, view.clone());
    }
    Ok(view)
}

fn model_view_blocking(path: &Path) -> Result<ModelView, ModelError> {
    let cache = CACHE.get_or_init(|| Mutex::new(ViewCache::new(CACHE_SIZE)));
    cached_view(path, cache, |open| {
        let mesh = open.parse()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(build_view(&name, &mesh))
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

    #[test]
    fn unchanged_file_is_not_rebuilt_but_a_changed_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let cache = Mutex::new(ViewCache::new(CACHE_SIZE));
        let builds = Cell::new(0);
        let build = |open: OpenModel| {
            builds.set(builds.get() + 1);
            Ok(build_view("cube.stl", &open.parse()?))
        };
        let a = cached_view(&p, &cache, build).unwrap();
        let b = cached_view(&p, &cache, build).unwrap();
        assert_eq!(builds.get(), 1, "second call must come from the cache");
        assert_eq!(a, b);

        let mut tris = cube_tris();
        tris.extend(cube_tris());
        std::fs::write(&p, binary_stl(&tris)).unwrap();
        let c = cached_view(&p, &cache, build).unwrap();
        assert_eq!(builds.get(), 2);
        assert_eq!(c.triangle_count, 24);
    }

    #[test]
    fn failures_are_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let cache = Mutex::new(ViewCache::new(CACHE_SIZE));
        let r = cached_view(&p, &cache, |_| Err(ModelError::Truncated));
        assert_eq!(r, Err(ModelError::Truncated));
        assert!(lock(&cache).entries.is_empty());
        assert!(cached_view(&p, &cache, |o| Ok(build_view("c", &o.parse()?))).is_ok());
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
