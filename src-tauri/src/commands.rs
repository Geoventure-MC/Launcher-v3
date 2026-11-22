use crate::azauth::{
    clear_account_session, load_account_session, save_account_session, AzAccount, AzAuthClient,
    AzAuthResponse,
};
use crate::config::{ConfigFetchResult, HttpTimeouts, PanelClient};
use crate::downloader::{
    diff_verify_manifest, sync_modpack_parallel, DiffScanReport, ManifestEntry, SyncReport,
    DEFAULT_CONCURRENCY,
};
use crate::downloader::{cleanup_stale_mods, sync_files, SyncOptions};
use crate::error::{LauncherError, LauncherResult};
use crate::install::{apply_forge_window_settings, InstallProgress, Installer};
use crate::java::probe_java_version;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::forge::{resolve_installed_forge_plan, ForgeResolveRequest};
use crate::java::{detect_installed_jdks, resolve_java17, JavaInstallation};
use crate::launch::{
    build_forge_1_20_1_args, build_performance_profile, recommend_profile, resolve_game_directory,
    spawn_forge_process, spawn_resolved_forge_with_exit, ForgeLaunchConfig, PerformancePlan,
    PerformanceProfileId,
};
use crate::logs::{LogEntry, LogLevel, LogStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use sysinfo::System;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

pub struct AppState {
    pub panel_client: std::sync::RwLock<PanelClient>,
    pub cache_dir: PathBuf,
    pub session_dir: PathBuf,
    pub data_root_dir: PathBuf,
    pub logs: LogStore,
    pub launching: Arc<AtomicBool>,
    pub game_running: Arc<AtomicBool>,
    /// « Fermer le jeu » : réveille la tâche qui attend la fin du processus.
    pub game_kill: Arc<tokio::sync::Notify>,
    /// Fermeture demandée par le joueur : pas d'analyse de plantage.
    pub kill_requested: Arc<AtomicBool>,
    /// Dossier de `launcher.log`.
    pub log_dir: PathBuf,
}

impl AppState {
    /// Client du panel en cours (l'adresse peut être changée dans les réglages).
    pub fn panel(&self) -> PanelClient {
        self.panel_client
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemOverview {
    pub total_ram_gb: f64,
    pub free_ram_gb: f64,
    pub cpu_cores: usize,
    pub cpu_name: String,
    pub os: String,
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
    #[serde(default = "default_loader_type")]
    pub loader_type: String,
    #[serde(default = "default_true")]
    pub loader_enable: bool,
    pub custom_java_path: Option<String>,
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub ram_min_gb: u32,
    pub ram_max_gb: u32,
    #[serde(default)]
    pub extra_jvm_args: Vec<String>,
    #[serde(default)]
    pub ignored: Vec<String>,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `keep` (défaut), `hide` (cachée pendant la partie, réaffichée à la fermeture du jeu) ou `close`.
    #[serde(default)]
    pub after_launch: Option<String>,
    /// Profil Économe avec accord du joueur : baisser aussi les réglages du jeu (`options.txt`).
    #[serde(default)]
    pub light_game_options: bool,
}

fn default_loader_type() -> String {
    "forge".into()
}
fn default_true() -> bool {
    true
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
        .panel()
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
        .panel()
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
        state.panel().http_client().clone(),
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

    let free_ram_gb = (sys.available_memory() as f64 / 1_073_741_824.0 * 10.0).trunc() / 10.0;
    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_default();
    let os = format!(
        "{} ({})",
        System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string()),
        std::env::consts::ARCH
    );
    SystemOverview {
        total_ram_gb,
        free_ram_gb,
        cpu_cores,
        cpu_name,
        os,
        recommended_profile: recommended,
        plan,
    }
}

#[tauri::command]
pub fn preview_forge_launch_args(config: ForgeLaunchConfig) -> LauncherResult<Vec<String>> {
    build_forge_1_20_1_args(&config)
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchResult {
    pub ok: bool,
    pub pid: Option<u32>,
    /// `{ code, message }`, même forme que les erreurs des commandes.
    pub error: Option<serde_json::Value>,
}

/// Bouton « Jouer » : installe ce qui manque (Java 17, Minecraft, Forge, modpack) puis lance le
/// jeu. Fonctionne sur un PC neuf.
///
/// La commande répond TOUT DE SUITE et le travail continue en tâche de fond ; le résultat arrive
/// par l'événement `launcher://launch-result`. Une requête IPC qui dure plusieurs dizaines de
/// secondes peut être renvoyée une seconde fois par le pont Tauri sous Linux (le jeu se lançait
/// alors deux fois). Deux verrous en plus : un seul lancement à la fois, et pas de nouveau
/// lancement tant que le jeu tourne.
#[tauri::command]
pub fn launch_instance(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    req: LaunchInstanceRequest,
) -> LauncherResult<()> {
    if state.game_running.load(Ordering::SeqCst) {
        return Err(LauncherError::InvalidLaunchConfig("Le jeu est déjà ouvert.".into()));
    }
    if state.launching.swap(true, Ordering::SeqCst) {
        return Err(LauncherError::InvalidLaunchConfig(
            "Un lancement est déjà en cours.".into(),
        ));
    }
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        // Future volumineuse (installation complète) : sur le tas, pas sur la pile du thread Tokio.
        let res = Box::pin(run_launch(&app, &window, state.inner(), &req)).await;
        state.launching.store(false, Ordering::SeqCst);
        let payload = match res {
            Ok(pid) => LaunchResult { ok: true, pid: Some(pid), error: None },
            Err(err) => {
                emit_log(&app, &state.logs, LogLevel::Error, "launch", err.to_string());
                LaunchResult { ok: false, pid: None, error: serde_json::to_value(&err).ok() }
            }
        };
        let _ = app.emit("launcher://launch-result", payload);
    });
    Ok(())
}

/// Ferme le jeu lancé par le launcher (bouton « Fermer le jeu »).
#[tauri::command]
pub fn kill_game(app: AppHandle, state: State<'_, AppState>) -> bool {
    if !state.game_running.load(Ordering::SeqCst) {
        return false;
    }
    state.kill_requested.store(true, Ordering::SeqCst);
    state.game_kill.notify_waiters();
    emit_log(&app, &state.logs, LogLevel::Warn, "launch", "Fermeture du jeu demandée par le joueur.");
    true
}

/// Journal à afficher : `latest` (logs/latest.log du jeu), `crash` (dernier rapport de plantage)
/// ou `launcher` (launcher.log). Fin du fichier seulement, chemins et jeton masqués.
#[tauri::command]
pub async fn read_game_log(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    kind: String,
) -> LauncherResult<serde_json::Value> {
    use crate::extras::{redact, tail_file};
    let game_dir = resolve_game_directory(
        &state.data_root_dir,
        &data_directory,
        instance.as_deref(),
        "geoventure",
        channel.as_deref().unwrap_or("stable"),
    );
    let path = match kind.as_str() {
        "latest" => Some(game_dir.join("logs/latest.log")),
        "crash" => crate::crash::crash_report_since(&game_dir, std::time::UNIX_EPOCH),
        "launcher" => Some(state.log_dir.join("launcher.log")),
        _ => return Err(LauncherError::InvalidLaunchConfig(format!("Journal inconnu : {kind}"))),
    };
    let Some(path) = path else {
        return Ok(serde_json::json!({ "path": null, "text": "" }));
    };
    let text = tail_file(&path, 1500).unwrap_or_default();
    let mut secrets = Vec::new();
    if let Ok(Some(acc)) = load_account_session(&state.session_dir).await {
        secrets.push(acc.access_token);
    }
    let homes: Vec<String> = directories::BaseDirs::new()
        .map(|b| vec![b.home_dir().to_string_lossy().to_string()])
        .unwrap_or_default();
    Ok(serde_json::json!({
        "path": path.exists().then(|| path.to_string_lossy().to_string()),
        "text": redact(&text, &homes, &secrets),
    }))
}

/// Cartes graphiques détectées (diagnostic et conseils).
#[tauri::command]
pub async fn detect_gpus() -> serde_json::Value {
    let gpus = tokio::task::spawn_blocking(crate::gpu::detect).await.unwrap_or_default();
    serde_json::json!({ "gpus": gpus, "only_integrated": crate::gpu::only_integrated(&gpus) })
}

/// Remet les réglages du jeu comme avant le profil Économe.
#[tauri::command]
pub async fn restore_game_options(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
) -> LauncherResult<bool> {
    let game_dir = resolve_game_directory(
        &state.data_root_dir,
        &data_directory,
        instance.as_deref(),
        "geoventure",
        channel.as_deref().unwrap_or("stable"),
    );
    Ok(crate::gameopts::restore(&game_dir)?)
}

/// Ouvre le dossier des journaux du launcher.
#[tauri::command]
pub fn open_launcher_logs(app: AppHandle, state: State<'_, AppState>) -> LauncherResult<String> {
    use tauri_plugin_opener::OpenerExt;
    let _ = std::fs::create_dir_all(&state.log_dir);
    let s = state.log_dir.to_string_lossy().to_string();
    app.opener()
        .open_path(s.clone(), None::<&str>)
        .map_err(|e| LauncherError::Internal(e.to_string()))?;
    Ok(s)
}

/// Le jeu lancé par ce launcher est-il encore ouvert ?
#[tauri::command]
pub fn game_status(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::json!({
        "launching": state.launching.load(Ordering::SeqCst),
        "running": state.game_running.load(Ordering::SeqCst),
    })
}

async fn run_launch(
    app: &AppHandle,
    window: &WebviewWindow,
    state: &AppState,
    req: &LaunchInstanceRequest,
) -> LauncherResult<u32> {
    let ch = req.channel.as_deref().unwrap_or("stable");
    let game_dir = resolve_game_directory(
        &state.data_root_dir,
        &req.data_directory,
        req.instance.as_deref(),
        "geoventure",
        ch,
    );
    tokio::fs::create_dir_all(&game_dir).await?;
    emit_log(
        app,
        &state.logs,
        LogLevel::Info,
        "launch",
        format!("Préparation de {} (canal {ch}) dans {}", req.instance.as_deref().unwrap_or("geoventure"), game_dir.display()),
    );

    let app_p = app.clone();
    let app_l = app.clone();
    let logs_l = state.logs.clone();
    let installer = Installer {
        http: PanelClient::download_client()?,
        game_dir: game_dir.clone(),
        on_progress: Arc::new(move |p| {
            let _ = app_p.emit("launcher://install-progress", p);
        }),
        on_log: Arc::new(move |level, msg| {
            let entry = logs_l.push(level, "install", msg);
            let _ = app_l.emit("launcher://log", entry);
        }),
    };

    // 1. Minecraft vanilla (donne aussi la version de Java attendue)
    let vanilla = Box::pin(installer.install_vanilla(&req.mc_version)).await?;

    // 2. Java : chemin choisi par le joueur s'il est valide, sinon le Java 17 de Mojang
    let java_path = match req
        .custom_java_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|p| probe_java_version(std::path::Path::new(p)))
    {
        Some(custom) => {
            emit_log(app, &state.logs, LogLevel::Info, "java", format!("Java personnalisé : {}", custom.label));
            PathBuf::from(custom.path)
        }
        None => Box::pin(installer.install_java(&vanilla)).await?,
    };

    // 3. Forge
    let use_forge = req.loader_enable && req.loader_type.eq_ignore_ascii_case("forge");
    if use_forge {
        Box::pin(installer.install_forge(&req.mc_version, &req.loader_build, &java_path, &vanilla)).await?;
    } else if req.loader_enable {
        return Err(LauncherError::InvalidLaunchConfig(format!(
            "Loader « {} » pas encore pris en charge par ce launcher (Forge uniquement).",
            req.loader_type
        )));
    }

    // 4. Modpack du panel
    let _ = app.emit(
        "launcher://install-progress",
        InstallProgress {
            phase: "modpack".into(),
            label: "Modpack".into(),
            completed: 0,
            total: 0,
            current_file: String::new(),
            bytes: 0,
        },
    );
    let mut ignored = req.ignored.clone();
    ignored.push("launcher_config".into());
    match state
        .panel()
        .fetch_modpack_manifest(req.instance.as_deref(), ch)
        .await
    {
        Ok(manifest) => {
            let app_m = app.clone();
            let report = Box::pin(sync_files(
                PanelClient::download_client()?,
                &manifest,
                &game_dir,
                &SyncOptions::modpack(ignored.clone()),
                move |p| {
                    let _ = app_m.emit(
                        "launcher://install-progress",
                        InstallProgress {
                            phase: "modpack".into(),
                            label: "Modpack".into(),
                            completed: p.completed,
                            total: p.total,
                            current_file: p.current_file,
                            bytes: p.downloaded_bytes,
                        },
                    );
                },
            ))
            .await?;
            if let Some(first) = report.failed.first() {
                return Err(LauncherError::Internal(format!(
                    "Modpack : {} fichier(s) en échec (ex. {} : {}). Relancez.",
                    report.failed.len(),
                    first.path,
                    first.error
                )));
            }
            let removed = cleanup_stale_mods(&game_dir, &manifest, &ignored).await?;
            emit_log(
                app,
                &state.logs,
                LogLevel::Info,
                "modpack",
                format!(
                    "Modpack : {} à jour, {} téléchargés, {} ancien(s) mod(s) retiré(s){}.",
                    report.valid_count,
                    report.downloaded_count,
                    removed.len(),
                    if removed.is_empty() { String::new() } else { format!(" ({})", removed.join(", ")) }
                ),
            );
        }
        Err(err) => {
            if !game_dir.join("mods").is_dir() {
                return Err(err);
            }
            emit_log(
                app,
                &state.logs,
                LogLevel::Warn,
                "modpack",
                format!("Liste du modpack injoignable ({err}) : lancement avec les fichiers déjà installés."),
            );
        }
    }
    if use_forge {
        let ok = apply_forge_window_settings(&game_dir);
        emit_log(
            app,
            &state.logs,
            LogLevel::Info,
            "launch",
            format!("fml.toml : earlyWindowControl=true {}", if ok { "appliqué" } else { "NON appliqué" }),
        );
    }

    if req.light_game_options {
        match crate::gameopts::apply_light(&game_dir) {
            Ok(changed) if !changed.is_empty() => emit_log(app, &state.logs, LogLevel::Info, "launch", format!("Réglages légers du jeu appliqués : {}.", changed.join(", "))),
            Ok(_) => {}
            Err(e) => emit_log(app, &state.logs, LogLevel::Warn, "launch", format!("options.txt non modifié : {e}")),
        }
    }

    // 5. Lancement
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
        return Err(LauncherError::InvalidLaunchConfig(format!(
            "{} librairie(s) manquante(s) après installation (ex. {}). Utilisez « Réparer » dans les réglages.",
            plan.missing_libraries.len(),
            plan.missing_libraries[0]
        )));
    }
    let _ = app.emit(
        "launcher://install-progress",
        InstallProgress {
            phase: "launch".into(),
            label: "Lancement de Minecraft".into(),
            completed: 1,
            total: 1,
            current_file: String::new(),
            bytes: 0,
        },
    );
    emit_log(
        app,
        &state.logs,
        LogLevel::Info,
        "launch",
        format!(
            "Lancement : RAM {}-{} Go, Java {}, {} jars.",
            req.ram_min_gb,
            req.ram_max_gb,
            java_path.display(),
            plan.classpath.len()
        ),
    );

    let after = req.after_launch.clone().unwrap_or_else(|| "keep".into());
    let app_log = app.clone();
    let app_exit = app.clone();
    let win_exit = window.clone();
    let logs_exit = state.logs.clone();
    let after_exit = after.clone();
    let running_exit = Arc::clone(&state.game_running);
    let hide_on_window = after == "hide";
    let window_seen = Arc::new(AtomicBool::new(false));
    let win_hide = window.clone();
    let window_seen_watch = Arc::clone(&window_seen);
    let started_at = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
    let game_dir_exit = game_dir.clone();
    let killed_exit = Arc::clone(&state.kill_requested);
    state.kill_requested.store(false, Ordering::SeqCst);
    let pid = spawn_resolved_forge_with_exit(
        &java_path.to_string_lossy(),
        &game_dir,
        &plan,
        state.logs.clone(),
        Some(Arc::clone(&state.game_kill)),
        move |entry| {
            // « Cacher le launcher » : seulement quand une fenêtre du jeu est vraiment à l'écran
            // (fenêtre de chargement de Forge ou de Minecraft), jamais avant.
            if hide_on_window
                && !window_seen.load(Ordering::SeqCst)
                && is_game_window_line(&entry.message)
            {
                window_seen.store(true, Ordering::SeqCst);
                let _ = win_hide.hide();
            }
            let _ = app_log.emit("launcher://log", entry);
        },
        move |code| {
            running_exit.store(false, Ordering::SeqCst);
            let entry = logs_exit.push(
                if code == Some(0) { LogLevel::Info } else { LogLevel::Warn },
                "launch",
                format!("Minecraft fermé (code {}).", code.map(|c| c.to_string()).unwrap_or_else(|| "?".into())),
            );
            let _ = app_exit.emit("launcher://log", entry);
            let _ = app_exit.emit("launcher://game-exit", code);
            // Fermeture anormale (pas demandée par le joueur) : cause probable et conseil.
            if code != Some(0) && !killed_exit.swap(false, Ordering::SeqCst) {
                let recent: String = logs_exit
                    .list()
                    .iter()
                    .rev()
                    .filter(|e| e.source == "minecraft")
                    .take(400)
                    .map(|e| e.message.as_str())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                let analysis = crate::crash::analyze_exit(&game_dir_exit, started_at, code, &recent);
                let msg = match &analysis.cause {
                    Some(c) => format!("Plantage analysé : {} — {}", c.title, c.advice),
                    None => "Plantage sans cause reconnue : lancez un diagnostic.".to_string(),
                };
                let entry = logs_exit.push(LogLevel::Warn, "crash", msg);
                let _ = app_exit.emit("launcher://log", entry);
                let _ = app_exit.emit("launcher://game-crash", analysis);
            }
            if after_exit == "hide" {
                let _ = win_exit.show();
                let _ = win_exit.set_focus();
            }
        },
    )
    .await?;
    state.game_running.store(true, Ordering::SeqCst);

    emit_log(app, &state.logs, LogLevel::Info, "launch", format!("Minecraft démarré (PID {pid})."));
    // Garde-fou : 3 min sans fenêtre de jeu → le launcher reste/revient au premier plan avec un message.
    {
        let app_w = app.clone();
        let win_w = window.clone();
        let logs_w = state.logs.clone();
        let running_w = Arc::clone(&state.game_running);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(180)).await;
            if running_w.load(Ordering::SeqCst) && !window_seen_watch.load(Ordering::SeqCst) {
                let entry = logs_w.push(
                    LogLevel::Warn,
                    "launch",
                    "Aucune fenêtre de jeu après 3 minutes : le chargement est très long ou bloqué. Lancez un diagnostic s'il n'apparaît pas.",
                );
                let _ = app_w.emit("launcher://log", entry);
                let _ = app_w.emit("launcher://game-slow", ());
                let _ = win_w.show();
                let _ = win_w.set_focus();
            }
        });
    }
    match after.as_str() {
        // « hide » : la fenêtre est cachée dans le rappel du journal, quand le jeu est visible.
        "close" => {
            let app_c = app.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                app_c.exit(0);
            });
        }
        _ => {}
    }
    Ok(pid)
}

