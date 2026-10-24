//! Tauri commands for the filament advisor.

use std::path::Path;

use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;
use tracing::warn;

use crate::advisor::advice::{build_advice, Advice, AdviceInput};
use crate::advisor::interpret::{interpret, AiConfig};
use crate::advisor::needs::Need;
use crate::advisor::products::{brands_from_value, brands_to_value, PREFERRED_BRANDS_KEY};
use crate::advisor::score::Geometry;
use crate::commands::model_view::model_view_blocking;
use crate::commands::scraper::{
    get_ai_model, get_ai_provider, get_api_key_for_provider, get_catalog_path, use_ai_for_filament,
};
use crate::model::ModelError;
use crate::printer::service::PrinterService;
use crate::scraper::catalog::FilamentCatalog;

const STORE: &str = "preferences.json";
pub const INTERPRET_FAILED: &str = "Couldn't read the description; pick the needs below.";

/// The configured AI, or `None` when AI is off or not set up.
fn ai_config(app: &AppHandle) -> Option<AiConfig> {
    if !use_ai_for_filament(app) {
        return None;
    }
    let provider = get_ai_provider(app).ok()?;
    let model = get_ai_model(app).ok()?;
    let api_key = get_api_key_for_provider(app, &provider).ok()?;
    Some(AiConfig {
        provider,
        model,
        api_key,
    })
}

fn read_preferred(app: &AppHandle) -> Vec<String> {
    match app.store(STORE) {
        Ok(store) => brands_from_value(store.get(PREFERRED_BRANDS_KEY).as_ref()),
        Err(e) => {
            warn!("preferred brands unavailable: {e}");
            Vec::new()
        }
    }
}

/// What to store for `brands`, and the list that reads back from it.
fn stored_brands(brands: &[String]) -> (serde_json::Value, Vec<String>) {
    let value = brands_to_value(brands);
    let read_back = brands_from_value(Some(&value));
    (value, read_back)
}

/// The model's geometry through `load` (the cached `model_view` path).
/// `Ok(None)` without a path; `Err` is a plain sentence, so the advice can
/// go ahead without geometry.
fn geometry_for(
    model_path: Option<&str>,
    load: impl FnOnce(&Path) -> Result<crate::model::view::ModelView, ModelError>,
) -> Result<Option<Geometry>, String> {
    match model_path.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(p) => load(Path::new(p))
            .map(|v| Some(Geometry::from_view(&v)))
            .map_err(|e| {
                warn!("advisor: model unreadable: {e}");
                e.user_message()
            }),
    }
}

/// The needs a description implies. Errs with [`INTERPRET_FAILED`] when AI
/// is off or its answer can't be used; the page then scores from the chips.
#[tauri::command]
pub async fn advise_interpret(app: AppHandle, text: String) -> Result<Vec<Need>, String> {
    let ai = ai_config(&app);
    interpret(&text, ai.as_ref())
        .await
        .ok_or_else(|| INTERPRET_FAILED.to_string())
}

/// Score materials for `needs`, using the printer, its AMS, the model's
/// geometry (cached by `model_view`) and the catalog. Read-only. The
/// optional inputs fall back instead of failing: no printer scores against
/// the unknown printer, an unreadable model scores without geometry (and
/// says why), and a missing catalog is empty.
#[tauri::command]
pub async fn advise(
    app: AppHandle,
    service: State<'_, PrinterService>,
    needs: Vec<Need>,
    model_path: Option<String>,
) -> Result<Advice, String> {
    let service = service.inner().clone();
    let catalog_path = get_catalog_path(&app)
        .map_err(|e| warn!("advisor: no catalog path: {e}"))
        .ok();
    let preferred = read_preferred(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let view = service.view();
        let geometry = geometry_for(model_path.as_deref(), model_view_blocking);
        let catalog = catalog_path
            .ok_or_else(|| "no catalog path".to_string())
            .and_then(|p| FilamentCatalog::new(&p))
            .and_then(|c| c.all())
            .unwrap_or_else(|e| {
                warn!("advisor: catalog unavailable: {e}");
                Vec::new()
            });
        build_advice(AdviceInput {
            needs: &needs,
            view: &view,
            geometry,
            catalog: &catalog,
            preferred: &preferred,
        })
    })
    .await
    .map_err(|_| "The advisor couldn't finish; try again.".to_string())
}

#[tauri::command]
pub fn get_preferred_brands(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(read_preferred(&app))
}

/// The slice of the store that saving the brands touches.
trait BrandStore {
    fn get_value(&self, key: &str) -> Option<serde_json::Value>;
    fn set_value(&self, key: &str, value: serde_json::Value);
    fn delete_value(&self, key: &str);
    fn save_to_disk(&self) -> Result<(), String>;
}

impl BrandStore for tauri_plugin_store::Store<tauri::Wry> {
    fn get_value(&self, key: &str) -> Option<serde_json::Value> {
        self.get(key)
    }
    fn set_value(&self, key: &str, value: serde_json::Value) {
        self.set(key, value);
    }
    fn delete_value(&self, key: &str) {
        self.delete(key);
    }
    fn save_to_disk(&self) -> Result<(), String> {
        self.save().map_err(|e| e.to_string())
    }
}

