mod commands;
mod error;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