/// Ligne du journal qui prouve qu'une fenêtre du jeu est à l'écran : fenêtre de chargement de
/// Forge (`EARLYDISPLAY`) ou fenêtre de Minecraft.
pub fn is_game_window_line(line: &str) -> bool {
    (line.contains("EARLYDISPLAY") && (line.contains("Requested GL version") || line.contains("GL info")))
        || line.contains("Backend library: LWJGL")
        || line.contains("Setting user:")
}

/// Routes du panel que l'interface peut lire (lecture seule, liste fermée).
pub const PANEL_READ_ROUTES: &[&str] = &[
    "utils/notifications",
    "utils/servers-status",
    "utils/achievements",
    "utils/leaderboards",
    "utils/factions",
    "utils/seasons",
    "utils/game-events",
    "utils/mods",
    "utils/community-mods",
    "utils/wonder",
    "utils/changelog",
    "utils/launcher-content",
    "utils/servers-history",
    "api/centralcorp/community-mods",
];

/// Lit une route JSON du panel (annonces, statut, succès, classements, saisons…).
#[tauri::command]
pub async fn panel_get(
    state: State<'_, AppState>,
    endpoint: String,
    instance: Option<String>,
    channel: Option<String>,
) -> LauncherResult<serde_json::Value> {
    let ep = endpoint.trim().trim_start_matches('/');
    if !PANEL_READ_ROUTES.contains(&ep) {
        return Err(LauncherError::InvalidUrl(format!("Route du panel non autorisée : {ep}")));
    }
    // `utils/game-events` est globale : jamais `?instance=`.
    let inst = if ep == "utils/game-events" { None } else { instance };
    state
        .panel()
        .fetch_panel_json(ep, inst.as_deref(), channel.as_deref().unwrap_or("stable"))
        .await
}

