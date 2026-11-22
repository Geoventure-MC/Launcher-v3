use crate::error::{LauncherError, LauncherResult};
use crate::java::is_valid_java_binary_syntax;
use crate::logs::{LogEntry, LogLevel, LogStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub const MAX_RAM_SHARE: f64 = 0.60;
pub const MODPACK_MIN_GB: u32 = 3;
pub const UTF8_ARG: &str = "-Dfile.encoding=UTF-8";
pub const FORGE_1_20_1_MAIN_CLASS: &str = "cpw.mods.bootstraplauncher.BootstrapLauncher";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PerformanceProfileId {
    Eco,
    Balanced,
    Performance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformancePlan {
    pub id: PerformanceProfileId,
    pub ram_min_gb: Option<u32>,
    pub ram_max_gb: Option<u32>,
    pub jvm_args: Vec<String>,
    pub render_distance: u32,
    pub capped: bool,
    pub insufficient: bool,
    pub cap_gb: u32,
}

pub fn ram_cap_gb(total_gb: f64) -> u32 {
    (total_gb.max(0.0) * MAX_RAM_SHARE).floor() as u32
}

pub fn launch_ram_max_gb(requested_gb: u32, total_gb: f64) -> u32 {
    let physical_safe = (total_gb - 1.0).floor().max(1.0) as u32;
    let cap = ram_cap_gb(total_gb).max(MODPACK_MIN_GB.min(physical_safe));
    let wanted = if requested_gb > 0 { requested_gb } else { cap };
    wanted.clamp(1, cap.max(1))
}

pub fn recommend_profile(total_gb: f64, cores: usize) -> PerformanceProfileId {
    if total_gb >= 16.0 && cores >= 6 {
        PerformanceProfileId::Performance
    } else if total_gb >= 8.0 && cores >= 4 {
        PerformanceProfileId::Balanced
    } else {
        PerformanceProfileId::Eco
    }
}

/// Arguments JVM compatibles Java 17 (jamais `-XX:+ZGenerational` qui exige Java 21).
pub fn jvm_args_for_profile(id: PerformanceProfileId, cores: usize, max_gb: u32) -> Vec<String> {
    let raw: &[&str] = match id {
        PerformanceProfileId::Eco => &[
            "-XX:+UseG1GC",
            "-XX:+ParallelRefProcEnabled",
            "-XX:MaxGCPauseMillis=200",
            "-XX:+DisableExplicitGC",
        ],
        PerformanceProfileId::Balanced => &[
            "-XX:+UseG1GC",
            "-XX:+ParallelRefProcEnabled",
            "-XX:MaxGCPauseMillis=130",
            "-XX:+UnlockExperimentalVMOptions",
            "-XX:+DisableExplicitGC",
            "-XX:G1NewSizePercent=30",
            "-XX:G1MaxNewSizePercent=40",
            "-XX:G1HeapRegionSize=8M",
            "-XX:G1ReservePercent=20",
            "-XX:G1HeapWastePercent=5",
            "-XX:G1MixedGCCountTarget=4",
            "-XX:InitiatingHeapOccupancyPercent=15",
            "-XX:G1MixedGCLiveThresholdPercent=90",
            "-XX:SurvivorRatio=32",
            "-XX:MaxTenuringThreshold=1",
        ],
        PerformanceProfileId::Performance => {
            if cores >= 8 && max_gb >= 6 {
                &["-XX:+UseZGC", "-XX:+DisableExplicitGC"]
            } else {
                &[
                    "-XX:+UseG1GC",
                    "-XX:+ParallelRefProcEnabled",
                    "-XX:MaxGCPauseMillis=130",
                    "-XX:+UnlockExperimentalVMOptions",
                    "-XX:+DisableExplicitGC",
                    "-XX:G1NewSizePercent=30",
                    "-XX:G1MaxNewSizePercent=40",
                    "-XX:G1HeapRegionSize=8M",
                    "-XX:G1ReservePercent=20",
                    "-XX:G1HeapWastePercent=5",
                    "-XX:G1MixedGCCountTarget=4",
                    "-XX:InitiatingHeapOccupancyPercent=15",
                    "-XX:G1MixedGCLiveThresholdPercent=90",
                    "-XX:SurvivorRatio=32",
                    "-XX:MaxTenuringThreshold=1",
                ]
            }
        }
    };
    raw.iter().map(|s| (*s).to_string()).collect()
}

pub fn build_performance_profile(
    id: PerformanceProfileId,
    total_gb: f64,
    cores: usize,
) -> PerformancePlan {
    let (target_min, target_max, render_distance) = match id {
        PerformanceProfileId::Eco => (2, 4, 8),
        PerformanceProfileId::Balanced => (3, 6, 12),
        PerformanceProfileId::Performance => (4, 8, 16),
    };
    let cap_gb = ram_cap_gb(total_gb);
    if cap_gb < MODPACK_MIN_GB {
        return PerformancePlan {
            id,
            ram_min_gb: None,
            ram_max_gb: None,
            jvm_args: Vec::new(),
            render_distance,
            capped: false,
            insufficient: true,
            cap_gb,
        };
    }

    let ram_max = target_max.min(cap_gb);
    let ram_min = target_min.min(ram_max);
    PerformancePlan {
        id,
        ram_min_gb: Some(ram_min),
        ram_max_gb: Some(ram_max),
        jvm_args: jvm_args_for_profile(id, cores, ram_max),
        render_distance,
        capped: ram_max < target_max,
        insufficient: false,
        cap_gb,
    }
}

/// Nettoie les arguments JVM : garde uniquement les flags commençant par `-`, retire `-Xms`/`-Xmx`.
pub fn sanitize_jvm_args(raw: &[String]) -> Vec<String> {
    raw.iter()
        .map(|s| s.trim())
        .filter(|s| {
            s.starts_with('-')
                && !s.to_ascii_lowercase().starts_with("-xms")
                && !s.to_ascii_lowercase().starts_with("-xmx")
                && !s.chars().any(|c| matches!(c, '\0' | '\r' | '\n'))
        })
        .map(String::from)
        .collect()
}

pub fn with_utf8(mut args: Vec<String>) -> Vec<String> {
    let has_encoding = args
        .iter()
        .any(|a| a.to_ascii_lowercase().starts_with("-dfile.encoding="));
    if !has_encoding {
        args.push(UTF8_ARG.to_string());
    }
    args
}

pub fn slugify_instance(value: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "server".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Contrat de rétrocompatibilité avec Launcher-v3 :
/// - instance par défaut sur `stable` -> `<base_data_dir>/.<data_directory>` (sans `.` sur macOS)
/// - autre instance -> `<base>/instances/<slug>`
/// - canal `test` -> `<base>/instances/<slug>-test`
pub fn resolve_game_directory(
    base_data_dir: &Path,
    data_directory_name: &str,
    instance_slug: Option<&str>,
    default_instance: &str,
    channel: &str,
) -> PathBuf {
    let clean_name = data_directory_name.trim().trim_start_matches('.');
    let root_folder = if cfg!(target_os = "macos") {
        clean_name.to_string()
    } else {
        format!(".{clean_name}")
    };
    let base = base_data_dir.join(root_folder);

    let is_test = channel.eq_ignore_ascii_case("test");
    let is_default = instance_slug
        .map(|s| s.eq_ignore_ascii_case(default_instance))
        .unwrap_or(true);

    if !is_test && is_default {
        return base;
    }

    let slug = slugify_instance(instance_slug.unwrap_or(default_instance));
    let folder = if is_test {
        format!("{slug}-test")
    } else {
        slug
    };
    base.join("instances").join(folder)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeLaunchConfig {
    pub java_bin: String,
    pub game_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub natives_dir: PathBuf,
    pub classpath: Vec<PathBuf>,
    pub mc_version: String,
    pub forge_version: String,
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

/// Génère la ligne de commande complète pour lancer Minecraft Forge 1.20.1 (ModLauncher).
pub fn build_forge_1_20_1_args(cfg: &ForgeLaunchConfig) -> LauncherResult<Vec<String>> {
    if !is_valid_java_binary_syntax(&cfg.java_bin) {
        return Err(LauncherError::InvalidLaunchConfig(format!(
            "Binaire Java invalide : {}",
            cfg.java_bin
        )));
    }
    if cfg.username.trim().is_empty() || cfg.uuid.trim().is_empty() {
        return Err(LauncherError::InvalidLaunchConfig(
            "Compte joueur incomplet (username/uuid requis)".into(),
        ));
    }
    if cfg.mc_version.trim().is_empty() || cfg.forge_version.trim().is_empty() {
        return Err(LauncherError::InvalidLaunchConfig(
            "Version Minecraft ou Forge manquante".into(),
        ));
    }

    let ram_max = cfg.ram_max_gb.max(1);
    let ram_min = cfg.ram_min_gb.clamp(1, ram_max);
    let cp_sep = if cfg!(target_os = "windows") { ";" } else { ":" };
    let classpath_joined = cfg
        .classpath
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(cp_sep);

    let mut args = Vec::<String>::new();
    args.push(format!("-Xms{}M", ram_min * 1024));
    args.push(format!("-Xmx{}M", ram_max * 1024));
    args.extend(with_utf8(sanitize_jvm_args(&cfg.extra_jvm_args)));
    args.push(format!(
        "-Djava.library.path={}",
        cfg.natives_dir.to_string_lossy()
    ));
    args.push("-Dminecraft.launcher.brand=GeoventureRust".to_string());
    args.push("-Dminecraft.launcher.version=2.0".to_string());

    // Options module-path requises par Forge 1.20.1 / Java 17
    args.push("--add-exports=java.base/sun.security.util=ALL-UNNAMED".to_string());
    args.push("--add-opens=java.base/java.util.jar=ALL-UNNAMED".to_string());
    args.push("--add-opens=java.base/java.lang.invoke=ALL-UNNAMED".to_string());

    args.push("-cp".to_string());
    args.push(classpath_joined);
    args.push(FORGE_1_20_1_MAIN_CLASS.to_string());

    // Arguments ModLauncher / Forge 1.20.1
    args.extend([
        "--launchTarget".to_string(),
        "forgeclient".to_string(),
        "--fml.forgeVersion".to_string(),
        cfg.forge_version.clone(),
        "--fml.mcVersion".to_string(),
        cfg.mc_version.clone(),
        "--fml.forgeGroup".to_string(),
        "net.minecraftforge".to_string(),
        "--username".to_string(),
        cfg.username.clone(),
        "--version".to_string(),
        format!("{}-forge-{}", cfg.mc_version, cfg.forge_version),
        "--gameDir".to_string(),
        cfg.game_dir.to_string_lossy().to_string(),
        "--assetsDir".to_string(),
        cfg.assets_dir.to_string_lossy().to_string(),
        "--assetIndex".to_string(),
        "5".to_string(),
        "--uuid".to_string(),
        cfg.uuid.clone(),
        "--accessToken".to_string(),
        cfg.access_token.clone(),
        "--userType".to_string(),
        "mojang".to_string(),
        "--versionType".to_string(),
        "release".to_string(),
    ]);

    if let (Some(w), Some(h)) = (cfg.width, cfg.height) {
        if w > 0 && h > 0 {
            args.push("--width".to_string());
            args.push(w.to_string());
            args.push("--height".to_string());
            args.push(h.to_string());
        }
    }

    if let Some(host) = cfg.server_host.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            args.push("--server".to_string());
            args.push(host.to_string());
            if let Some(port) = cfg.server_port.filter(|p| *p != 25565) {
                args.push("--port".to_string());
                args.push(port.to_string());
            }
        }
    }

    Ok(args)
}

/// Démarre le processus Minecraft Forge depuis un plan résolu (`forge.rs`) et diffuse stdout/stderr.
pub async fn spawn_resolved_forge<F>(
    java_bin: &str,
    game_dir: &Path,
    plan: &crate::forge::ResolvedForgePlan,
    logs: LogStore,
    on_log: F,
) -> LauncherResult<u32>
where
    F: Fn(LogEntry) + Send + Sync + 'static,
{
    spawn_resolved_forge_with_exit(java_bin, game_dir, plan, logs, on_log, |_| {}).await
}

/// Comme [`spawn_resolved_forge`], avec un rappel à la fermeture du jeu (code de sortie).
pub async fn spawn_resolved_forge_with_exit<F, E>(
    java_bin: &str,
    game_dir: &Path,
    plan: &crate::forge::ResolvedForgePlan,
    logs: LogStore,
    on_log: F,
    on_exit: E,
) -> LauncherResult<u32>
where
    F: Fn(LogEntry) + Send + Sync + 'static,
    E: FnOnce(Option<i32>) + Send + 'static,
{
    if !is_valid_java_binary_syntax(java_bin) {
        return Err(LauncherError::InvalidLaunchConfig(format!(
            "Binaire Java invalide : {java_bin}"
        )));
    }
    let mut full_args = Vec::with_capacity(plan.jvm_args.len() + 1 + plan.game_args.len());
    full_args.extend(plan.jvm_args.iter().cloned());
    full_args.push(plan.main_class.clone());
    full_args.extend(plan.game_args.iter().cloned());

    let mut cmd = Command::new(java_bin);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000);
    }
    cmd.args(&full_args)
        .current_dir(game_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    let pid = child.id().unwrap_or(0);
    let cb = std::sync::Arc::new(on_log);

    if let Some(stdout) = child.stdout.take() {
        let logs_out = logs.clone();
        let cb_out = std::sync::Arc::clone(&cb);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let entry = logs_out.push(LogLevel::Info, "minecraft", line);
                cb_out(entry);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let logs_err = logs.clone();
        let cb_err = std::sync::Arc::clone(&cb);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let entry = logs_err.push(LogLevel::Error, "minecraft", line);
                cb_err(entry);
            }
        });
    }

    tokio::spawn(async move {
        let code = child.wait().await.ok().and_then(|st| st.code());
        on_exit(code);
    });

    Ok(pid)
}

