use crate::error::{LauncherError, LauncherResult};
use crate::launch::{sanitize_jvm_args, with_utf8};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct VersionJson {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: Option<String>,
    #[serde(rename = "inheritsFrom")]
    inherits_from: Option<String>,
    #[serde(rename = "assetIndex")]
    asset_index: Option<AssetIndex>,
    #[serde(default)]
    libraries: Vec<LibraryEntry>,
    #[serde(default)]
    arguments: Option<ArgumentsSection>,
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ArgumentsSection {
    #[serde(default)]
    game: Vec<serde_json::Value>,
    #[serde(default)]
    jvm: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LibraryEntry {
    name: String,
    #[serde(default)]
    downloads: Option<LibraryDownloads>,
    #[serde(default)]
    rules: Vec<OsRule>,
}

#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    artifact: Option<LibraryArtifact>,
}

#[derive(Debug, Deserialize)]
struct LibraryArtifact {
    path: String,
}

#[derive(Debug, Deserialize)]
struct OsRule {
    action: String,
    os: Option<OsRuleTarget>,
}

#[derive(Debug, Deserialize)]
struct OsRuleTarget {
    name: Option<String>,
}

/// Résultat résolu prêt à lancer Minecraft Forge 1.20.1 depuis un dossier `.Nexus` / `.geoventure`.
#[derive(Debug, Clone)]
pub struct ResolvedForgePlan {
    pub main_class: String,
    pub version_id: String,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub classpath: Vec<PathBuf>,
    pub missing_libraries: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ForgeResolveRequest {
    pub game_dir: PathBuf,
    pub mc_version: String,
    pub loader_build: String,
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub ram_min_gb: u32,
    pub ram_max_gb: u32,
    pub extra_jvm_args: Vec<String>,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Extrait la version Forge pure (ex: `"1.20.1-47.4.23"` -> `"47.4.23"`).
pub fn normalize_forge_version(mc_version: &str, loader_build: &str) -> String {
    let trimmed = loader_build.trim();
    let prefix = format!("{mc_version}-");
    if let Some(stripped) = trimmed.strip_prefix(&prefix) {
        stripped.to_string()
    } else if let Some(stripped) = trimmed.strip_prefix(&format!("{mc_version}-forge-")) {
        stripped.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn forge_version_id(mc_version: &str, loader_build: &str) -> String {
    let forge_ver = normalize_forge_version(mc_version, loader_build);
    format!("{mc_version}-forge-{forge_ver}")
}

fn current_mojang_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn library_allowed_for_current_os(rules: &[OsRule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let os = current_mojang_os();
    let mut allowed = false;
    for rule in rules {
        let matches_os = match &rule.os {
            None => true,
            Some(target) => target.name.as_deref().map(|n| n == os).unwrap_or(true),
        };
        if matches_os {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

/// Convertit une coordonnée Maven `group:artifact:version[:classifier]` en chemin relatif `.jar`.
pub fn maven_coord_to_rel_path(coord: &str) -> Option<String> {
    let parts: Vec<&str> = coord.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let group_path = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    let filename = if parts.len() >= 4 {
        format!("{artifact}-{version}-{}.jar", parts[3])
    } else {
        format!("{artifact}-{version}.jar")
    };
    Some(format!("{group_path}/{artifact}/{version}/{filename}"))
}

fn maven_key(coord: &str) -> String {
    let parts: Vec<&str> = coord.split(':').collect();
    if parts.len() >= 4 {
        format!("{}:{}:{}", parts[0], parts[1], parts[3])
    } else if parts.len() >= 2 {
        format!("{}:{}", parts[0], parts[1])
    } else {
        coord.to_string()
    }
}

fn extract_string_args(values: &[serde_json::Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

fn find_forge_json(game_dir: &Path, version_id: &str) -> Option<PathBuf> {
    let candidates = [
        game_dir
            .join("loader")
            .join("forge")
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json")),
        game_dir
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json")),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn resolve_library_on_disk(game_dir: &Path, rel_path: &str) -> Option<PathBuf> {
    let forge_lib = game_dir.join("loader").join("forge").join("libraries").join(rel_path);
    if forge_lib.is_file() {
        return Some(forge_lib);
    }
    let main_lib = game_dir.join("libraries").join(rel_path);
    if main_lib.is_file() {
        return Some(main_lib);
    }
    None
}

fn substitute_vars(input: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = input.to_string();
    for (k, v) in vars {
        let token = format!("${{{k}}}");
        if out.contains(&token) {
            out = out.replace(&token, v);
        }
    }
    out
}

/// Résout le classpath complet et tous les arguments JVM + jeu depuis les JSON Vanilla + Forge installés dans `game_dir`.
pub fn resolve_installed_forge_plan(req: &ForgeResolveRequest) -> LauncherResult<ResolvedForgePlan> {
    let version_id = forge_version_id(&req.mc_version, &req.loader_build);
    let forge_json_path = find_forge_json(&req.game_dir, &version_id).ok_or_else(|| {
        LauncherError::InvalidLaunchConfig(format!(
            "Profil Forge introuvable : {version_id}.json dans {}",
            req.game_dir.display()
        ))
    })?;

    let forge_raw = std::fs::read_to_string(&forge_json_path)?;
    let forge_json: VersionJson = serde_json::from_str(&forge_raw)?;

    let parent_id = forge_json
        .inherits_from
        .clone()
        .unwrap_or_else(|| req.mc_version.clone());
    let vanilla_json_path = req
        .game_dir
        .join("versions")
        .join(&parent_id)
        .join(format!("{parent_id}.json"));
    let vanilla_json: Option<VersionJson> = if vanilla_json_path.is_file() {
        let raw = std::fs::read_to_string(&vanilla_json_path)?;
        Some(serde_json::from_str(&raw)?)
    } else {
        None
    };

    // Résolution des librairies : Forge prioritaire sur Vanilla pour une même coordonnée Maven
    let mut seen_keys = HashSet::<String>::new();
    let mut classpath = Vec::<PathBuf>::new();
    let mut missing_libraries = Vec::<String>::new();

    let mut collect_libs = |libs: &[LibraryEntry]| {
        for lib in libs {
            if !library_allowed_for_current_os(&lib.rules) {
                continue;
            }
            let key = maven_key(&lib.name);
            if !seen_keys.insert(key) {
                continue;
            }
            let rel = lib
                .downloads
                .as_ref()
                .and_then(|d| d.artifact.as_ref())
                .map(|a| a.path.clone())
                .or_else(|| maven_coord_to_rel_path(&lib.name));

            if let Some(rel_path) = rel {
                if let Some(found) = resolve_library_on_disk(&req.game_dir, &rel_path) {
                    classpath.push(found);
                } else {
                    missing_libraries.push(rel_path);
                }
            }
        }
    };

    collect_libs(&forge_json.libraries);
    if let Some(v) = &vanilla_json {
        collect_libs(&v.libraries);
    }

    let client_jar = req
        .game_dir
        .join("versions")
        .join(&parent_id)
        .join(format!("{parent_id}.jar"));
    if client_jar.is_file() {
        classpath.push(client_jar);
    }

    let cp_sep = if cfg!(target_os = "windows") { ";" } else { ":" };
    let classpath_str = classpath
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(cp_sep);

    // Dossier des librairies Forge (utilisé par -DlibraryDirectory et -p dans le JSON Forge)
    let forge_libs_dir = req.game_dir.join("loader").join("forge").join("libraries");
    let lib_dir = if forge_libs_dir.is_dir() {
        forge_libs_dir
    } else {
        req.game_dir.join("libraries")
    };
    let natives_dir = req
        .game_dir
        .join("versions")
        .join(&parent_id)
        .join("natives");
    let assets_dir = req.game_dir.join("assets");
    let asset_index = vanilla_json
        .as_ref()
        .and_then(|v| v.asset_index.as_ref())
        .map(|a| a.id.clone())
        .unwrap_or_else(|| "5".to_string());

    let mut vars = HashMap::<&str, String>::new();
    vars.insert("library_directory", lib_dir.to_string_lossy().to_string());
    vars.insert("classpath_separator", cp_sep.to_string());
    // Le jar vanilla du classpath est `versions/<mc>/<mc>.jar` : `${version_name}.jar` doit le
    // désigner pour que Forge l'exclue (`-DignoreList`), sinon deux modules « minecraft » se
    // chargent et le jeu plante. Même valeur que l'ancien launcher (minecraft-java-core).
    vars.insert("version_name", parent_id.clone());
    vars.insert("natives_directory", natives_dir.to_string_lossy().to_string());
    vars.insert("launcher_name", "GeoventureRust".to_string());
    vars.insert("launcher_version", "2.0".to_string());
    vars.insert("classpath", classpath_str.clone());
    vars.insert("auth_player_name", req.username.clone());
    vars.insert("auth_uuid", req.uuid.clone());
    vars.insert("auth_access_token", req.access_token.clone());
    vars.insert("game_directory", req.game_dir.to_string_lossy().to_string());
    vars.insert("assets_root", assets_dir.to_string_lossy().to_string());
    vars.insert("assets_index_name", asset_index);
    vars.insert("user_type", "mojang".to_string());
    vars.insert("version_type", "release".to_string());
    vars.insert("auth_session", req.access_token.clone());
    vars.insert("auth_xuid", req.access_token.clone());
    vars.insert("clientid", req.access_token.clone());
    vars.insert("user_properties", "{}".to_string());
    vars.insert("game_assets", assets_dir.to_string_lossy().to_string());

    let ram_max = req.ram_max_gb.max(1);
    let ram_min = req.ram_min_gb.clamp(1, ram_max);

    let mut jvm_args = vec![
        format!("-Xms{}M", ram_min * 1024),
        format!("-Xmx{}M", ram_max * 1024),
    ];
    jvm_args.extend(with_utf8(sanitize_jvm_args(&req.extra_jvm_args)));
    let natives = natives_dir.to_string_lossy().to_string();
    jvm_args.push(format!("-Djava.library.path={natives}"));
    jvm_args.push(format!("-Djna.tmpdir={natives}"));
    jvm_args.push(format!("-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}"));
    jvm_args.push(format!("-Dio.netty.native.workdir={natives}"));
    jvm_args.push("-Dfml.ignoreInvalidMinecraftCertificates=true".to_string());
    jvm_args.push("-Dminecraft.launcher.brand=Nexus".to_string());
    jvm_args.push("-Dminecraft.launcher.version=2.0".to_string());
    if cfg!(target_os = "macos") {
        jvm_args.push("-XstartOnFirstThread".to_string());
    } else if cfg!(target_os = "windows") {
        jvm_args.push(
            "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"
                .to_string(),
        );
    } else {
        jvm_args.push("-Xss1M".to_string());
    }

    if let Some(args_sec) = &forge_json.arguments {
        for raw in extract_string_args(&args_sec.jvm) {
            jvm_args.push(substitute_vars(&raw, &vars));
        }
    }
    jvm_args.push("-cp".to_string());
    jvm_args.push(classpath_str);

    let mut game_args = Vec::<String>::new();
    if let Some(v) = &vanilla_json {
        if let Some(args_sec) = &v.arguments {
            for raw in extract_string_args(&args_sec.game) {
                game_args.push(substitute_vars(&raw, &vars));
            }
        }
    }
    if game_args.is_empty() {
        game_args.extend([
            "--username".into(),
            req.username.clone(),
            "--version".into(),
            forge_json.id.clone(),
            "--gameDir".into(),
            req.game_dir.to_string_lossy().to_string(),
            "--assetsDir".into(),
            assets_dir.to_string_lossy().to_string(),
            "--assetIndex".into(),
            vars.get("assets_index_name").cloned().unwrap_or_else(|| "5".into()),
            "--uuid".into(),
            req.uuid.clone(),
            "--accessToken".into(),
            req.access_token.clone(),
            "--userType".into(),
            "mojang".into(),
            "--versionType".into(),
            "release".into(),
        ]);
    }
    if let Some(args_sec) = &forge_json.arguments {
        for raw in extract_string_args(&args_sec.game) {
            game_args.push(substitute_vars(&raw, &vars));
        }
    }

    if let (Some(w), Some(h)) = (req.width, req.height) {
        if w > 0 && h > 0 {
            game_args.push("--width".into());
            game_args.push(w.to_string());
            game_args.push("--height".into());
            game_args.push(h.to_string());
        }
    }

    if let Some(host) = req.server_host.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            game_args.push("--server".into());
            game_args.push(host.to_string());
            if let Some(port) = req.server_port.filter(|p| *p != 25565) {
                game_args.push("--port".into());
                game_args.push(port.to_string());
            }
        }
    }

    let main_class = forge_json
        .main_class
        .unwrap_or_else(|| "cpw.mods.bootstraplauncher.BootstrapLauncher".to_string());

    Ok(ResolvedForgePlan {
        main_class,
        version_id: forge_json.id,
        jvm_args,
        game_args,
        classpath,
        missing_libraries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_forge_version_handles_both_panel_formats() {
        assert_eq!(normalize_forge_version("1.20.1", "1.20.1-47.4.23"), "47.4.23");
        assert_eq!(normalize_forge_version("1.20.1", "47.4.23"), "47.4.23");
        assert_eq!(
            forge_version_id("1.20.1", "1.20.1-47.4.23"),
            "1.20.1-forge-47.4.23"
        );
    }

    #[test]
    fn resolve_installed_forge_plan_reads_nexus_layout_and_substitutes_vars() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let forge_ver_dir = root.join("loader/forge/versions/1.20.1-forge-47.4.23");
        std::fs::create_dir_all(&forge_ver_dir).unwrap();
        let lib_path = root.join("loader/forge/libraries/cpw/mods/securejarhandler/2.1.10/securejarhandler-2.1.10.jar");
        std::fs::create_dir_all(lib_path.parent().unwrap()).unwrap();
        std::fs::write(&lib_path, b"jar").unwrap();

        let forge_json = r#"{
            "id": "1.20.1-forge-47.4.23",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "inheritsFrom": "1.20.1",
            "arguments": {
                "jvm": ["-DlibraryDirectory=${library_directory}", "-DignoreList=client-extra,${version_name}.jar", "--add-modules", "ALL-MODULE-PATH"],
                "game": ["--launchTarget", "forgeclient", "--fml.forgeVersion", "47.4.23"]
            },
            "libraries": [
                {
                    "name": "cpw.mods:securejarhandler:2.1.10",
                    "downloads": {
                        "artifact": {
                            "path": "cpw/mods/securejarhandler/2.1.10/securejarhandler-2.1.10.jar"
                        }
                    }
                }
            ]
        }"#;
        std::fs::write(forge_ver_dir.join("1.20.1-forge-47.4.23.json"), forge_json).unwrap();

        let plan = resolve_installed_forge_plan(&ForgeResolveRequest {
            game_dir: root.to_path_buf(),
            mc_version: "1.20.1".into(),
            loader_build: "1.20.1-47.4.23".into(),
            username: "GeoventurePlayer".into(),
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            access_token: "az-token".into(),
            ram_min_gb: 3,
            ram_max_gb: 6,
            extra_jvm_args: vec!["-XX:+UseG1GC".into()],
            server_host: Some("play.geoventure.fr".into()),
            server_port: Some(2086),
            width: Some(1280),
            height: Some(720),
        })
        .unwrap();

        assert_eq!(plan.main_class, "cpw.mods.bootstraplauncher.BootstrapLauncher");
        assert_eq!(plan.classpath.len(), 1);
        assert!(plan.missing_libraries.is_empty());
        assert!(plan.jvm_args.iter().any(|a| a.starts_with("-DlibraryDirectory=") && !a.contains("${")));
        assert!(plan.jvm_args.contains(&"-DignoreList=client-extra,1.20.1.jar".to_string()));
        assert!(plan.game_args.contains(&"--port".to_string()));
        assert!(plan.game_args.contains(&"2086".to_string()));
    }
}