/// Ouvre un lien http(s) dans le navigateur du joueur.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> LauncherResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !crate::downloader::is_safe_http_url(&url) {
        return Err(LauncherError::InvalidUrl(url));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| LauncherError::Internal(e.to_string()))
}

/// Ouvre le dossier de jeu de l'instance dans l'explorateur de fichiers.
#[tauri::command]
pub async fn open_game_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
) -> LauncherResult<String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = resolve_game_directory(
        &state.data_root_dir,
        &data_directory,
        instance.as_deref(),
        "geoventure",
        channel.as_deref().unwrap_or("stable"),
    );
    tokio::fs::create_dir_all(&dir).await?;
    let s = dir.to_string_lossy().to_string();
    app.opener()
        .open_path(s.clone(), None::<&str>)
        .map_err(|e| LauncherError::Internal(e.to_string()))?;
    Ok(s)
}

/// Réparation 1 clic : vide le cache de config, revérifie tout le modpack (empreintes) et
/// supprime les fichiers altérés, qui seront re-téléchargés au prochain lancement. Les réglages
/// du joueur (options.txt, config/…) ne sont pas touchés.
#[tauri::command]
pub async fn repair_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    ignored: Option<Vec<String>>,
) -> LauncherResult<DiffScanReport> {
    let ch = channel.as_deref().unwrap_or("stable");
    let game_dir = resolve_game_directory(&state.data_root_dir, &data_directory, instance.as_deref(), "geoventure", ch);
    let _ = tokio::fs::remove_file(crate::config::cache_file_path(&state.cache_dir, instance.as_deref(), ch)).await;
    let manifest = state.panel().fetch_modpack_manifest(instance.as_deref(), ch).await?;
    let mut ign = ignored.unwrap_or_default();
    ign.push("launcher_config".into());
    let report = diff_verify_manifest(&manifest, &game_dir, &ign).await?;
    for e in &report.corrupted {
        if let Ok(p) = crate::downloader::resolve_safe_path(&game_dir, &e.path) {
            let _ = tokio::fs::remove_file(p).await;
        }
    }
    emit_log(
        &app,
        &state.logs,
        LogLevel::Info,
        "repair",
        format!(
            "Réparation : {} fichiers vérifiés, {} manquants, {} altérés supprimés.",
            report.total,
            report.missing.len(),
            report.corrupted.len()
        ),
    );
    Ok(report)
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

#[derive(Debug, Clone, Serialize)]
pub struct LauncherUpdateInfo {
    pub current: String,
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// Cherche une nouvelle version du launcher (fichier `latest.json` signé de la release `rust-latest`).
#[tauri::command]
pub async fn check_launcher_update(app: AppHandle) -> LauncherResult<LauncherUpdateInfo> {
    use tauri_plugin_updater::UpdaterExt;
    let current = app.package_info().version.to_string();
    // Build de développement : jamais de mise à jour (elle remplacerait le binaire en test).
    if cfg!(debug_assertions) && std::env::var_os("GEO_DEV_UPDATER").is_none() {
        return Ok(LauncherUpdateInfo { current, available: false, version: None, notes: None });
    }
    let updater = app
        .updater()
        .map_err(|e| LauncherError::Internal(format!("Mise à jour indisponible : {e}")))?;
    let update = updater
        .check()
        .await
        .map_err(|e| LauncherError::Internal(format!("Vérification de mise à jour impossible : {e}")))?;
    Ok(match update {
        Some(u) => LauncherUpdateInfo {
            current,
            available: true,
            version: Some(u.version.clone()),
            notes: u.body.clone(),
        },
        None => LauncherUpdateInfo {
            current,
            available: false,
            version: None,
            notes: None,
        },
    })
}

/// Télécharge la mise à jour (signature vérifiée), l'installe puis redémarre le launcher.
/// Répond tout de suite ; une erreur arrive par `launcher://update-error`.
#[tauri::command]
pub fn install_launcher_update(app: AppHandle, state: State<'_, AppState>) -> LauncherResult<()> {
    if state.launching.load(Ordering::SeqCst) || state.game_running.load(Ordering::SeqCst) {
        return Err(LauncherError::Internal("Partie en cours : mise à jour reportée.".into()));
    }
    tauri::async_runtime::spawn(async move {
        if let Err(err) = download_and_install_update(&app).await {
            let state = app.state::<AppState>();
            emit_log(&app, &state.logs, LogLevel::Warn, "update", err.to_string());
            let _ = app.emit("launcher://update-error", err.to_string());
        }
    });
    Ok(())
}

async fn download_and_install_update(app: &AppHandle) -> LauncherResult<()> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| LauncherError::Internal(e.to_string()))?
        .check()
        .await
        .map_err(|e| LauncherError::Internal(e.to_string()))?;
    let Some(update) = update else {
        return Ok(());
    };
    {
        let state = app.state::<AppState>();
        emit_log(app, &state.logs, LogLevel::Info, "update", format!("Mise à jour du launcher vers {}…", update.version));
    }
    let app_p = app.clone();
    let mut downloaded = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = app_p.emit("launcher://update-progress", UpdateDownloadProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| LauncherError::Internal(format!("Installation de la mise à jour impossible : {e}")))?;
    app.restart();
}

