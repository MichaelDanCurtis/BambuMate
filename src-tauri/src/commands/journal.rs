//! Commands for Printer → Journal (spec 2026-10-24 §3). Every database and
//! file call runs on a blocking thread. Errors are the plain sentences of
//! `journal::store` and `journal::photos`; every write emits
//! `journal://changed`.

use tauri::{AppHandle, Emitter, Manager};

use crate::journal::photos::{self, image_data_url, image_data_url_within};
use crate::journal::store::{month_bounds, JournalStore, GONE, PHOTO_FAILED, READ_FAILED};
use crate::journal::types::{EntryUpdate, JournalFilter, MonthSummary, PrintEntry};
use crate::journal::{JournalPaths, CHANGED_EVENT};

/// What the page is told when a photo or thumbnail can't be shown. The page
/// shows it as "Photo unavailable".
pub const PHOTO_UNAVAILABLE: &str = "Photo unavailable";

fn paths(app: &AppHandle) -> Result<JournalPaths, String> {
    app.path()
        .app_data_dir()
        .map(|dir| JournalPaths::in_dir(&dir))
        .map_err(|e| {
            tracing::warn!("print journal: no app data folder: {e}");
            READ_FAILED.to_string()
        })
}

/// Runs `f` against the journal on a blocking thread. A task that panics
/// gives `on_panic`.
async fn run_blocking<T, F>(paths: JournalPaths, on_panic: &'static str, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&JournalStore) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(&JournalStore::open(&paths)?))
        .await
        .map_err(|e| {
            tracing::warn!("print journal task failed: {e}");
            on_panic.to_string()
        })?
}

/// [`run_blocking`] on the app's journal; a panic reads as the journal
/// not being readable.
async fn with_store<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&JournalStore) -> Result<T, String> + Send + 'static,
{
    run_blocking(paths(app)?, READ_FAILED, f).await
}

