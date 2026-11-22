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
use tauri::{AppHandle, Emitter, State, WebviewWindow};

pub struct AppState {
    pub panel_client: PanelClient,
    pub cache_dir: PathBuf,
    pub session_dir: PathBuf,
    pub data_root_dir: PathBuf,
    pub logs: LogStore,
    pub launching: Arc<AtomicBool>,
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

/// Bouton « Jouer » : installe ce qui manque (Java 17, Minecraft, Forge, modpack) puis lance le
/// jeu. Fonctionne sur un PC neuf. Un seul lancement à la fois.
#[tauri::command]
pub async fn launch_instance(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    req: LaunchInstanceRequest,
) -> LauncherResult<u32> {
    if state.launching.swap(true, Ordering::SeqCst) {
        return Err(LauncherError::InvalidLaunchConfig(
            "Un lancement est déjà en cours.".into(),
        ));
    }
    let res = run_launch(&app, &window, &state, &req).await;
    state.launching.store(false, Ordering::SeqCst);
    if let Err(err) = &res {
        emit_log(&app, &state.logs, LogLevel::Error, "launch", err.to_string());
    }
    res
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
    let vanilla = installer.install_vanilla(&req.mc_version).await?;

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
        None => installer.install_java(&vanilla).await?,
    };

    // 3. Forge
    let use_forge = req.loader_enable && req.loader_type.eq_ignore_ascii_case("forge");
    if use_forge {
        installer
            .install_forge(&req.mc_version, &req.loader_build, &java_path, &vanilla)
            .await?;
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
        },
    );
    let mut ignored = req.ignored.clone();
    ignored.push("launcher_config".into());
    match state
        .panel_client
        .fetch_modpack_manifest(req.instance.as_deref(), ch)
        .await
    {
        Ok(manifest) => {
            let app_m = app.clone();
            let report = sync_files(
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
                        },
                    );
                },
            )
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
            format!("fml.toml : earlyWindowControl=false {}", if ok { "appliqué" } else { "NON appliqué" }),
        );
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
    let pid = spawn_resolved_forge_with_exit(
        &java_path.to_string_lossy(),
        &game_dir,
        &plan,
        state.logs.clone(),
        move |entry| {
            let _ = app_log.emit("launcher://log", entry);
        },
        move |code| {
            let entry = logs_exit.push(
                if code == Some(0) { LogLevel::Info } else { LogLevel::Warn },
                "launch",
                format!("Minecraft fermé (code {}).", code.map(|c| c.to_string()).unwrap_or_else(|| "?".into())),
            );
            let _ = app_exit.emit("launcher://log", entry);
            let _ = app_exit.emit("launcher://game-exit", code);
            if after_exit == "hide" {
                let _ = win_exit.show();
                let _ = win_exit.set_focus();
            }
        },
    )
    .await?;

    emit_log(app, &state.logs, LogLevel::Info, "launch", format!("Minecraft démarré (PID {pid})."));
    match after.as_str() {
        "hide" => {
            let _ = window.hide();
        }
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
        .panel_client
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
    let manifest = state.panel_client.fetch_modpack_manifest(instance.as_deref(), ch).await?;
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