/// Set `value` and save it. The in-memory value is what `advise` and
/// `get_preferred_brands` read, so when the save fails the previous value
/// goes back (or the key is removed if it was absent): the list that failed
/// to save must not stay live.
fn persist_brands(store: &impl BrandStore, value: serde_json::Value) -> Result<(), String> {
    let previous = store.get_value(PREFERRED_BRANDS_KEY);
    store.set_value(PREFERRED_BRANDS_KEY, value);
    store.save_to_disk().inspect_err(|_| match previous {
        Some(old) => store.set_value(PREFERRED_BRANDS_KEY, old),
        None => store.delete_value(PREFERRED_BRANDS_KEY),
    })
}

/// Store the brands (normalised) and return what was stored.
#[tauri::command]
pub fn set_preferred_brands(app: AppHandle, brands: Vec<String>) -> Result<Vec<String>, String> {
    let failed = |e: &dyn std::fmt::Display| {
        warn!("preferred brands not saved: {e}");
        "Couldn't save your preferred brands.".to_string()
    };
    let store = app.store(STORE).map_err(|e| failed(&e))?;
    let (value, stored) = stored_brands(&brands);
    persist_brands(store.as_ref(), value).map_err(|e| failed(&e))?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_failure_sentence_is_the_one_the_page_shows() {
        assert_eq!(
            INTERPRET_FAILED,
            "Couldn't read the description; pick the needs below."
        );
    }

    #[test]
    fn no_model_path_means_no_geometry_and_no_error() {
        let load = |_: &Path| -> Result<_, ModelError> { panic!("must not load") };
        assert_eq!(geometry_for(None, load), Ok(None));
        assert_eq!(geometry_for(Some("   "), load), Ok(None));
    }

    #[test]
    fn a_model_that_fails_to_load_becomes_a_plain_sentence() {
        let got = geometry_for(Some("/x/part.stl"), |_| Err(ModelError::Truncated));
        assert_eq!(got, Err("The STL file is cut short.".to_string()));
    }

    #[test]
    fn os_error_text_never_reaches_the_page() {
        let got = geometry_for(Some("/x/gone.stl"), |_| {
            Err(ModelError::Io(
                "No such file or directory (os error 2)".into(),
            ))
        });
        assert_eq!(got, Err("The model file couldn't be read.".to_string()));
        // The other kinds are already fixed sentences.
        let got = geometry_for(Some("/x/a.3mf"), |_| {
            Err(ModelError::Archive("The 3MF is damaged."))
        });
        assert_eq!(got, Err("The 3MF is damaged.".to_string()));
    }

    #[test]
    fn the_model_path_is_trimmed_before_loading() {
        let mut seen = String::new();
        let _ = geometry_for(Some("  /x/part.stl \n"), |p| {
            seen = p.to_string_lossy().into_owned();
            Err(ModelError::Empty)
        });
        assert_eq!(seen, "/x/part.stl");
    }

    #[test]
    fn preferred_brands_round_trip_normalised() {
        let (value, stored) = stored_brands(&[
            " Polymaker ".into(),
            "polymaker".into(),
            "".into(),
            "Bambu Lab".into(),
        ]);
        assert_eq!(value, json!(["Polymaker", "Bambu Lab"]));
        assert_eq!(stored, ["Polymaker", "Bambu Lab"]);
        assert_eq!(brands_from_value(Some(&value)), stored);
    }

    #[test]
    fn unreadable_stored_brands_read_as_none() {
        assert!(brands_from_value(None).is_empty());
        assert!(brands_from_value(Some(&json!("Polymaker"))).is_empty());
    }

    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeStore {
        data: RefCell<HashMap<String, serde_json::Value>>,
        fail_save: bool,
    }

    impl BrandStore for FakeStore {
        fn get_value(&self, key: &str) -> Option<serde_json::Value> {
            self.data.borrow().get(key).cloned()
        }
        fn set_value(&self, key: &str, value: serde_json::Value) {
            self.data.borrow_mut().insert(key.to_string(), value);
        }
        fn delete_value(&self, key: &str) {
            self.data.borrow_mut().remove(key);
        }
        fn save_to_disk(&self) -> Result<(), String> {
            if self.fail_save {
                Err("disk full".to_string())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn a_saved_list_stays_in_the_store() {
        let store = FakeStore::default();
        assert_eq!(persist_brands(&store, json!(["Polymaker"])), Ok(()));
        assert_eq!(
            store.get_value(PREFERRED_BRANDS_KEY),
            Some(json!(["Polymaker"]))
        );
    }

    #[test]
    fn a_failed_save_puts_the_previous_list_back() {
        let store = FakeStore {
            fail_save: true,
            ..Default::default()
        };
        store.set_value(PREFERRED_BRANDS_KEY, json!(["Bambu Lab"]));
        assert!(persist_brands(&store, json!(["Polymaker"])).is_err());
        assert_eq!(
            store.get_value(PREFERRED_BRANDS_KEY),
            Some(json!(["Bambu Lab"]))
        );
    }

    #[test]
    fn a_failed_save_removes_a_key_that_was_absent() {
        let store = FakeStore {
            fail_save: true,
            ..Default::default()
        };
        assert!(persist_brands(&store, json!(["Polymaker"])).is_err());
        assert_eq!(store.get_value(PREFERRED_BRANDS_KEY), None);
    }
}
