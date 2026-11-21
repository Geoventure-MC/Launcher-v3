use crate::azauth::{
    clear_account_session, load_account_session, save_account_session, AzAccount, AzAuthClient,
    AzAuthResponse,
};
use crate::config::{ConfigFetchResult, HttpTimeouts, PanelClient};
use crate::downloader::{
    diff_verify_manifest, sync_modpack_parallel, DiffScanReport, ManifestEntry, SyncReport,
    DEFAULT_CONCURRENCY,
};
use crate::error::LauncherResult;
use crate::forge::{resolve_installed_forge_plan, ForgeResolveRequest};
use crate::java::{detect_installed_jdks, resolve_java17, JavaInstallation};
use crate::launch::{
    build_forge_1_20_1_args, build_performance_profile, recommend_profile, resolve_game_directory,
    spawn_forge_process, spawn_resolved_forge, ForgeLaunchConfig, PerformancePlan,
    PerformanceProfileId,
};
use crate::logs::{LogEntry, LogLevel, LogStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use sysinfo::System;
use tauri::{AppHandle, Emitter, State, WebviewWindow};

pub struct AppState {
    pub panel_client: PanelClient,
    pub cache_dir: PathBuf,
    pub session_dir: PathBuf,
    pub data_root_dir: PathBuf,
    pub logs: LogStore,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemOverview {
    pub total_ram_gb: f64,
    pub cpu_cores: usize,
    pub recommended_profile: PerformanceProfileId,
    pub plan: PerformancePlan,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoredSession {
    pub account: Option<AzAccount>,
    pub verified_online: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchInstanceRequest {
    pub data_directory: String,
    pub instance: Option<String>,
    pub channel: Option<String>,
    pub mc_version: String,
    pub loader_build: String,
    pub custom_java_path: Option<String>,
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub ram_min_gb: u32,
    pub ram_max_gb: u32,
    #[serde(default)]
    pub extra_jvm_args: Vec<String>,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

fn emit_log(app: &AppHandle, logs: &LogStore, level: LogLevel, source: &str, message: impl Into<String>) {
    let entry = logs.push(level, source, message);
    let _ = app.emit("launcher://log", entry);
}

#[tauri::command]
pub async fn fetch_panel_config(
    app: AppHandle,
    state: State<'_, AppState>,
    instance: Option<String>,
    channel: Option<String>,
) -> LauncherResult<ConfigFetchResult> {
    let ch = channel.as_deref().unwrap_or("stable");
    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "panel",
        format!(
            "Récupération de la configuration ({}, canal={ch})…",
            instance.as_deref().unwrap_or("default")
        ),
    );
    let res = state
        .panel_client
        .fetch_config_with_cache(instance.as_deref(), ch, &state.cache_dir)
        .await?;
    if res.offline {
        emit_log(
            &app,
            &state.logs,
            LogLevel::Warn,
            "panel",
            res.warning
                .clone()
                .unwrap_or_else(|| "Mode hors-ligne actif (cache local).".into()),
        );
    } else {
        emit_log(
            &app,
            &state.logs,
            LogLevel::Info,
            "panel",
            format!(
                "Configuration chargée (MC {}, Forge {}, dossier .{}).",
                res.config.game_version, res.config.loader.build, res.config.data_directory
            ),
        );
    }
    Ok(res)
}

#[tauri::command]
pub async fn azuriom_login(
    app: AppHandle,
    state: State<'_, AppState>,
    site_url: String,
    email: String,
    password: String,
    code_2fa: Option<String>,
) -> LauncherResult<AzAuthResponse> {
    emit_log(&app, &state.logs, LogLevel::Info, "azauth", "Connexion Azuriom en cours…");
    let client = AzAuthClient::new(&site_url, HttpTimeouts::default())?;
    let res = client
        .authenticate(&email, &password, code_2fa.as_deref())
        .await?;
    match &res {
        AzAuthResponse::Authenticated { account } => {
            let _ = save_account_session(&state.session_dir, account).await;
            emit_log(
                &app,
                &state.logs,
                LogLevel::Info,
                "azauth",
                format!("Connecté en tant que {} (session sauvegardée).", account.username),
            );
        }
        AzAuthResponse::Requires2fa => emit_log(
            &app,
            &state.logs,
            LogLevel::Info,
            "azauth",
            "Code 2FA Azuriom requis.",
        ),
        AzAuthResponse::Rejected { reason, .. } => emit_log(
            &app,
            &state.logs,
            LogLevel::Warn,
            "azauth",
            format!("Connexion Azuriom refusée ({reason})."),
        ),
    }
    Ok(res)
}

#[tauri::command]
pub async fn azuriom_restore_session(
    app: AppHandle,
    state: State<'_, AppState>,
    fallback_site_url: Option<String>,
) -> LauncherResult<RestoredSession> {
    let Some(saved) = load_account_session(&state.session_dir).await? else {
        return Ok(RestoredSession {
            account: None,
            verified_online: false,
        });
    };

    let site = if !saved.site_url.is_empty() {
        saved.site_url.clone()
    } else {
        fallback_site_url.unwrap_or_else(|| "https://geoventure.fr/".to_string())
    };

    if let Ok(client) = AzAuthClient::new(&site, HttpTimeouts::default()) {
        match client.verify(&saved.access_token).await {
            Ok(fresh) => {
                let _ = save_account_session(&state.session_dir, &fresh).await;
                emit_log(
                    &app,
                    &state.logs,
                    LogLevel::Info,
                    "azauth",
                    format!("Session Azuriom vérifiée pour {}.", fresh.username),
                );
                return Ok(RestoredSession {
                    account: Some(fresh),
                    verified_online: true,
                });
            }
            Err(crate::error::LauncherError::HttpStatus { status: 401 | 403 | 422, .. }) => {
                let _ = clear_account_session(&state.session_dir).await;
                emit_log(
                    &app,
                    &state.logs,
                    LogLevel::Warn,
                    "azauth",
                    "Session Azuriom expirée : reconnexion requise.",
                );
                return Ok(RestoredSession {
                    account: None,
                    verified_online: false,
                });
            }
            Err(_) => {
                emit_log(
                    &app,
                    &state.logs,
                    LogLevel::Warn,
                    "azauth",
                    format!("Site Azuriom injoignable : session locale conservée ({}).", saved.username),
                );
            }
        }
    }

    Ok(RestoredSession {
        account: Some(saved),
        verified_online: false,
    })
}

#[tauri::command]
pub async fn azuriom_logout(app: AppHandle, state: State<'_, AppState>) -> LauncherResult<()> {
    if let Some(saved) = load_account_session(&state.session_dir).await? {
        if let Ok(client) = AzAuthClient::new(&saved.site_url, HttpTimeouts::default()) {
            let _ = client.logout(&saved.access_token).await;
        }
    }
    clear_account_session(&state.session_dir).await?;
    emit_log(&app, &state.logs, LogLevel::Info, "azauth", "Déconnexion Azuriom effectuée.");
    Ok(())
}

#[tauri::command]
pub async fn azuriom_verify_session(
    site_url: String,
    access_token: String,
) -> LauncherResult<AzAccount> {
    let client = AzAuthClient::new(&site_url, HttpTimeouts::default())?;
    client.verify(&access_token).await
}

#[tauri::command]
pub async fn fetch_modpack_manifest(
    state: State<'_, AppState>,
    instance: Option<String>,
    channel: Option<String>,
) -> LauncherResult<Vec<ManifestEntry>> {
    let ch = channel.as_deref().unwrap_or("stable");
    state
        .panel_client
        .fetch_modpack_manifest(instance.as_deref(), ch)
        .await
}

#[tauri::command]
pub async fn verify_modpack_files(
    state: State<'_, AppState>,
    manifest: Vec<ManifestEntry>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    ignored: Option<Vec<String>>,
) -> LauncherResult<DiffScanReport> {
    let ch = channel.as_deref().unwrap_or("stable");
    let game_dir = resolve_game_directory(
        &state.data_root_dir,
        &data_directory,
        instance.as_deref(),
        "geoventure",
        ch,
    );
    diff_verify_manifest(&manifest, &game_dir, &ignored.unwrap_or_default()).await
}

#[tauri::command]
pub async fn sync_modpack_files(
    app: AppHandle,
    state: State<'_, AppState>,
    manifest: Vec<ManifestEntry>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    ignored: Option<Vec<String>>,
    concurrency: Option<usize>,
) -> LauncherResult<SyncReport> {
    let ch = channel.as_deref().unwrap_or("stable");
    let game_dir = resolve_game_directory(
        &state.data_root_dir,
        &data_directory,
        instance.as_deref(),
        "geoventure",
        ch,
    );
    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "downloader",
        format!("Synchronisation du modpack dans {}…", game_dir.display()),
    );

    let app_for_cb = app.clone();
    let report = sync_modpack_parallel(
        state.panel_client.http_client().clone(),
        &manifest,
        &game_dir,
        &ignored.unwrap_or_default(),
        concurrency.unwrap_or(DEFAULT_CONCURRENCY),
        move |progress| {
            let _ = app_for_cb.emit("launcher://download-progress", progress);
        },
    )
    .await?;

    emit_log(
        &app,
        &state.logs,
        if report.failed.is_empty() {
            LogLevel::Info
        } else {
            LogLevel::Warn
        },
        "downloader",
        format!(
            "Sync terminée : {} valides, {} téléchargés, {} erreurs.",
            report.valid_count,
            report.downloaded_count,
            report.failed.len()
        ),
    );
    Ok(report)
}

#[tauri::command]
pub fn detect_jdks(state: State<'_, AppState>, data_directory: Option<String>) -> Vec<JavaInstallation> {
    let game_dir = data_directory.map(|d| {
        resolve_game_directory(&state.data_root_dir, &d, None, "geoventure", "stable")
    });
    detect_installed_jdks(game_dir.as_deref())
}

#[tauri::command]
pub fn resolve_preferred_java(
    state: State<'_, AppState>,
    custom_path: Option<String>,
    data_directory: Option<String>,
) -> LauncherResult<JavaInstallation> {
    let game_dir = data_directory.map(|d| {
        resolve_game_directory(&state.data_root_dir, &d, None, "geoventure", "stable")
    });
    resolve_java17(custom_path.as_deref(), game_dir.as_deref())
}

#[tauri::command]
pub fn get_system_overview(profile: Option<PerformanceProfileId>) -> SystemOverview {
    let mut sys = System::new_all();
    sys.refresh_memory();
    let total_ram_gb = (sys.total_memory() as f64 / 1_073_741_824.0 * 10.0).trunc() / 10.0;
    let cpu_cores = sys.cpus().len().max(1);
    let recommended = recommend_profile(total_ram_gb, cpu_cores);
    let chosen = profile.unwrap_or(recommended);
    let plan = build_performance_profile(chosen, total_ram_gb, cpu_cores);

    SystemOverview {
        total_ram_gb,
        cpu_cores,
        recommended_profile: recommended,
        plan,
    }
}

#[tauri::command]
pub fn preview_forge_launch_args(config: ForgeLaunchConfig) -> LauncherResult<Vec<String>> {
    build_forge_1_20_1_args(&config)
}

#[tauri::command]
pub async fn launch_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    req: LaunchInstanceRequest,
) -> LauncherResult<u32> {
    let ch = req.channel.as_deref().unwrap_or("stable");
    let game_dir = resolve_game_directory(
        &state.data_root_dir,
        &req.data_directory,
        req.instance.as_deref(),
        "geoventure",
        ch,
    );
    let java = resolve_java17(req.custom_java_path.as_deref(), Some(&game_dir))?;

    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "launch",
        format!(
            "Préparation Forge {} dans {} avec {}…",
            req.loader_build,
            game_dir.display(),
            java.label
        ),
    );

    let plan = resolve_installed_forge_plan(&ForgeResolveRequest {
        game_dir: game_dir.clone(),
        mc_version: req.mc_version.clone(),
        loader_build: req.loader_build.clone(),
        username: req.username.clone(),
        uuid: req.uuid.clone(),
        access_token: req.access_token.clone(),
        ram_min_gb: req.ram_min_gb,
        ram_max_gb: req.ram_max_gb,
        extra_jvm_args: req.extra_jvm_args.clone(),
        server_host: req.server_host.clone(),
        server_port: req.server_port,
        width: req.width,
        height: req.height,
    })?;

    if !plan.missing_libraries.is_empty() {
        emit_log(
            &app,
            &state.logs,
            LogLevel::Warn,
            "launch",
            format!(
                "{} librairies manquantes détectées ({} dans le classpath).",
                plan.missing_libraries.len(),
                plan.classpath.len()
            ),
        );
    }

    let app_for_log = app.clone();
    let pid = spawn_resolved_forge(&java.path, &game_dir, &plan, state.logs.clone(), move |entry| {
        let _ = app_for_log.emit("launcher://log", entry);
    })
    .await?;

    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "launch",
        format!(
            "Minecraft {} démarré (PID {pid}, {} jars).",
            plan.version_id,
            plan.classpath.len()
        ),
    );
    Ok(pid)
}

#[tauri::command]
pub async fn launch_forge_game(
    app: AppHandle,
    state: State<'_, AppState>,
    config: ForgeLaunchConfig,
) -> LauncherResult<u32> {
    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "launch",
        format!(
            "Lancement Forge {} (MC {}) pour {}…",
            config.forge_version, config.mc_version, config.username
        ),
    );
    let app_for_log = app.clone();
    let pid = spawn_forge_process(&config, state.logs.clone(), move |entry| {
        let _ = app_for_log.emit("launcher://log", entry);
    })
    .await?;
    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "launch",
        format!("Processus Minecraft démarré (PID {pid})."),
    );
    Ok(pid)
}

#[tauri::command]
pub fn get_logs(state: State<'_, AppState>) -> Vec<LogEntry> {
    state.logs.list()
}

#[tauri::command]
pub fn clear_logs(state: State<'_, AppState>) {
    state.logs.clear();
}

#[tauri::command]
pub fn export_logs_text(state: State<'_, AppState>) -> String {
    state.logs.export_text()
}

/// Raccourci garanti F12 / bouton console développeur (disponible même en release grâce à la feature `devtools`).
#[tauri::command]
pub fn toggle_devtools(window: WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}