fn changed(app: &AppHandle) {
    let _ = app.emit(CHANGED_EVENT, ());
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Analyze's defect ids (`config/defect_rules.toml`): the only issue tags
/// an entry takes.
pub(crate) fn known_tags() -> Vec<String> {
    let mut tags: Vec<String> = crate::mapper::default_rules().defects.into_keys().collect();
    tags.sort();
    tags
}

/// Decodes, checks and re-encodes the page's photo, saves it on the entry
/// and returns the entry.
fn add_photo_to(
    store: &JournalStore,
    id: i64,
    data_base64: String,
    now: i64,
) -> Result<PrintEntry, String> {
    let bytes = photos::decode_upload(&data_base64)?;
    drop(data_base64);
    let jpeg = photos::prepare_photo(&bytes)?;
    drop(bytes);
    store.add_photo(id, &jpeg, now)?;
    store.get(id)?.ok_or_else(|| GONE.to_string())
}

/// Removes the photo and returns the entry. A photo that isn't this entry's
/// (or is already gone) is an error, so the page doesn't take a no-op for
/// a removal.
fn remove_photo_from(
    store: &JournalStore,
    id: i64,
    photo_id: i64,
    now: i64,
) -> Result<PrintEntry, String> {
    if !store.remove_photo(id, photo_id, now)? {
        return Err(GONE.to_string());
    }
    store.get(id)?.ok_or_else(|| GONE.to_string())
}

/// A photo of the entry, or its plate thumbnail without `photo_id`, as a
/// `data:` URL. The path comes only from the store's own ids.
fn image_for(store: &JournalStore, id: i64, photo_id: Option<i64>) -> Result<String, String> {
    let url = match photo_id {
        Some(photo) => store
            .photo_path(id, photo)?
            .and_then(|p| image_data_url(&p)),
        None => image_data_url_within(&store.thumbnail_path(id), photos::MAX_THUMBNAIL_BYTES),
    };
    url.ok_or_else(|| PHOTO_UNAVAILABLE.to_string())
}

#[tauri::command]
pub async fn journal_list(
    app: AppHandle,
    filter: JournalFilter,
) -> Result<Vec<PrintEntry>, String> {
    with_store(&app, move |s| s.list(&filter)).await
}

#[tauri::command]
pub async fn journal_get(app: AppHandle, id: i64) -> Result<Option<PrintEntry>, String> {
    with_store(&app, move |s| s.get(id)).await
}

#[tauri::command]
pub async fn journal_update(
    app: AppHandle,
    id: i64,
    update: EntryUpdate,
) -> Result<PrintEntry, String> {
    let update = update.cleaned(&known_tags());
    let entry = with_store(&app, move |s| {
        s.update(id, &update, now())?
            .ok_or_else(|| GONE.to_string())
    })
    .await?;
    changed(&app);
    Ok(entry)
}

/// The page reads the file and sends its bytes: the webview never sees a
/// file path.
#[tauri::command]
pub async fn journal_add_photo(
    app: AppHandle,
    id: i64,
    data_base64: String,
) -> Result<PrintEntry, String> {
    // A decoder that panics on a bad file is a photo that couldn't be saved,
    // not an unreadable journal.
    let entry = run_blocking(paths(&app)?, PHOTO_FAILED, move |s| {
        add_photo_to(s, id, data_base64, now())
    })
    .await?;
    changed(&app);
    Ok(entry)
}

#[tauri::command]
pub async fn journal_remove_photo(
    app: AppHandle,
    id: i64,
    photo_id: i64,
) -> Result<PrintEntry, String> {
    let entry = with_store(&app, move |s| remove_photo_from(s, id, photo_id, now())).await?;
    changed(&app);
    Ok(entry)
}

#[tauri::command]
pub async fn journal_delete(app: AppHandle, id: i64) -> Result<(), String> {
    with_store(&app, move |s| s.delete(id).map(|_| ())).await?;
    changed(&app);
    Ok(())
}

/// This calendar month, in the computer's time zone.
#[tauri::command]
pub async fn journal_month_summary(app: AppHandle) -> Result<MonthSummary, String> {
    let (from, to) = month_bounds(&chrono::Local::now());
    with_store(&app, move |s| s.month_summary(from, to)).await
}

#[tauri::command]
pub async fn journal_last(app: AppHandle) -> Result<Option<PrintEntry>, String> {
    with_store(&app, |s| s.last()).await
}

/// A photo of the entry, or its plate thumbnail without `photo_id`, as a
/// `data:` URL. A missing or unreadable file is the error
/// [`PHOTO_UNAVAILABLE`]: the page shows a placeholder.
#[tauri::command]
pub async fn journal_image(
    app: AppHandle,
    id: i64,
    photo_id: Option<i64>,
) -> Result<String, String> {
    with_store(&app, move |s| image_for(s, id, photo_id)).await
}

/// Records that analysis `session_id` was made from journal entry
/// `journal_id`. A failure is logged: the analysis itself has succeeded.
pub(crate) async fn link_analysis(app: &AppHandle, journal_id: i64, session_id: i64) {
    let linked = with_store(app, move |s| s.link_analysis(journal_id, session_id, now())).await;
    match linked {
        Ok(()) => changed(app),
        Err(e) => {
            tracing::warn!("analysis {session_id} not linked to journal entry {journal_id}: {e}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use base64::Engine;
    use image::{DynamicImage, ImageFormat};

    use super::*;
    use crate::journal::types::{NewPrint, SliceSnapshot};

    fn open(dir: &tempfile::TempDir) -> JournalStore {
        JournalStore::open(&JournalPaths::in_dir(dir.path())).unwrap()
    }

    fn start(s: &JournalStore, key: &str) -> i64 {
        s.start(
            &NewPrint {
                job_key: key.into(),
                name: key.into(),
                gcode_file: None,
                plate: None,
                printer_model: None,
                started_at: 100,
                start_approx: false,
            },
            100,
        )
        .unwrap()
    }

    fn png_b64(width: u32, height: u32) -> String {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(out.into_inner())
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(png_b64(width, height))
            .unwrap()
    }

    #[test]
    fn issue_tags_are_analyzes_defect_ids() {
        assert_eq!(
            known_tags(),
            [
                "elephants_foot",
                "layer_adhesion",
                "over_extrusion",
                "stringing",
                "under_extrusion",
                "warping",
                "z_banding"
            ]
        );
    }

    #[test]
    fn an_added_photo_is_stored_as_a_jpeg_and_read_back_as_a_data_url() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        let entry = add_photo_to(&s, id, png_b64(3000, 100), 200).unwrap();
        assert_eq!(entry.photos.len(), 1);
        let url = image_for(&s, id, Some(entry.photos[0].id)).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,/9j/"));
        let on_disk = dir
            .path()
            .join("print-journal/photos")
            .join(id.to_string())
            .join(&entry.photos[0].file_name);
        let back = image::open(on_disk).unwrap();
        assert_eq!(back.width(), photos::MAX_EDGE);
    }

    #[test]
    fn a_refused_photo_gives_the_exact_sentence_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        let not_an_image = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert_eq!(
            add_photo_to(&s, id, not_an_image, 200),
            Err(
                "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image."
                    .to_string()
            )
        );
        let too_big = "A".repeat(photos::MAX_PHOTO_BYTES.div_ceil(3) * 4 + 4);
        assert_eq!(
            add_photo_to(&s, id, too_big, 200),
            Err("That photo is over 25 MB. Choose a smaller one.".to_string())
        );
        assert!(s.get(id).unwrap().unwrap().photos.is_empty());
    }

    #[test]
    fn a_photo_for_an_entry_that_is_gone_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        assert_eq!(
            add_photo_to(&s, 99, png_b64(10, 10), 200),
            Err(GONE.to_string())
        );
    }

    #[test]
    fn missing_or_foreign_images_are_unavailable_never_a_path_or_os_error() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let a = start(&s, "a");
        let b = start(&s, "b");
        let photo = add_photo_to(&s, a, png_b64(10, 10), 200).unwrap().photos[0].id;
        // No thumbnail was stored, and an unknown photo id.
        assert_eq!(image_for(&s, a, None), Err(PHOTO_UNAVAILABLE.to_string()));
        assert_eq!(
            image_for(&s, a, Some(999)),
            Err(PHOTO_UNAVAILABLE.to_string())
        );
        // Another entry's photo is not reachable through this entry.
        assert_eq!(
            image_for(&s, b, Some(photo)),
            Err(PHOTO_UNAVAILABLE.to_string())
        );
        // A photo whose file was deleted or corrupted.
        let file = s.photo_path(a, photo).unwrap().unwrap();
        std::fs::write(&file, b"garbage").unwrap();
        assert_eq!(
            image_for(&s, a, Some(photo)),
            Err(PHOTO_UNAVAILABLE.to_string())
        );
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            image_for(&s, a, Some(photo)),
            Err(PHOTO_UNAVAILABLE.to_string())
        );
    }

    #[test]
    fn the_plate_thumbnail_is_served_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        let thumb = s.thumbnail_path(id);
        std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
        std::fs::write(
            &thumb,
            base64::engine::general_purpose::STANDARD
                .decode(png_b64(4, 4))
                .unwrap(),
        )
        .unwrap();
        assert!(image_for(&s, id, None)
            .unwrap()
            .starts_with("data:image/png;base64,iVBOR"));
    }

    #[test]
    fn linking_an_analysis_to_a_deleted_entry_fails_without_touching_others() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        assert_eq!(s.link_analysis(99, 7, 300), Err(GONE.to_string()));
        s.link_analysis(id, 7, 300).unwrap();
        assert_eq!(s.get(id).unwrap().unwrap().analyses, vec![7]);
    }

    #[test]
    fn removing_a_photo_that_is_not_the_entrys_is_an_error_and_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let a = start(&s, "a");
        let b = start(&s, "b");
        let photo = add_photo_to(&s, a, png_b64(10, 10), 200).unwrap().photos[0].id;
        assert_eq!(remove_photo_from(&s, b, photo, 300), Err(GONE.to_string()));
        assert_eq!(remove_photo_from(&s, a, 999, 300), Err(GONE.to_string()));
        assert_eq!(s.get(a).unwrap().unwrap().photos.len(), 1);
        let entry = remove_photo_from(&s, a, photo, 300).unwrap();
        assert!(entry.photos.is_empty());
        // Removing it a second time is an error too.
        assert_eq!(remove_photo_from(&s, a, photo, 400), Err(GONE.to_string()));
    }

    #[test]
    fn a_poisoned_file_name_in_the_database_never_leaves_the_photo_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        // A valid JPEG outside the photo folder, where a traversal would land.
        let outside = dir.path().join("print-journal").join("secret.jpg");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, photos::prepare_photo(&png_bytes(8, 8)).unwrap()).unwrap();
        let absolute = outside.to_string_lossy().to_string();
        let db = rusqlite::Connection::open(dir.path().join("print_journal.db")).unwrap();
        let mut ids = Vec::new();
        for name in [
            "../../secret.jpg",
            "..",
            "a/b.jpg",
            "..\\secret.jpg",
            absolute.as_str(),
        ] {
            db.execute(
                "INSERT INTO print_photos (print_id, file_name, added_at) VALUES (?1, ?2, 1)",
                rusqlite::params![id, name],
            )
            .unwrap();
            ids.push(db.last_insert_rowid());
        }
        for photo in ids {
            assert_eq!(
                image_for(&s, id, Some(photo)),
                Err(PHOTO_UNAVAILABLE.to_string())
            );
        }
    }

    #[test]
    fn a_panic_in_the_blocking_task_gives_the_sentence_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let out: Result<(), String> =
            tauri::async_runtime::block_on(run_blocking(paths.clone(), PHOTO_FAILED, |_| {
                panic!("a decoder panicked")
            }));
        assert_eq!(out, Err("The photo couldn't be saved.".to_string()));
        let out: Result<(), String> =
            tauri::async_runtime::block_on(run_blocking(paths, READ_FAILED, |_| panic!("again")));
        assert_eq!(out, Err("The print journal couldn't be read.".to_string()));
    }

    #[test]
    fn a_thumbnail_the_journal_saves_can_be_shown() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        let slice = SliceSnapshot {
            model_name: "cube.stl".into(),
            model_path: "/m/cube.stl".into(),
            printer: String::new(),
            process: String::new(),
            filament: String::new(),
            plate: 1,
            time_seconds: 1,
            weight_g: 1.0,
            filaments: Vec::new(),
            has_thumbnail: false,
        };
        let mut largest = png_bytes(4, 4);
        largest.resize(photos::MAX_THUMBNAIL_BYTES as usize, 0);
        s.set_slice(id, &slice, Some(&largest), 2).unwrap();
        assert!(s.get(id).unwrap().unwrap().slice.unwrap().has_thumbnail);
        assert!(image_for(&s, id, None)
            .unwrap()
            .starts_with("data:image/png;base64,"));
        largest.push(0);
        s.set_slice(id, &slice, Some(&largest), 3).unwrap();
        assert!(
            !s.get(id).unwrap().unwrap().slice.unwrap().has_thumbnail,
            "one too large isn't saved to fail later"
        );
    }

    #[test]
    fn an_oversized_thumbnail_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = start(&s, "a");
        let thumb = s.thumbnail_path(id);
        std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
        let mut big = png_bytes(4, 4);
        big.resize(photos::MAX_THUMBNAIL_BYTES as usize + 1, 0);
        std::fs::write(&thumb, big).unwrap();
        assert_eq!(image_for(&s, id, None), Err(PHOTO_UNAVAILABLE.to_string()));
    }
}
