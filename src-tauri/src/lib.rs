#![recursion_limit = "256"]

pub mod agent;
pub mod analyzer;
mod commands;
pub mod diagnostics;
pub mod history;
pub mod mapper;
pub mod model_catalog;
mod process_command;
pub mod profile;
pub mod scraper;
pub mod slicer;
pub mod stl_watcher;
pub mod str_utils;

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(stl_watcher::StlWatcherState::new())
        .invoke_handler(tauri::generate_handler![
            commands::keychain::set_api_key,
            commands::keychain::get_api_key,
            commands::keychain::delete_api_key,
            commands::config::get_preference,
            commands::config::set_preference,
            commands::config::get_feature_flags,
            commands::config::check_setup_complete,
            commands::config::reset_to_clean_install,
            commands::health::run_health_check,
            commands::health::search_bambu_studio_config,
            commands::health::validate_bambu_studio_path,
            commands::health::pick_config_folder,
            commands::diagnostics::run_diagnostics,
            commands::models::list_models,
            commands::models::validate_model,
            commands::models::get_ai_capabilities,
            commands::models::refresh_model_catalog,
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
            commands::profile::search_base_profiles,
            commands::profile::refresh_base_profile_index,
            commands::profile::list_target_printer_options,
            commands::scraper::search_filament,
            commands::scraper::get_cached_filament,
            commands::scraper::clear_filament_cache,
            commands::scraper::extract_specs_from_url,
            commands::scraper::get_catalog_status,
            commands::scraper::refresh_catalog,
            commands::scraper::search_catalog,
            commands::scraper::fetch_filament_from_catalog,
            commands::scraper::generate_specs_from_ai,
            commands::scraper::tune_specs_for_nozzle,
            commands::analyzer::analyze_print,
            commands::analyzer::apply_recommendations,
            commands::history::list_history_sessions,
            commands::history::get_history_session,
            commands::history::revert_to_backup,
            commands::launcher::detect_bambu_studio_path,
            commands::launcher::launch_bambu_studio,
            commands::launcher::open_external_url,
            commands::batch::list_catalog_brands,
            commands::batch::batch_generate_brand,
            commands::slicer::slicer_status,
            commands::slicer::slicer_presets,
            commands::slicer::slicer_get_settings,
            commands::slicer::slicer_set_settings,
            commands::slicer::slicer_slice,
            commands::slicer::slicer_cancel,
            commands::slicer::slicer_jobs,
            commands::slicer::slicer_thumbnail,
            commands::slicer::slicer_open_in_bambu_studio,
            commands::slicer::slicer_clear_cache,
            commands::slicer::slicer_pick_model,
            commands::slicer::slicer_stage_model,
            commands::stl_bridge::set_stl_watch_dir,
            commands::stl_bridge::get_stl_watch_dir,
            commands::stl_bridge::list_received_stls,
            commands::stl_bridge::clear_received_stls,
            commands::stl_bridge::dismiss_stl,
            commands::updater::get_app_version,
            commands::updater::check_for_updates,
            commands::agent::agent_readiness,
            commands::agent::agent_models,
            commands::agent::agent_login,
            commands::agent::agent_start,
            commands::agent::agent_open,
            commands::agent::agent_send,
            commands::agent::agent_interrupt,
            commands::agent::agent_answer,
            commands::agent::agent_rewind,
            commands::agent::agent_rewind_preview,
            commands::agent::agent_list_sessions,
            commands::agent::agent_delete_session,
            commands::agent::agent_set_app_state,
            commands::agent::agent_stage_image,
            commands::agent::agent_get_settings,
            commands::agent::agent_set_settings,
        ])
        .setup(|app| {
            // Apply the user-configured Bambu Studio config folder before any
            // command can run, so path resolution honors it from the start.
            commands::config::sync_bambu_studio_path_override(app.handle());

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

            // -- Slicing with Bambu Studio -----------------------------------
            commands::slicer::start(app.handle());
            commands::slicer::install_auto_slice(app.handle());

            // -- Agent backends --------------------------------------------
            {
                use std::sync::Arc;
                use tauri::Emitter;

                let (tx, _) = tokio::sync::broadcast::channel::<agent::types::AgentEvent>(1024);
                // Subscribe the webview forwarder before anything can publish.
                let mut forward_rx = tx.subscribe();
                let asks = Arc::new(agent::asks::AskBroker::new(tx.clone()));
                let host = Arc::new(agent::host::TauriToolHost::new(app.handle().clone()));
                let codex: Arc<dyn agent::backend::AgentBackend> =
                    Arc::new(agent::codex::CodexBackend::new(
                        Arc::new(agent::codex::ProcessSpawner),
                        tx.clone(),
                        asks.clone(),
                    ));
                let claude = Arc::new(agent::claude::ClaudeBackend::new(
                    Arc::new(agent::claude::ProcessSpawner),
                    Arc::new(agent::claude::KeychainKeys),
                    tx.clone(),
                    asks.clone(),
                ));
                // A broken agent setup (unreadable app-data dir, corrupt
                // session DB) must not stop the rest of the app launching:
                // agent commands report it instead.
                let service = app
                    .path()
                    .app_data_dir()
                    .map_err(|e| e.to_string())
                    .and_then(|app_data| {
                        agent::service::AgentService::new(
                            vec![
                                codex,
                                claude.clone() as Arc<dyn agent::backend::AgentBackend>,
                            ],
                            host.clone(),
                            asks,
                            tx,
                            app_data,
                        )
                    });
                match &service {
                    Ok(svc) => {
                        commands::agent::apply_stored_settings(app.handle(), svc, &claude);
                        tauri::async_runtime::spawn(svc.validator_loop());
                    }
                    Err(e) => tracing::error!("agent service failed to start: {e}"),
                }

                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::sync::broadcast::error::RecvError;
                    loop {
                        match forward_rx.recv().await {
                            Ok(ev) => {
                                let _ = handle.emit("agent://event", &ev);
                            }
                            Err(RecvError::Lagged(n)) => {
                                tracing::warn!("agent event forwarder lagged by {n}")
                            }
                            Err(RecvError::Closed) => break,
                        }
                    }
                });
                app.manage(commands::agent::AgentSlot::new(service));
                app.manage(host);
                app.manage(claude);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // No Bambu Studio CLI may outlive BambuMate.
                commands::slicer::stop(app);
            }
        });
}