/// Nom de fichier de mod sûr : un seul segment, se termine par `.jar`.
pub fn safe_mod_file_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty()
        || n.len() > 200
        || n.contains(['/', '\\', '\0'])
        || n.contains("..")
        || !n.to_ascii_lowercase().ends_with(".jar")
    {
        return None;
    }
    Some(n.to_string())
}

fn mods_dir_for(
    state: &AppState,
    data_directory: &str,
    instance: Option<&str>,
    channel: Option<&str>,
) -> PathBuf {
    resolve_game_directory(
        &state.data_root_dir,
        data_directory,
        instance,
        "geoventure",
        channel.unwrap_or("stable"),
    )
    .join("mods")
}

/// État des mods optionnels : `true` = actif (ou pas encore téléchargé et non désactivé).
#[tauri::command]
pub fn optional_mods_state(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    files: Vec<String>,
) -> std::collections::HashMap<String, bool> {
    let dir = mods_dir_for(&state, &data_directory, instance.as_deref(), channel.as_deref());
    files
        .into_iter()
        .filter_map(|f| safe_mod_file_name(&f))
        .map(|f| {
            let disabled = dir.join(format!("{f}-disable")).exists();
            (f, !disabled)
        })
        .collect()
}

/// Active ou désactive un mod optionnel (`mods/<nom>.jar` ↔ `mods/<nom>.jar-disable`).
/// Un mod pas encore téléchargé est désactivé par un fichier vide `<nom>.jar-disable`, que la
/// synchronisation du modpack respecte.
#[tauri::command]
pub async fn set_optional_mod(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    file: String,
    enabled: bool,
) -> LauncherResult<bool> {
    let name = safe_mod_file_name(&file).ok_or_else(|| LauncherError::UnsafePath(file.clone()))?;
    let dir = mods_dir_for(&state, &data_directory, instance.as_deref(), channel.as_deref());
    tokio::fs::create_dir_all(&dir).await?;
    let on = dir.join(&name);
    let off = dir.join(format!("{name}-disable"));
    if enabled {
        if let Ok(meta) = tokio::fs::metadata(&off).await {
            if meta.len() == 0 || on.exists() {
                tokio::fs::remove_file(&off).await?;
            } else {
                tokio::fs::rename(&off, &on).await?;
            }
        }
    } else if on.exists() {
        if off.exists() {
            tokio::fs::remove_file(&off).await?;
        }
        tokio::fs::rename(&on, &off).await?;
    } else if !off.exists() {
        tokio::fs::write(&off, b"").await?;
    }
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_window_lines_are_detected() {
        assert!(is_game_window_line("[main/INFO] [EARLYDISPLAY/]: Requested GL version 4.5 got version 4.5"));
        assert!(is_game_window_line("[Render thread/INFO] [minecraft/Minecraft]: Setting user: Steve"));
        assert!(!is_game_window_line("[main/INFO] [cpw.mods.modlauncher.Launcher/MODLAUNCHER]: ModLauncher running"));
    }

    #[test]
    fn mod_file_names_are_confined() {
        assert_eq!(safe_mod_file_name("minimap.jar").as_deref(), Some("minimap.jar"));
        assert!(safe_mod_file_name("../evil.jar").is_none());
        assert!(safe_mod_file_name("a/b.jar").is_none());
        assert!(safe_mod_file_name("a\\b.jar").is_none());
        assert!(safe_mod_file_name("readme.txt").is_none());
    }
}

