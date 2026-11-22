pub mod azauth;
pub mod commands;
pub mod config;
pub mod downloader;
pub mod error;
pub mod forge;
pub mod install;
pub mod java;
pub mod launch;
pub mod logs;

use commands::AppState;
use config::PanelClient;
use directories::{BaseDirs, ProjectDirs};
use logs::{LogLevel, LogStore};
use std::path::PathBuf;

/// Résout `(cache_dir, session_dir, data_root_dir)` en conservant le chemin historique `%APPDATA%\.Nexus` de Launcher-v3.
fn resolve_directories() -> (PathBuf, PathBuf, PathBuf) {
    let data_root = if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .ok()
            .or_else(|| BaseDirs::new().map(|b| b.data_dir().to_path_buf()))
    } else if cfg!(target_os = "macos") {
        BaseDirs::new().map(|b| b.data_dir().to_path_buf())
    } else {
        BaseDirs::new().map(|b| b.home_dir().to_path_buf())
    }
    .unwrap_or_else(|| std::env::temp_dir().join("geoventure-launcher"));

    if let Some(proj) = ProjectDirs::from("fr", "geoventure", "GeoventureLauncher") {
        let cache = proj.cache_dir().to_path_buf();
        let session = proj.data_local_dir().to_path_buf();
        return (cache, session, data_root);
    }

    let fallback = std::env::temp_dir().join("geoventure-launcher");
    (fallback.join("cache"), fallback.join("session"), data_root)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let panel_client =
        PanelClient::default_panel().expect("Initialisation du client HTTP Geoventure");
    let (cache_dir, session_dir, data_root_dir) = resolve_directories();
    let logs = LogStore::default();
    logs.push(
        LogLevel::Info,
        "core",
        "Geoventure Launcher Rust (Tauri v2) initialisé — Azuriom + timeouts stricts actifs.",
    );

    let state = AppState {
        panel_client,
        cache_dir,
        session_dir,
        data_root_dir,
        logs,
        launching: Default::default(),
        game_running: Default::default(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::fetch_panel_config,
            commands::azuriom_login,
            commands::azuriom_restore_session,
            commands::azuriom_logout,
            commands::azuriom_verify_session,
            commands::fetch_modpack_manifest,
            commands::verify_modpack_files,
            commands::sync_modpack_files,
            commands::detect_jdks,
            commands::resolve_preferred_java,
            commands::get_system_overview,
            commands::preview_forge_launch_args,
            commands::launch_instance,
            commands::launch_forge_game,
            commands::get_logs,
            commands::clear_logs,
            commands::export_logs_text,
            commands::toggle_devtools,
            commands::panel_get,
            commands::open_url,
            commands::open_game_dir,
            commands::repair_instance,
            commands::check_launcher_update,
            commands::optional_mods_state,
            commands::game_status,
            commands::set_optional_mod,
            commands::install_launcher_update,
        ])
        .run(tauri::generate_context!())
        .expect("Erreur lors du démarrage de Geoventure Launcher");
}
