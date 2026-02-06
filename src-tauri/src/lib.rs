mod commands;
mod error;
pub mod mapper;
pub mod profile;
pub mod scraper;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::keychain::set_api_key,
            commands::keychain::get_api_key,
            commands::keychain::delete_api_key,
            commands::config::get_preference,
            commands::config::set_preference,
            commands::health::run_health_check,
            commands::models::list_models,
            commands::profile::list_profiles,
            commands::profile::read_profile_command,
            commands::profile::get_system_profile_count,
            commands::profile::generate_profile_from_specs,
            commands::profile::install_generated_profile,
            commands::scraper::search_filament,
            commands::scraper::get_cached_filament,
            commands::scraper::clear_filament_cache,
            commands::scraper::extract_specs_from_url,
            commands::scraper::get_catalog_status,
            commands::scraper::refresh_catalog,
            commands::scraper::search_catalog,
            commands::scraper::fetch_filament_from_catalog,
            commands::scraper::generate_specs_from_ai,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