// ---------------------------------------------------------------------------------------------
// Actualité, adresse du panel, diagnostic
// ---------------------------------------------------------------------------------------------

/// Actualité du site Azuriom (flux RSS `/api/rss`), en texte brut.
#[tauri::command]
pub async fn fetch_news(site_url: String) -> LauncherResult<Vec<crate::extras::NewsItem>> {
    let http = PanelClient::new(&site_url, HttpTimeouts::default())?;
    crate::extras::fetch_news(http.http_client(), &site_url).await
}

pub const PANEL_URL_FILE: &str = "panel_url.txt";

#[tauri::command]
pub fn get_panel_url(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::json!({
        "current": state.panel().base_url().to_string(),
        "default": crate::config::DEFAULT_PANEL_URL,
    })
}

/// Change l'adresse du panel (comme `geoventure_server_url` dans le launcher Electron).
/// L'adresse est d'abord testée : `utils/api` doit répondre avec une configuration lisible.
/// Une adresse vide remet celle par défaut.
#[tauri::command]
pub async fn set_panel_url(app: AppHandle, state: State<'_, AppState>, url: String) -> LauncherResult<String> {
    let wanted = if url.trim().is_empty() {
        crate::config::DEFAULT_PANEL_URL.to_string()
    } else {
        url.trim().to_string()
    };
    let client = PanelClient::new(&wanted, HttpTimeouts::default())?;
    let probe_dir = state.cache_dir.join("panel-probe");
    let res = client.fetch_config_with_cache(None, "stable", &probe_dir).await?;
    let _ = tokio::fs::remove_dir_all(&probe_dir).await;
    if res.offline {
        return Err(LauncherError::Internal("Ce panel ne répond pas.".into()));
    }
    let base = client.base_url().to_string();
    tokio::fs::create_dir_all(&state.session_dir).await?;
    if base == crate::config::DEFAULT_PANEL_URL {
        let _ = tokio::fs::remove_file(state.session_dir.join(PANEL_URL_FILE)).await;
    } else {
        tokio::fs::write(state.session_dir.join(PANEL_URL_FILE), &base).await?;
    }
    if let Ok(mut g) = state.panel_client.write() {
        *g = client;
    }
    emit_log(&app, &state.logs, LogLevel::Info, "panel", format!("Panel utilisé : {base}"));
    Ok(base)
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticRequest {
    pub data_directory: String,
    pub instance: Option<String>,
    pub channel: Option<String>,
    pub site_url: Option<String>,
    #[serde(default)]
    pub settings_summary: Option<String>,
}

/// Rapport de diagnostic : système, Java, installation, sondes réseau, journaux du launcher et
/// du jeu, dernier rapport de plantage. Tout est masqué (`extras::redact`) avant d'être renvoyé.
#[tauri::command]
pub async fn collect_diagnostic(app: AppHandle, state: State<'_, AppState>, req: DiagnosticRequest) -> LauncherResult<String> {
    use crate::extras::{probe, redact, tail_file, tail_lines};
    use std::fmt::Write as _;
    let ch = req.channel.as_deref().unwrap_or("stable");
    let game_dir = resolve_game_directory(&state.data_root_dir, &req.data_directory, req.instance.as_deref(), "geoventure", ch);
    let sys = get_system_overview(None);
    let mut r = String::new();
    let _ = writeln!(r, "=== Diagnostic Geoventure Launcher (Rust) ===");
    let _ = writeln!(r, "Date : {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z"));
    let _ = writeln!(r, "Launcher : {}", app.package_info().version);
    let _ = writeln!(r, "Système : {}", sys.os);
    let _ = writeln!(r, "Processeur : {} ({} cœurs)", sys.cpu_name, sys.cpu_cores);
    let gpus = tokio::task::spawn_blocking(crate::gpu::detect).await.unwrap_or_default();
    let _ = writeln!(r, "Carte(s) graphique(s) : {}", if gpus.is_empty() { "non détectée".to_string() } else { gpus.join(" | ") });
    let _ = writeln!(r, "Mémoire : {} Go au total, {} Go libres, plafond jeu {} Go", sys.total_ram_gb, sys.free_ram_gb, sys.plan.cap_gb);
    let profile_fr = match sys.recommended_profile {
        PerformanceProfileId::Eco => "Économe",
        PerformanceProfileId::Balanced => "Équilibré",
        PerformanceProfileId::Performance => "Performant",
    };
    let _ = writeln!(r, "Profil conseillé : {profile_fr}");
    if let Some(s) = &req.settings_summary {
        let _ = writeln!(r, "Réglages : {s}");
    }
    let _ = writeln!(r, "Instance : {} / canal {ch}", req.instance.as_deref().unwrap_or("geoventure"));
    let _ = writeln!(r, "Dossier de jeu : {} ({})", game_dir.display(), if game_dir.is_dir() { "présent" } else { "absent" });
    let _ = writeln!(r, "Panel : {}", state.panel().base_url());

    let _ = writeln!(r, "\n--- Installation ---");
    let count = |p: &std::path::Path| std::fs::read_dir(p).map(|d| d.count()).unwrap_or(0);
    let _ = writeln!(r, "versions/ : {} · libraries/ : {} · loader/forge/versions : {}",
        count(&game_dir.join("versions")), count(&game_dir.join("libraries")), count(&game_dir.join("loader/forge/versions")));
    let mods: Vec<String> = std::fs::read_dir(game_dir.join("mods"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    let _ = writeln!(r, "mods/ : {} fichier(s), dont {} désactivé(s)", mods.len(), mods.iter().filter(|m| m.ends_with("-disable")).count());
    let runtimes: Vec<String> = std::fs::read_dir(game_dir.join("runtime"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    let _ = writeln!(r, "Java fourni : {}", if runtimes.is_empty() { "aucun".to_string() } else { runtimes.join(", ") });
    let gd = game_dir.clone();
    let jdks = tokio::task::spawn_blocking(move || detect_installed_jdks(Some(&gd))).await.unwrap_or_default();
    for j in jdks.iter().take(6) {
        let _ = writeln!(r, "Java détecté : {} — {}", j.label, j.path);
    }

    let _ = writeln!(r, "\n--- Réseau ---");
    let http = PanelClient::download_client()?;
    let panel_url = format!("{}api-schema.json", state.panel().base_url());
    let site = req.site_url.clone().unwrap_or_else(|| "https://geoventure.fr/".into());
    let probes = futures_util::future::join_all([
        probe(&http, "Panel", &panel_url),
        probe(&http, "Azuriom", &site),
        probe(&http, "Mojang", crate::install::VERSION_MANIFEST_URL),
        probe(&http, "Forge", "https://maven.minecraftforge.net/"),
    ])
    .await;
    for p in probes {
        let _ = writeln!(r, "{} : {} ({} ms){}", p.name,
            if p.ok { "OK" } else { "ÉCHEC" },
            p.millis,
            match (p.status, &p.error) { (Some(s), _) => format!(" HTTP {s}"), (None, Some(e)) => format!(" {e}"), _ => String::new() });
    }

    let _ = writeln!(r, "\n--- Journal du launcher (300 dernières lignes) ---");
    let _ = writeln!(r, "{}", tail_lines(&state.logs.export_text(), 300));
    let _ = writeln!(r, "\n--- logs/latest.log du jeu (300 dernières lignes) ---");
    let _ = writeln!(r, "{}", tail_file(&game_dir.join("logs/latest.log"), 300).unwrap_or_else(|| "(absent)".into()));
    let crash = std::fs::read_dir(game_dir.join("crash-reports")).ok().and_then(|d| {
        d.flatten().max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok()).map(|e| e.path())
    });
    if let Some(c) = crash {
        let _ = writeln!(r, "\n--- Dernier rapport de plantage : {} ---", c.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let head = std::fs::read_to_string(&c).map(|t| t.lines().take(80).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        let _ = writeln!(r, "{head}");
    }

    let mut secrets = Vec::new();
    if let Ok(Some(acc)) = load_account_session(&state.session_dir).await {
        secrets.push(acc.access_token);
        secrets.push(acc.uuid);
    }
    let mut homes = Vec::new();
    if let Some(b) = directories::BaseDirs::new() {
        homes.push(b.home_dir().to_string_lossy().to_string());
    }
    Ok(redact(&r, &homes, &secrets))
}

/// Enregistre le rapport dans `<dossier de jeu>/logs/diagnostic-<date>.txt` et renvoie le chemin.
#[tauri::command]
pub async fn save_diagnostic(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    report: String,
) -> LauncherResult<String> {
    let game_dir = resolve_game_directory(&state.data_root_dir, &data_directory, instance.as_deref(), "geoventure", channel.as_deref().unwrap_or("stable"));
    let dir = game_dir.join("logs");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("diagnostic-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    tokio::fs::write(&path, report).await?;
    Ok(path.to_string_lossy().to_string())
}

/// Envoie le rapport (déjà masqué) au panel : `POST utils/diagnostic`. Un panel sans cette route
/// (404, 405, 501) n'est pas une erreur : la réponse l'indique.
#[tauri::command]
pub async fn send_diagnostic(
    app: AppHandle,
    state: State<'_, AppState>,
    instance: Option<String>,
    report: String,
) -> LauncherResult<serde_json::Value> {
    let panel = state.panel();
    let url = panel
        .base_url()
        .join("utils/diagnostic")
        .map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;
    let body = serde_json::json!({
        "launcherVersion": app.package_info().version.to_string(),
        "instance": instance,
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "report": report,
    });
    let resp = panel
        .http_client()
        .post(url.clone())
        .json(&body)
        .send()
        .await
        .map_err(|e| LauncherError::Network { url: url.to_string(), message: e.to_string() })?;
    let status = resp.status().as_u16();
    Ok(match status {
        200..=299 => serde_json::json!({ "sent": true, "status": status }),
        404 | 405 | 501 => serde_json::json!({ "sent": false, "unsupported": true, "status": status }),
        _ => return Err(LauncherError::HttpStatus { status, url: url.to_string() }),
    })
}

// ============================================================================
// Lot 2 : Discord, mods communauté, captures, packs
// ============================================================================

fn game_dir_of(state: &AppState, data_directory: &str, instance: Option<&str>, channel: Option<&str>) -> PathBuf {
    resolve_game_directory(&state.data_root_dir, data_directory, instance, "geoventure", channel.unwrap_or("stable"))
}

/// Met à jour la présence Discord (`null` l'efface).
#[tauri::command]
pub fn discord_presence(presence: Option<crate::discord::Presence>) {
    crate::discord::set(presence);
}

#[tauri::command]
pub fn community_mods_installed(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
) -> Vec<String> {
    crate::media::installed_community(&game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref()))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn install_community_mod(
    app: AppHandle,
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    url: String,
    file_name: String,
    hash: Option<String>,
) -> LauncherResult<()> {
    if state.game_running.load(Ordering::SeqCst) {
        return Err(LauncherError::InvalidLaunchConfig("Fermez le jeu avant d’ajouter un mod.".into()));
    }
    let dir = game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref());
    Box::pin(crate::media::install_community(&dir, &url, &file_name, hash.as_deref().unwrap_or(""))).await?;
    emit_log(&app, &state.logs, LogLevel::Info, "mods", format!("Mod communauté installé : {file_name}"));
    Ok(())
}

#[tauri::command]
pub fn uninstall_community_mod(
    app: AppHandle,
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    file_name: String,
) -> LauncherResult<()> {
    if state.game_running.load(Ordering::SeqCst) {
        return Err(LauncherError::InvalidLaunchConfig("Fermez le jeu avant de retirer un mod.".into()));
    }
    let dir = game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref());
    crate::media::uninstall_community(&dir, &file_name)?;
    emit_log(&app, &state.logs, LogLevel::Info, "mods", format!("Mod communauté retiré : {file_name}"));
    Ok(())
}

/// Captures d'écran, packs de ressources ou de shaders (`kind`), le plus récent d'abord.
/// Pour les captures, le dossier est ouvert au protocole `asset:` afin d'afficher les images.
#[tauri::command]
pub fn list_content(
    app: AppHandle,
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    kind: String,
) -> LauncherResult<serde_json::Value> {
    let game_dir = game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref());
    let dir = crate::media::content_dir(&game_dir, &kind)
        .ok_or_else(|| LauncherError::InvalidLaunchConfig(format!("Dossier inconnu : {kind}")))?;
    if kind == "screenshots" && dir.is_dir() {
        let _ = app.asset_protocol_scope().allow_directory(&dir, false);
    }
    let entries = crate::media::list_entries(&dir, &kind, 200);
    let (enabled, pack) = crate::media::shader_state(&game_dir);
    Ok(serde_json::json!({
        "entries": entries,
        "shader_loader": crate::media::has_shader_loader(&game_dir),
        "shaders_enabled": enabled,
        "shader_pack": pack,
    }))
}

#[tauri::command]
pub fn delete_content(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    kind: String,
    name: String,
) -> LauncherResult<()> {
    let game_dir = game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref());
    let dir = crate::media::content_dir(&game_dir, &kind)
        .ok_or_else(|| LauncherError::InvalidLaunchConfig(format!("Dossier inconnu : {kind}")))?;
    crate::media::delete_entry(&dir, &name)
}

/// Ouvre le dossier (`name` absent) ou un fichier de ce dossier avec l'application du système.
#[tauri::command]
pub fn open_content(
    app: AppHandle,
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    kind: String,
    name: Option<String>,
) -> LauncherResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let game_dir = game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref());
    let dir = crate::media::content_dir(&game_dir, &kind)
        .ok_or_else(|| LauncherError::InvalidLaunchConfig(format!("Dossier inconnu : {kind}")))?;
    std::fs::create_dir_all(&dir)?;
    let target = match name.as_deref() {
        Some(n) => dir.join(crate::media::safe_entry_name(n).ok_or_else(|| LauncherError::InvalidUrl(format!("Nom refusé : {n}")))?),
        None => dir,
    };
    app.opener()
        .open_path(target.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| LauncherError::Internal(e.to_string()))
}

