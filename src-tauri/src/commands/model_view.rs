//! Tauri command behind the model viewport: read a model file and return
//! its dots, edges and risks. Read-only; nothing is written.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use crate::model::view::{build_view, load_model, ModelView};
use crate::model::ModelError;

/// Views kept so reopening a model is instant.
const CACHE_SIZE: usize = 4;

/// (path, modified-time nanos, length).
type Key = (PathBuf, u128, u64);

static CACHE: OnceLock<Mutex<VecDeque<(Key, ModelView)>>> = OnceLock::new();

fn cache() -> std::sync::MutexGuard<'static, VecDeque<(Key, ModelView)>> {
    CACHE
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The view for `path`, from the cache when the file is unchanged. The lock
/// is never held while the model is read or parsed.
fn model_view_blocking(path: &Path) -> Result<ModelView, ModelError> {
    // One metadata call, before reading, supplies the key. If it fails,
    // load_model gives the right error (NotAModel for a wrong extension).
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return Err(load_model(path)
                .err()
                .unwrap_or_else(|| ModelError::Io(e.to_string())))
        }
    };
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let key: Key = (path.to_path_buf(), nanos, meta.len());

    {
        let mut c = cache();
        if let Some(i) = c.iter().position(|(k, _)| *k == key) {
            if let Some(entry) = c.remove(i) {
                let view = entry.1.clone();
                c.push_front(entry);
                return Ok(view);
            }
        }
    }

    let mesh = load_model(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let view = build_view(&name, &mesh);

    let mut c = cache();
    c.retain(|(k, _)| *k != key);
    c.push_front((key, view.clone()));
    c.truncate(CACHE_SIZE);
    Ok(view)
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

    #[test]
    fn same_file_twice_gives_equal_views() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cube.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let a = model_view_blocking(&p).unwrap();
        let b = model_view_blocking(&p).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.name, "cube.stl");
        assert_eq!(a.triangle_count, 12);
    }

    #[test]
    fn changed_file_gives_a_new_view() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("change.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let a = model_view_blocking(&p).unwrap();
        let mut tris = cube_tris();
        tris.extend(cube_tris());
        std::fs::write(&p, binary_stl(&tris)).unwrap();
        let b = model_view_blocking(&p).unwrap();
        assert_ne!(a, b);
        assert_eq!(b.triangle_count, 24);
        assert_ne!(a.dots, b.dots);
    }

    #[test]
    fn errors_are_not_cached_and_are_user_facing() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("a.obj");
        assert_eq!(model_view_blocking(&bad), Err(ModelError::NotAModel));
        let p = dir.path().join("late.stl");
        assert!(matches!(model_view_blocking(&p), Err(ModelError::Io(_))));
        std::fs::write(&p, cube_stl()).unwrap();
        assert!(model_view_blocking(&p).is_ok());
    }

    #[test]
    fn cache_keeps_at_most_four_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..6 {
            let p = dir.path().join(format!("m{i}.stl"));
            std::fs::write(&p, cube_stl()).unwrap();
            model_view_blocking(&p).unwrap();
            paths.push(p);
        }
        assert!(cache().len() <= CACHE_SIZE);
    }
}
