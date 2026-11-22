pub mod azauth;
pub mod commands;
pub mod config;
pub mod downloader;
pub mod error;
pub mod extras;
pub mod forge;
pub mod install;
pub mod java;
pub mod launch;
pub mod logs;
pub mod secure;
pub mod crash;
pub mod gpu;
pub mod gameopts;
pub mod discord;
pub mod media;

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

    // Adresse du panel choisie dans les réglages (sinon celle par défaut).
    let panel_client = std::fs::read_to_string(session_dir.join(commands::PANEL_URL_FILE))
        .ok()
        .and_then(|u| PanelClient::new(u.trim(), config::HttpTimeouts::default()).ok())
        .unwrap_or(panel_client);
    // Copie du journal sur le disque : <données du launcher>/logs/launcher.log
    let log_dir = session_dir.join("logs");
    match logs.attach_file(&log_dir) {
        Ok(p) => {
            logs.push(LogLevel::Info, "core", format!("Journal enregistré dans {}", p.display()));
        }
        Err(e) => {
            logs.push(LogLevel::Warn, "core", format!("launcher.log impossible : {e}"));
        }
    }
    logs.push(LogLevel::Info, "core", format!("Version {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
    let state = AppState {
        panel_client: std::sync::RwLock::new(panel_client),
        cache_dir,
        session_dir,
        data_root_dir,
        logs,
        launching: Default::default(),
        game_running: Default::default(),
        game_kill: Default::default(),
        kill_requested: Default::default(),
        log_dir,
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
            commands::fetch_news,
            commands::get_panel_url,
            commands::set_panel_url,
            commands::collect_diagnostic,
            commands::save_diagnostic,
            commands::send_diagnostic,
            commands::set_optional_mod,
            commands::install_launcher_update,
            commands::kill_game,
            commands::read_game_log,
            commands::detect_gpus,
            commands::restore_game_options,
            commands::open_launcher_logs,
            commands::discord_presence,
            commands::community_mods_installed,
            commands::install_community_mod,
            commands::uninstall_community_mod,
            commands::list_content,
            commands::delete_content,
            commands::open_content,
            commands::set_shader_pack,
        ])
        .run(tauri::generate_context!())
        .expect("Erreur lors du démarrage de Geoventure Launcher");
}
