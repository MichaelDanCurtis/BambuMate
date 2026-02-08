#![recursion_limit = "256"]

pub mod analyzer;
mod commands;
mod error;
pub mod history;
pub mod mapper;
pub mod profile;
pub mod scraper;
pub mod stl_watcher;

pub use history::{AppliedChange, RefinementHistory, SessionDetail, SessionSummary};

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(stl_watcher::StlWatcherState::new())
        .invoke_handler(tauri::generate_handler![
            commands::keychain::set_api_key,
            commands::keychain::get_api_key,
            commands::keychain::delete_api_key,
            commands::config::get_preference,
            commands::config::set_preference,
            commands::health::run_health_check,
            commands::models::list_models,
            commands::profile::list_profiles,
            commands::profile::list_system_profiles,
            commands::profile::read_profile_command,
            commands::profile::get_system_profile_count,
            commands::profile::generate_profile_from_specs,
            commands::profile::install_generated_profile,
            commands::profile::delete_profile,
            commands::profile::update_profile_field,
            commands::profile::duplicate_profile,
            commands::profile::extract_specs_from_profile,
            commands::profile::save_profile_specs,
            commands::profile::compare_profiles,
            commands::scraper::search_filament,
            commands::scraper::get_cached_filament,
            commands::scraper::clear_filament_cache,
            commands::scraper::extract_specs_from_url,
            commands::scraper::get_catalog_status,
            commands::scraper::refresh_catalog,
            commands::scraper::search_catalog,
            commands::scraper::fetch_filament_from_catalog,
            commands::scraper::generate_specs_from_ai,
            commands::analyzer::analyze_print,
            commands::analyzer::apply_recommendations,
            commands::history::list_history_sessions,
            commands::history::get_history_session,
            commands::history::revert_to_backup,
            commands::launcher::detect_bambu_studio_path,
            commands::launcher::launch_bambu_studio,
            commands::batch::list_catalog_brands,
            commands::batch::batch_generate_brand,
            commands::stl_bridge::set_stl_watch_dir,
            commands::stl_bridge::get_stl_watch_dir,
            commands::stl_bridge::list_received_stls,
            commands::stl_bridge::clear_received_stls,
            commands::stl_bridge::dismiss_stl,
        ])
        .setup(|app| {
            // Restore STL watch directory from preferences
            use tauri::Manager;
            use tauri_plugin_store::StoreExt;
            if let Ok(store) = app.store("preferences.json") {
                if let Some(dir) = store
                    .get("stl_watch_dir")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty())
                {
                    let state = app.state::<stl_watcher::StlWatcherState>();
                    if let Err(e) = state.start_watching(&dir) {
                        tracing::warn!("Failed to restore STL watcher for {}: {}", dir, e);
                    }
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