/// Choisit le pack de shaders actif (Oculus), ou aucun.
#[tauri::command]
pub fn set_shader_pack(
    state: State<'_, AppState>,
    data_directory: String,
    instance: Option<String>,
    channel: Option<String>,
    pack: Option<String>,
) -> LauncherResult<()> {
    if state.game_running.load(Ordering::SeqCst) {
        return Err(LauncherError::InvalidLaunchConfig("Fermez le jeu avant de changer de shaders.".into()));
    }
    let game_dir = game_dir_of(&state, &data_directory, instance.as_deref(), channel.as_deref());
    crate::media::set_shader(&game_dir, pack.as_deref())
}

// ============================================================================
// Lot 3 : zone de notification, démarrage avec l'ordinateur
// ============================================================================

/// État : icône disponible, fermeture vers la zone de notification, démarrage automatique.
#[tauri::command]
pub fn tray_settings(app: AppHandle) -> serde_json::Value {
    use tauri_plugin_autostart::ManagerExt;
    serde_json::json!({
        "tray": crate::tray::TRAY_OK.load(Ordering::SeqCst),
        "close_to_tray": crate::tray::CLOSE_TO_TRAY.load(Ordering::SeqCst),
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
    })
}

#[tauri::command]
pub fn set_tray_settings(app: AppHandle, close_to_tray: Option<bool>, autostart: Option<bool>) -> LauncherResult<serde_json::Value> {
    use tauri_plugin_autostart::ManagerExt;
    if let Some(v) = close_to_tray {
        crate::tray::CLOSE_TO_TRAY.store(v, Ordering::SeqCst);
    }
    if let Some(v) = autostart {
        let al = app.autolaunch();
        let r = if v { al.enable() } else { al.disable() };
        r.map_err(|e| LauncherError::Internal(format!("Démarrage automatique : {e}")))?;
    }
    Ok(tray_settings(app))
}