/// Démarre le processus Minecraft Forge et diffuse stdout/stderr dans `LogStore` et le callback temps réel.
pub async fn spawn_forge_process<F>(
    cfg: &ForgeLaunchConfig,
    logs: LogStore,
    on_log: F,
) -> LauncherResult<u32>
where
    F: Fn(LogEntry) + Send + Sync + 'static,
{
    let args = build_forge_1_20_1_args(cfg)?;
    let mut cmd = Command::new(&cfg.java_bin);
    cmd.args(&args)
        .current_dir(&cfg.game_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    let pid = child.id().unwrap_or(0);
    let cb = std::sync::Arc::new(on_log);

    if let Some(stdout) = child.stdout.take() {
        let logs_out = logs.clone();
        let cb_out = std::sync::Arc::clone(&cb);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let entry = logs_out.push(LogLevel::Info, "minecraft", line);
                cb_out(entry);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let logs_err = logs.clone();
        let cb_err = std::sync::Arc::clone(&cb);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let entry = logs_err.push(LogLevel::Error, "minecraft", line);
                cb_err(entry);
            }
        });
    }

    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_cap_and_profile_respect_60_percent_guardrail() {
        assert_eq!(ram_cap_gb(16.0), 9);
        assert_eq!(ram_cap_gb(8.0), 4);
        assert_eq!(launch_ram_max_gb(12, 16.0), 9);
        assert_eq!(recommend_profile(16.0, 8), PerformanceProfileId::Performance);
        assert_eq!(recommend_profile(8.0, 4), PerformanceProfileId::Balanced);
        assert_eq!(recommend_profile(4.0, 2), PerformanceProfileId::Eco);

        let low = build_performance_profile(PerformanceProfileId::Eco, 4.0, 2);
        assert!(low.insufficient);
        assert!(low.jvm_args.is_empty());

        let perf = build_performance_profile(PerformanceProfileId::Performance, 16.0, 8);
        assert!(!perf.insufficient);
        assert!(perf.jvm_args.contains(&"-XX:+UseZGC".to_string()));
        assert!(!perf.jvm_args.iter().any(|a| a.contains("ZGenerational")));
    }

    #[test]
    fn sanitize_jvm_args_strips_xms_xmx_and_injects_utf8() {
        let raw = vec![
            "-Xms2G".into(),
            "-Xmx8G".into(),
            "-XX:+UseG1GC".into(),
            "not-a-flag".into(),
        ];
        let clean = with_utf8(sanitize_jvm_args(&raw));
        assert_eq!(clean, vec!["-XX:+UseG1GC", "-Dfile.encoding=UTF-8"]);
    }

    #[test]
    fn game_directory_preserves_legacy_default_and_isolates_instances() {
        let appdata = Path::new("/data");
        let def = resolve_game_directory(appdata, "geoventure", Some("geoventure"), "geoventure", "stable");
        assert!(def.ends_with(".geoventure") || def.ends_with("geoventure"));

        let elandor = resolve_game_directory(appdata, "geoventure", Some("elandor"), "geoventure", "stable");
        assert!(elandor.ends_with(Path::new("instances").join("elandor")));

        let test_ch = resolve_game_directory(appdata, "geoventure", Some("geoventure"), "geoventure", "test");
        assert!(test_ch.ends_with(Path::new("instances").join("geoventure-test")));
    }

    #[test]
    fn build_forge_1_20_1_args_produces_complete_command() {
        let cfg = ForgeLaunchConfig {
            java_bin: "/usr/lib/jvm/temurin-17/bin/java".into(),
            game_dir: PathBuf::from("/games/.geoventure"),
            assets_dir: PathBuf::from("/games/.geoventure/assets"),
            natives_dir: PathBuf::from("/games/.geoventure/natives"),
            classpath: vec![PathBuf::from("/games/lib1.jar"), PathBuf::from("/games/lib2.jar")],
            mc_version: "1.20.1".into(),
            forge_version: "47.3.0".into(),
            username: "Steve".into(),
            uuid: "00000000-0000-0000-0000-000000000000".into(),
            access_token: "test-token".into(),
            ram_min_gb: 2,
            ram_max_gb: 6,
            extra_jvm_args: vec!["-XX:+UseG1GC".into()],
            server_host: Some("play.geoventure.fr".into()),
            server_port: Some(25565),
            width: Some(1280),
            height: Some(720),
        };

        let args = build_forge_1_20_1_args(&cfg).unwrap();
        assert!(args.contains(&"-Xms2048M".to_string()));
        assert!(args.contains(&"-Xmx6144M".to_string()));
        assert!(args.contains(&"-Dfile.encoding=UTF-8".to_string()));
        assert!(args.contains(&FORGE_1_20_1_MAIN_CLASS.to_string()));
        assert!(args.contains(&"--fml.forgeVersion".to_string()));
        assert!(args.contains(&"47.3.0".to_string()));
        assert!(args.contains(&"--server".to_string()));
        assert!(args.contains(&"play.geoventure.fr".to_string()));
        // Port par défaut 25565 omis volontairement
        assert!(!args.contains(&"--port".to_string()));
    }
}