// ============================================================================
// Télémétrie (avec l'accord du joueur) et notifications du système
// ============================================================================

/// Nom d'événement accepté par le panel (`/utils/telemetry`).
pub fn valid_event_name(e: &str) -> bool {
    !e.is_empty() && e.len() <= 64 && e.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

/// Envoie un événement anonyme au panel. L'interface ne l'appelle que si le joueur a accepté.
/// Seuls des nombres et des booléens accompagnent l'événement : jamais de texte libre.
#[tauri::command]
pub async fn send_telemetry(
    app: AppHandle,
    state: State<'_, AppState>,
    event: String,
    instance: Option<String>,
    extra: Option<serde_json::Map<String, serde_json::Value>>,
) -> LauncherResult<bool> {
    if !valid_event_name(&event) {
        return Ok(false);
    }
    let mut body = serde_json::Map::new();
    for (k, v) in extra.unwrap_or_default() {
        if k.len() <= 40 && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && (v.is_number() || v.is_boolean()) {
            body.insert(k, v);
        }
    }
    body.insert("event".into(), event.into());
    body.insert("serverId".into(), instance.unwrap_or_else(|| "geoventure".into()).into());
    body.insert("launcherVersion".into(), format!("rust-{}", app.package_info().version).into());
    body.insert("os".into(), format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH).into());
    let panel = state.panel();
    let url = panel.base_url().join("utils/telemetry").map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;
    let ok = tokio::time::timeout(std::time::Duration::from_secs(5), panel.http_client().post(url).json(&body).send())
        .await
        .ok()
        .and_then(|r| r.ok())
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    Ok(ok)
}

/// Notification du système (Windows, macOS, Linux). Ignorée si le système la refuse.
#[tauri::command]
pub fn desktop_notify(app: AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    let title: String = title.chars().take(80).collect();
    let body: String = body.chars().take(240).collect();
    let _ = app.notification().builder().title(title).body(body).show();
}

#[cfg(test)]
mod telemetry_tests {
    #[test]
    fn event_names() {
        assert!(super::valid_event_name("game_crash:out_of_memory"));
        assert!(!super::valid_event_name("a b"));
        assert!(!super::valid_event_name(""));
    }
}
