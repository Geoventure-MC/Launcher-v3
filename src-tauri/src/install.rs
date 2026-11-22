//! Installation complète du jeu sur un PC neuf : Java 17, Minecraft vanilla, Forge.
//!
//! La disposition des fichiers est celle de `minecraft-java-core` (launcher Electron), pour que
//! les joueurs qui passent d'un launcher à l'autre ne re-téléchargent rien :
//!
//! ```text
//! <dossier de jeu>/
//!   runtime/jre-<version>-<plateforme>/bin/java(w)     Java fourni par Mojang
//!   versions/1.20.1/1.20.1.json + 1.20.1.jar           Minecraft vanilla
//!   libraries/…                                        librairies vanilla
//!   assets/indexes/<id>.json, assets/objects/ab/abcd…  ressources
//!   loader/forge/libraries/…                           librairies et jars patchés de Forge
//!   loader/forge/versions/1.20.1-forge-X/…json         profil Forge
//! ```

use crate::downloader::{
    resolve_safe_path, sync_files, DownloadProgress, ManifestEntry, SyncOptions, SyncReport,
    VerifyMode,
};
use crate::error::{LauncherError, LauncherResult};
use crate::forge::{forge_version_id, normalize_forge_version};
use crate::logs::LogLevel;
use serde::Serialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const JAVA_RUNTIMES_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
pub const RESOURCES_URL: &str = "https://resources.download.minecraft.net";
pub const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
/// Un processeur Forge qui n'écrit plus rien pendant ce délai est arrêté (même garde-fou que
/// le correctif de l'ancien launcher, où le patch « restait à l'infini »).
pub const PROCESSOR_IDLE_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    /// `java`, `minecraft`, `assets`, `forge`, `forge-patch`, `modpack`.
    pub phase: String,
    pub label: String,
    pub completed: usize,
    pub total: usize,
    pub current_file: String,
}

pub type ProgressFn = Arc<dyn Fn(InstallProgress) + Send + Sync>;
pub type LogFn = Arc<dyn Fn(LogLevel, String) + Send + Sync>;

#[derive(Clone)]
pub struct Installer {
    pub http: reqwest::Client,
    pub game_dir: PathBuf,
    pub on_progress: ProgressFn,
    pub on_log: LogFn,
}

// ---------------------------------------------------------------------------------------------
// Outils communs
// ---------------------------------------------------------------------------------------------

pub fn mojang_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Évalue les règles `rules` d'une librairie ou d'un argument Mojang pour ce PC.
/// Une règle `features` (démo, résolution personnalisée…) ne s'applique jamais ici.
pub fn rules_allow(rules: Option<&Value>) -> bool {
    let Some(rules) = rules.and_then(Value::as_array) else {
        return true;
    };
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.get("features").is_some() {
            continue;
        }
        let mut matches = true;
        if let Some(os) = rule.get("os") {
            if let Some(name) = os.get("name").and_then(Value::as_str) {
                matches &= name == mojang_os();
            }
            if let Some(arch) = os.get("arch").and_then(Value::as_str) {
                let here = if cfg!(target_pointer_width = "32") { "x86" } else { "x64" };
                matches &= arch == here || (arch == "arm64" && cfg!(target_arch = "aarch64"));
            }
        }
        if matches {
            allowed = rule.get("action").and_then(Value::as_str) == Some("allow");
        }
    }
    allowed
}

/// `group:artifact:version[:classifier][@ext]` → `group/…/artifact/version/artifact-version[-classifier].ext`.
pub fn maven_path(coord: &str) -> Option<String> {
    let (main, ext) = match coord.split_once('@') {
        Some((m, e)) => (m, e),
        None => (coord, "jar"),
    };
    let parts: Vec<&str> = main.split(':').collect();
    if parts.len() < 3 || parts.iter().any(|p| p.is_empty() || p.contains("..")) {
        return None;
    }
    let group = parts[0].replace('.', "/");
    let (artifact, version) = (parts[1], parts[2]);
    let file = match parts.get(3) {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!("{group}/{artifact}/{version}/{file}"))
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

pub fn sha1_file_sync(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

fn entry(path: String, url: &str, sha1: Option<&str>, size: Option<u64>) -> ManifestEntry {
    ManifestEntry {
        path,
        url: url.to_string(),
        size,
        hash: sha1.map(str::to_string),
        sha256: None,
    }
}

fn artifact_entry(prefix: &str, artifact: &Value) -> Option<ManifestEntry> {
    let path = artifact.get("path")?.as_str()?;
    let url = artifact.get("url")?.as_str()?;
    if url.is_empty() {
        return None;
    }
    Some(entry(
        format!("{prefix}{path}"),
        url,
        artifact.get("sha1").and_then(Value::as_str),
        artifact.get("size").and_then(Value::as_u64),
    ))
}

impl Installer {
    fn progress(&self, phase: &str, label: &str, p: &DownloadProgress) {
        (self.on_progress)(InstallProgress {
            phase: phase.into(),
            label: label.into(),
            completed: p.completed,
            total: p.total,
            current_file: p.current_file.clone(),
        });
    }

    fn step(&self, phase: &str, label: &str) {
        (self.on_progress)(InstallProgress {
            phase: phase.into(),
            label: label.into(),
            completed: 0,
            total: 0,
            current_file: String::new(),
        });
    }

    fn log(&self, level: LogLevel, msg: impl Into<String>) {
        (self.on_log)(level, msg.into());
    }

    async fn sync(
        &self,
        base: &Path,
        files: &[ManifestEntry],
        mode: VerifyMode,
        phase: &'static str,
        label: &'static str,
    ) -> LauncherResult<SyncReport> {
        let me = self.clone();
        let report = Box::pin(sync_files(
            self.http.clone(),
            files,
            base,
            &SyncOptions::game_files(mode),
            move |p| me.progress(phase, label, &p),
        ))
        .await?;
        if let Some(first) = report.failed.first() {
            return Err(LauncherError::Internal(format!(
                "{label} : {} fichier(s) n'ont pas pu être téléchargés (ex. {} : {}). Vérifiez votre connexion puis relancez.",
                report.failed.len(),
                first.path,
                first.error
            )));
        }
        Ok(report)
    }

    async fn get_bytes(&self, url: &str) -> LauncherResult<Vec<u8>> {
        let resp = self.http.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                LauncherError::Timeout(url.to_string())
            } else {
                LauncherError::Network {
                    url: url.to_string(),
                    message: e.to_string(),
                }
            }
        })?;
        if !resp.status().is_success() {
            return Err(LauncherError::HttpStatus {
                status: resp.status().as_u16(),
                url: url.to_string(),
            });
        }
        let bytes = resp.bytes().await.map_err(|e| LauncherError::Network {
            url: url.to_string(),
            message: e.to_string(),
        })?;
        Ok(bytes.to_vec())
    }

    async fn get_json(&self, url: &str) -> LauncherResult<Value> {
        let bytes = self.get_bytes(url).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    // -----------------------------------------------------------------------------------------
    // Minecraft vanilla
    // -----------------------------------------------------------------------------------------

    /// Télécharge (ou lit hors ligne) le JSON de la version, puis la librairie, le client et les
    /// ressources manquants. Renvoie le JSON de la version.
    pub async fn install_vanilla(&self, mc_version: &str) -> LauncherResult<Value> {
        if !mc_version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(LauncherError::InvalidLaunchConfig(format!(
                "Version de Minecraft invalide : {mc_version}"
            )));
        }
        self.step("minecraft", "Recherche de Minecraft");
        let json_path = self
            .game_dir
            .join("versions")
            .join(mc_version)
            .join(format!("{mc_version}.json"));

        let version_json = match self.fetch_version_json(mc_version).await {
            Ok(v) => {
                if let Some(parent) = json_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&json_path, serde_json::to_vec(&v)?).await?;
                v
            }
            Err(err) => {
                let local = tokio::fs::read(&json_path).await.map_err(|_| err.clone_msg())?;
                self.log(
                    LogLevel::Warn,
                    format!("Serveurs Mojang injoignables ({err}) : installation locale de Minecraft {mc_version} utilisée."),
                );
                serde_json::from_slice(&local)?
            }
        };

        // Librairies + client
        let mut files = Vec::new();
        for lib in version_json["libraries"].as_array().into_iter().flatten() {
            if !rules_allow(lib.get("rules")) {
                continue;
            }
            if let Some(a) = lib.pointer("/downloads/artifact") {
                if let Some(e) = artifact_entry("libraries/", a) {
                    files.push(e);
                }
            }
        }
        if let Some(client) = version_json.pointer("/downloads/client") {
            if let Some(url) = client.get("url").and_then(Value::as_str) {
                files.push(entry(
                    format!("versions/{mc_version}/{mc_version}.jar"),
                    url,
                    client.get("sha1").and_then(Value::as_str),
                    client.get("size").and_then(Value::as_u64),
                ));
            }
        }
        // Index des ressources
        let asset_index = version_json.get("assetIndex").cloned().unwrap_or(Value::Null);
        let index_id = asset_index
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(mc_version)
            .to_string();
        if let Some(url) = asset_index.get("url").and_then(Value::as_str) {
            files.push(entry(
                format!("assets/indexes/{index_id}.json"),
                url,
                asset_index.get("sha1").and_then(Value::as_str),
                asset_index.get("size").and_then(Value::as_u64),
            ));
        }
        self.sync(&self.game_dir, &files, VerifyMode::Full, "minecraft", "Minecraft")
            .await?;

        // Ressources (≈ 3 700 fichiers rangés sous leur empreinte : contrôle de taille seulement)
        let index_path = self.game_dir.join(format!("assets/indexes/{index_id}.json"));
        let index: Value = serde_json::from_slice(&tokio::fs::read(&index_path).await?)?;
        let mut assets = Vec::new();
        let mut seen = HashSet::new();
        if let Some(objects) = index.get("objects").and_then(Value::as_object) {
            for obj in objects.values() {
                let Some(hash) = obj.get("hash").and_then(Value::as_str) else {
                    continue;
                };
                if hash.len() < 3 || !hash.chars().all(|c| c.is_ascii_hexdigit()) || !seen.insert(hash) {
                    continue;
                }
                let sub = &hash[..2];
                assets.push(entry(
                    format!("assets/objects/{sub}/{hash}"),
                    &format!("{RESOURCES_URL}/{sub}/{hash}"),
                    Some(hash),
                    obj.get("size").and_then(Value::as_u64),
                ));
            }
        }
        self.sync(&self.game_dir, &assets, VerifyMode::SizeOnly, "assets", "Ressources du jeu")
            .await?;
        self.log(
            LogLevel::Info,
            format!("Minecraft {mc_version} prêt ({} fichiers, {} ressources).", files.len(), assets.len()),
        );
        Ok(version_json)
    }

    async fn fetch_version_json(&self, mc_version: &str) -> LauncherResult<Value> {
        let manifest = self.get_json(VERSION_MANIFEST_URL).await?;
        let info = manifest["versions"]
            .as_array()
            .and_then(|v| v.iter().find(|x| x["id"].as_str() == Some(mc_version)))
            .ok_or_else(|| {
                LauncherError::InvalidLaunchConfig(format!("Minecraft {mc_version} introuvable chez Mojang"))
            })?;
        let url = info["url"].as_str().unwrap_or_default();
        let bytes = self.get_bytes(url).await?;
        if let Some(expected) = info["sha1"].as_str() {
            let actual = sha1_hex(&bytes);
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(LauncherError::ChecksumMismatch {
                    algorithm: "SHA-1",
                    path: format!("{mc_version}.json"),
                    expected: expected.into(),
                    actual,
                });
            }
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    // -----------------------------------------------------------------------------------------
    // Java
    // -----------------------------------------------------------------------------------------

    /// Installe le Java fourni par Mojang pour cette version (Java 17 « java-runtime-gamma » pour
    /// la 1.20.1) dans `runtime/`. Hors ligne, réutilise un runtime déjà présent.
    pub async fn install_java(&self, version_json: &Value) -> LauncherResult<PathBuf> {
        let component = version_json
            .pointer("/javaVersion/component")
            .and_then(Value::as_str)
            .unwrap_or("java-runtime-gamma")
            .to_string();
        let platform = mojang_java_platform().ok_or_else(|| {
            LauncherError::JavaNotFound("Plateforme non prise en charge par le Java de Mojang".into())
        })?;
        self.step("java", "Préparation de Java 17");

        match self.download_java(&component, platform).await {
            Ok(path) => Ok(path),
            Err(err) => {
                if let Some(existing) = find_existing_runtime(&self.game_dir, platform) {
                    self.log(
                        LogLevel::Warn,
                        format!("Java de Mojang injoignable ({err}) : runtime local réutilisé."),
                    );
                    Ok(existing)
                } else {
                    Err(err)
                }
            }
        }
    }

    async fn download_java(&self, component: &str, platform: &str) -> LauncherResult<PathBuf> {
        let all = self.get_json(JAVA_RUNTIMES_URL).await?;
        let runtime = all
            .pointer(&format!("/{platform}/{component}/0"))
            .ok_or_else(|| {
                LauncherError::JavaNotFound(format!("Aucun Java « {component} » pour {platform}"))
            })?;
        let version_name = runtime
            .pointer("/version/name")
            .and_then(Value::as_str)
            .unwrap_or("17")
            .to_string();
        if !version_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        {
            return Err(LauncherError::JavaNotFound(format!("Version Java invalide : {version_name}")));
        }
        let manifest_url = runtime
            .pointer("/manifest/url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let manifest = self.get_json(manifest_url).await?;
        let files_map = manifest
            .get("files")
            .and_then(Value::as_object)
            .ok_or_else(|| LauncherError::JavaNotFound("Manifeste Java vide".into()))?;

        let exe = java_exe_suffix();
        let prefix = files_map
            .keys()
            .find(|k| k.ends_with(exe))
            .map(|k| k[..k.len() - exe.len()].to_string())
            .ok_or_else(|| LauncherError::JavaNotFound("Exécutable Java absent du manifeste".into()))?;

        let base = format!("runtime/jre-{version_name}-{platform}");
        let mut files = Vec::new();
        let mut executables = Vec::new();
        for (rel, info) in files_map {
            if info.get("type").and_then(Value::as_str) != Some("file") {
                continue;
            }
            let Some(raw) = info.pointer("/downloads/raw") else { continue };
            let stripped = rel.strip_prefix(prefix.as_str()).unwrap_or(rel);
            let path = format!("{base}/{stripped}");
            if info.get("executable").and_then(Value::as_bool) == Some(true) {
                executables.push(path.clone());
            }
            if let Some(url) = raw.get("url").and_then(Value::as_str) {
                files.push(entry(
                    path,
                    url,
                    raw.get("sha1").and_then(Value::as_str),
                    raw.get("size").and_then(Value::as_u64),
                ));
            }
        }
        self.sync(&self.game_dir, &files, VerifyMode::Full, "java", "Java 17")
            .await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for rel in &executables {
                if let Ok(p) = resolve_safe_path(&self.game_dir, rel) {
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
                }
            }
        }
        #[cfg(not(unix))]
        let _ = &executables;

        let java = self.game_dir.join(&base).join(exe);
        if !java.is_file() {
            return Err(LauncherError::JavaNotFound(format!(
                "Java téléchargé mais introuvable : {}",
                java.display()
            )));
        }
        self.log(LogLevel::Info, format!("Java {version_name} prêt ({} fichiers).", files.len()));
        Ok(java)
    }

    // -----------------------------------------------------------------------------------------
    // Forge
    // -----------------------------------------------------------------------------------------

    pub fn loader_dir(&self) -> PathBuf {
        self.game_dir.join("loader").join("forge")
    }

    /// Installe Forge : installeur, librairies, puis processeurs (patch du client).
    /// Rien n'est refait si le client patché est déjà là et intact.
    pub async fn install_forge(
        &self,
        mc_version: &str,
        loader_build: &str,
        java: &Path,
        vanilla_json: &Value,
    ) -> LauncherResult<String> {
        let forge_ver = normalize_forge_version(mc_version, loader_build);
        let full = format!("{mc_version}-{forge_ver}");
        if !full
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(LauncherError::InvalidLaunchConfig(format!("Version Forge invalide : {full}")));
        }
        let id = forge_version_id(mc_version, loader_build);
        let loader_dir = self.loader_dir();
        let libs_dir = loader_dir.join("libraries");
        self.step("forge", &format!("Forge {forge_ver}"));

        // 1. Installeur
        let installer_rel = format!("libraries/net/minecraftforge/installer/forge-{full}-installer.jar");
        let installer = loader_dir.join(&installer_rel);
        if !installer.is_file() || open_zip(&installer).is_err() {
            let url = format!("{FORGE_MAVEN}/{full}/forge-{full}-installer.jar");
            let sha1 = self
                .get_bytes(&format!("{url}.sha1"))
                .await
                .ok()
                .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                .filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()));
            self.sync(
                &loader_dir,
                &[entry(installer_rel.clone(), &url, sha1.as_deref(), None)],
                VerifyMode::Full,
                "forge",
                "Installeur Forge",
            )
            .await?;
        }

        // 2. Profils
        let inst = installer.clone();
        let (profile, version): (Value, Value) = tokio::task::spawn_blocking(move || {
            let mut zip = open_zip(&inst)?;
            let profile: Value = serde_json::from_slice(&read_zip_entry(&mut zip, "install_profile.json")?)?;
            let json_name = profile
                .get("json")
                .and_then(Value::as_str)
                .unwrap_or("/version.json")
                .trim_start_matches('/')
                .to_string();
            let version: Value = serde_json::from_slice(&read_zip_entry(&mut zip, &json_name)?)?;
            Ok::<_, LauncherError>((profile, version))
        })
        .await
        .map_err(|e| LauncherError::Internal(e.to_string()))??;

        let version_dir = loader_dir.join("versions").join(&id);
        tokio::fs::create_dir_all(&version_dir).await?;
        tokio::fs::write(version_dir.join(format!("{id}.json")), serde_json::to_vec_pretty(&version)?).await?;
        let client_jar = self
            .game_dir
            .join("versions")
            .join(mc_version)
            .join(format!("{mc_version}.jar"));

        // 3. Jars embarqués dans l'installeur (maven/…) puis librairies
        let inst = installer.clone();
        let libs = libs_dir.clone();
        tokio::task::spawn_blocking(move || extract_maven_dir(&inst, &libs))
            .await
            .map_err(|e| LauncherError::Internal(e.to_string()))??;

        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for lib in profile["libraries"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(version["libraries"].as_array().into_iter().flatten())
        {
            if !rules_allow(lib.get("rules")) {
                continue;
            }
            if let Some(e) = lib.pointer("/downloads/artifact").and_then(|a| artifact_entry("libraries/", a)) {
                if seen.insert(e.path.clone()) {
                    files.push(e);
                }
            }
        }
        self.sync(&loader_dir, &files, VerifyMode::Full, "forge", "Librairies Forge")
            .await?;

        // 4. Processeurs
        self.run_processors(&profile, &installer, &client_jar, mc_version, java, vanilla_json)
            .await?;
        self.log(LogLevel::Info, format!("Forge {forge_ver} installé ({id})."));
        Ok(id)
    }

    async fn run_processors(
        &self,
        profile: &Value,
        installer: &Path,
        client_jar: &Path,
        mc_version: &str,
        java: &Path,
        vanilla_json: &Value,
    ) -> LauncherResult<()> {
        let loader_dir = self.loader_dir();
        let libs_dir = loader_dir.join("libraries");
        let tmp_dir = loader_dir.join("installer-data");

        let mut data = HashMap::<String, String>::new();
        if let Some(map) = profile.get("data").and_then(Value::as_object) {
            for (key, sides) in map {
                let Some(raw) = sides.get("client").and_then(Value::as_str) else { continue };
                let value = if let Some(coord) = raw.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    let rel = maven_path(coord).ok_or_else(|| {
                        LauncherError::InvalidLaunchConfig(format!("Coordonnée Forge invalide : {coord}"))
                    })?;
                    libs_dir.join(rel).to_string_lossy().to_string()
                } else if let Some(lit) = raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
                    lit.to_string()
                } else if let Some(inner) = raw.strip_prefix('/') {
                    let dest = resolve_safe_path(&tmp_dir, inner)?;
                    let (inst, name, d) = (installer.to_path_buf(), inner.to_string(), dest.clone());
                    tokio::task::spawn_blocking(move || -> LauncherResult<()> {
                        let mut zip = open_zip(&inst)?;
                        let bytes = read_zip_entry(&mut zip, &name)?;
                        if let Some(p) = d.parent() {
                            std::fs::create_dir_all(p)?;
                        }
                        std::fs::write(&d, bytes)?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| LauncherError::Internal(e.to_string()))??;
                    dest.to_string_lossy().to_string()
                } else {
                    raw.to_string()
                };
                data.insert(key.clone(), value);
            }
        }
        data.insert("SIDE".into(), "client".into());
        data.insert("MINECRAFT_JAR".into(), client_jar.to_string_lossy().to_string());
        data.insert("MINECRAFT_VERSION".into(), mc_version.to_string());
        data.insert("ROOT".into(), loader_dir.to_string_lossy().to_string());
        data.insert("INSTALLER".into(), installer.to_string_lossy().to_string());
        data.insert("LIBRARY_DIR".into(), libs_dir.to_string_lossy().to_string());

        let processors: Vec<&Value> = profile["processors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| {
                p.get("sides")
                    .and_then(Value::as_array)
                    .map(|s| s.iter().any(|x| x.as_str() == Some("client")))
                    .unwrap_or(true)
            })
            .collect();
        let total = processors.len();

        for (idx, proc_) in processors.into_iter().enumerate() {
            let args: Vec<String> = proc_["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|a| resolve_processor_arg(a, &data, &libs_dir))
                .collect::<LauncherResult<_>>()?;

            // Sorties déjà présentes et intactes : rien à refaire.
            let outputs: Vec<(String, String)> = proc_
                .get("outputs")
                .and_then(Value::as_object)
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| {
                            Some((
                                resolve_processor_arg(k, &data, &libs_dir).ok()?,
                                resolve_processor_arg(v.as_str()?, &data, &libs_dir).ok()?,
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let task = args
                .iter()
                .position(|a| a == "--task")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_else(|| proc_["jar"].as_str().unwrap_or("").to_string());
            let label = format!("Patch Forge {}/{} : {task}", idx + 1, total);
            (self.on_progress)(InstallProgress {
                phase: "forge-patch".into(),
                label: label.clone(),
                completed: idx,
                total,
                current_file: task.clone(),
            });

            if !outputs.is_empty() && outputs_ok(&outputs) {
                continue;
            }
            if outputs.is_empty() && processor_output_exists(&task, &args) {
                continue;
            }

            if task == "DOWNLOAD_MOJMAPS" {
                // Fait ici plutôt que par Java : ça suit le proxy et les délais du launcher.
                let out = arg_value(&args, "--output").ok_or_else(|| {
                    LauncherError::InvalidLaunchConfig("DOWNLOAD_MOJMAPS sans --output".into())
                })?;
                let maps = vanilla_json
                    .pointer("/downloads/client_mappings")
                    .ok_or_else(|| LauncherError::InvalidLaunchConfig("Mappings Mojang absents".into()))?;
                let url = maps["url"].as_str().unwrap_or_default();
                let bytes = self.get_bytes(url).await?;
                if let Some(expected) = maps["sha1"].as_str() {
                    let actual = sha1_hex(&bytes);
                    if !actual.eq_ignore_ascii_case(expected) {
                        return Err(LauncherError::ChecksumMismatch {
                            algorithm: "SHA-1",
                            path: "client_mappings.txt".into(),
                            expected: expected.into(),
                            actual,
                        });
                    }
                }
                let out = PathBuf::from(out);
                if let Some(p) = out.parent() {
                    tokio::fs::create_dir_all(p).await?;
                }
                tokio::fs::write(&out, bytes).await?;
                continue;
            }

            let jar_coord = proc_["jar"].as_str().unwrap_or_default();
            let jar = libs_dir.join(maven_path(jar_coord).ok_or_else(|| {
                LauncherError::InvalidLaunchConfig(format!("Processeur Forge invalide : {jar_coord}"))
            })?);
            let mut cp = vec![jar.clone()];
            for c in proc_["classpath"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if let Some(rel) = maven_path(c) {
                    cp.push(libs_dir.join(rel));
                }
            }
            let jar_for_main = jar.clone();
            let main_class = tokio::task::spawn_blocking(move || jar_main_class(&jar_for_main))
                .await
                .map_err(|e| LauncherError::Internal(e.to_string()))??;
            let sep = if cfg!(windows) { ";" } else { ":" };
            let cp_str = cp
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(sep);

            self.log(LogLevel::Info, label);
            let mut cmd = tokio::process::Command::new(java);
            cmd.arg("-cp")
                .arg(cp_str)
                .arg(&main_class)
                .args(&args)
                .current_dir(&loader_dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                cmd.creation_flags(0x0800_0000);
            }
            self.run_with_idle_timeout(cmd, &task).await?;

            if !outputs.is_empty() && !outputs_ok(&outputs) {
                return Err(LauncherError::InvalidLaunchConfig(format!(
                    "Le patch Forge « {task} » a produit un fichier invalide. Relancez ; si ça recommence, supprimez le dossier loader/forge."
                )));
            }
        }
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        Ok(())
    }

    async fn run_with_idle_timeout(&self, mut cmd: tokio::process::Command, task: &str) -> LauncherResult<()> {
        let mut child = cmd.spawn().map_err(|e| {
            LauncherError::InvalidLaunchConfig(format!("Impossible de démarrer Java pour le patch Forge : {e}"))
        })?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(LogLevel, String)>();
        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let _ = tx.send((LogLevel::Debug, l));
                }
            });
        }
        if let Some(err) = child.stderr.take() {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let _ = tx.send((LogLevel::Warn, l));
                }
            });
        }
        drop(tx);
        let mut tail: Vec<String> = Vec::new();
        loop {
            tokio::select! {
                status = child.wait() => {
                    while let Ok((_, l)) = rx.try_recv() { tail.push(l); }
                    let status = status?;
                    if status.success() {
                        return Ok(());
                    }
                    for l in &tail {
                        self.log(LogLevel::Warn, l.clone());
                    }
                    let last = tail.iter().rev().take(5).rev().cloned().collect::<Vec<_>>().join(" | ");
                    return Err(LauncherError::InvalidLaunchConfig(format!(
                        "Le patch Forge « {task} » a échoué (code {:?}) : {last}", status.code()
                    )));
                }
                msg = tokio::time::timeout(PROCESSOR_IDLE_TIMEOUT, rx.recv()) => {
                    match msg {
                        Ok(Some((_level, line))) => {
                            // Les processeurs écrivent des milliers de lignes (« Patching … ») : on
                            // ne garde que les dernières, montrées seulement en cas d'échec.
                            if tail.len() > 50 { tail.remove(0); }
                            tail.push(line);
                        }
                        Ok(None) => {
                            // Sorties fermées : on attend la fin du processus, avec le même délai.
                            match tokio::time::timeout(PROCESSOR_IDLE_TIMEOUT, child.wait()).await {
                                Ok(status) => {
                                    let status = status?;
                                    if status.success() { return Ok(()); }
                                    return Err(LauncherError::InvalidLaunchConfig(format!(
                                        "Le patch Forge « {task} » a échoué (code {:?}).", status.code()
                                    )));
                                }
                                Err(_) => {
                                    let _ = child.kill().await;
                                    return Err(LauncherError::Timeout(format!("patch Forge {task}")));
                                }
                            }
                        }
                        Err(_) => {
                            let _ = child.kill().await;
                            return Err(LauncherError::InvalidLaunchConfig(format!(
                                "Le patch Forge « {task} » ne répond plus depuis 3 minutes : arrêté. Relancez."
                            )));
                        }
                    }
                }
            }
        }
    }
}

/// Désactive la fenêtre de chargement anticipée de Forge (`config/fml.toml`,
/// `earlyWindowControl = false`), comme le launcher Electron : sur certains pilotes elle affichait
/// le logo Mojang rouge puis figeait. Renvoie `true` si le réglage est bien en place après relecture.
pub fn apply_forge_window_settings(game_dir: &Path) -> bool {
    let file = game_dir.join("config").join("fml.toml");
    let _ = std::fs::create_dir_all(game_dir.join("config"));
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with("earlyWindowControl") && t["earlyWindowControl".len()..].trim_start().starts_with('=') {
                found = true;
                "earlyWindowControl = false".to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push("earlyWindowControl = false".to_string());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    if out != text {
        let _ = std::fs::write(&file, &out);
    }
    std::fs::read_to_string(&file)
        .map(|t| t.lines().any(|l| l.trim() == "earlyWindowControl = false"))
        .unwrap_or(false)
}

trait CloneMsg {
    fn clone_msg(&self) -> LauncherError;
}
impl CloneMsg for LauncherError {
    fn clone_msg(&self) -> LauncherError {
        LauncherError::Internal(self.to_string())
    }
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// Les processeurs sans `outputs` (MCP_DATA, MERGE_MAPPING, patch binaire…) ne sont refaits que si
/// leur fichier `--output` manque.
fn processor_output_exists(_task: &str, args: &[String]) -> bool {
    match arg_value(args, "--output") {
        Some(out) => Path::new(&out).is_file(),
        None => false,
    }
}

fn outputs_ok(outputs: &[(String, String)]) -> bool {
    outputs.iter().all(|(file, sha)| {
        sha1_file_sync(Path::new(file))
            .map(|actual| actual.eq_ignore_ascii_case(sha))
            .unwrap_or(false)
    })
}

/// Remplace `{CLE}` par la donnée du profil et `[coordonnée]` par le chemin de la librairie.
pub fn resolve_processor_arg(
    arg: &str,
    data: &HashMap<String, String>,
    libs_dir: &Path,
) -> LauncherResult<String> {
    if let Some(coord) = arg.strip_prefix('[').and_then(|a| a.strip_suffix(']')) {
        let rel = maven_path(coord)
            .ok_or_else(|| LauncherError::InvalidLaunchConfig(format!("Coordonnée invalide : {coord}")))?;
        return Ok(libs_dir.join(rel).to_string_lossy().to_string());
    }
    if let Some(key) = arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')) {
        if let Some(v) = data.get(key) {
            return Ok(v.clone());
        }
    }
    let mut out = arg.to_string();
    for (k, v) in data {
        let token = format!("{{{k}}}");
        if out.contains(&token) {
            out = out.replace(&token, v);
        }
    }
    Ok(out)
}

pub fn mojang_java_platform() -> Option<&'static str> {
    let arch = std::env::consts::ARCH;
    match (std::env::consts::OS, arch) {
        ("windows", "x86_64") => Some("windows-x64"),
        ("windows", "x86") => Some("windows-x86"),
        ("windows", "aarch64") => Some("windows-arm64"),
        ("macos", "aarch64") => Some("mac-os-arm64"),
        ("macos", "x86_64") => Some("mac-os"),
        ("linux", "x86_64") => Some("linux"),
        ("linux", "x86") => Some("linux-i386"),
        _ => None,
    }
}

pub fn java_exe_suffix() -> &'static str {
    if cfg!(windows) {
        "bin/javaw.exe"
    } else {
        "bin/java"
    }
}

/// Runtime Java déjà installé par l'un des launchers (`runtime/jre-*-<plateforme>`).
pub fn find_existing_runtime(game_dir: &Path, platform: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(game_dir.join("runtime")).ok()?;
    let mut found: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("jre-17") || (n.starts_with("jre-") && n.ends_with(platform))
                })
                .unwrap_or(false)
        })
        .map(|p| p.join(java_exe_suffix()))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found.pop()
}

fn open_zip(path: &Path) -> LauncherResult<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path)?;
    zip::ZipArchive::new(f).map_err(|e| {
        LauncherError::InvalidLaunchConfig(format!("Archive illisible {} : {e}", path.display()))
    })
}

fn read_zip_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> LauncherResult<Vec<u8>> {
    let mut f = zip
        .by_name(name)
        .map_err(|e| LauncherError::InvalidLaunchConfig(format!("{name} absent de l'installeur Forge : {e}")))?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Copie les jars `maven/…` de l'installeur vers `libraries/` (universal, fmlcore, langages…).
fn extract_maven_dir(installer: &Path, libs_dir: &Path) -> LauncherResult<usize> {
    let mut zip = open_zip(installer)?;
    let mut count = 0;
    for i in 0..zip.len() {
        let mut f = zip
            .by_index(i)
            .map_err(|e| LauncherError::Internal(e.to_string()))?;
        let name = f.name().to_string();
        let Some(rel) = name.strip_prefix("maven/") else { continue };
        if f.is_dir() || rel.is_empty() {
            continue;
        }
        let dest = resolve_safe_path(libs_dir, rel)?;
        if dest.metadata().map(|m| m.len() == f.size()).unwrap_or(false) {
            continue;
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        std::fs::write(&dest, buf)?;
        count += 1;
    }
    Ok(count)
}

fn jar_main_class(jar: &Path) -> LauncherResult<String> {
    let mut zip = open_zip(jar)?;
    let manifest = String::from_utf8_lossy(&read_zip_entry(&mut zip, "META-INF/MANIFEST.MF")?).to_string();
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("Main-Class:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::InvalidLaunchConfig(format!("Main-Class absente de {}", jar.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maven_path_handles_classifier_and_extension() {
        assert_eq!(
            maven_path("net.minecraftforge:forge:1.20.1-47.4.28:client").unwrap(),
            "net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-client.jar"
        );
        assert_eq!(
            maven_path("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412:mappings@txt").unwrap(),
            "de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412-mappings.txt"
        );
        assert_eq!(
            maven_path("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip").unwrap(),
            "de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412.zip"
        );
        assert!(maven_path("a:b").is_none());
        assert!(maven_path("a:..:c").is_none());
    }

    #[test]
    fn rules_follow_os_and_ignore_features() {
        let here = mojang_os();
        let other = if here == "windows" { "linux" } else { "windows" };
        assert!(rules_allow(None));
        assert!(rules_allow(Some(&json!([{"action":"allow","os":{"name":here}}]))));
        assert!(!rules_allow(Some(&json!([{"action":"allow","os":{"name":other}}]))));
        assert!(!rules_allow(Some(&json!([{"action":"allow"},{"action":"disallow","os":{"name":here}}]))));
        assert!(!rules_allow(Some(&json!([{"action":"allow","features":{"is_demo_user":true}}]))));
    }

    #[test]
    fn processor_args_resolve_data_and_coordinates() {
        let libs = Path::new("/g/loader/forge/libraries");
        let mut data = HashMap::new();
        data.insert("SIDE".to_string(), "client".to_string());
        data.insert("ROOT".to_string(), "/g/loader/forge".to_string());
        assert_eq!(resolve_processor_arg("{SIDE}", &data, libs).unwrap(), "client");
        assert_eq!(resolve_processor_arg("{ROOT}/run.sh", &data, libs).unwrap(), "/g/loader/forge/run.sh");
        assert!(resolve_processor_arg("[net.minecraft:client:1.20.1:slim]", &data, libs)
            .unwrap()
            .ends_with("net/minecraft/client/1.20.1/client-1.20.1-slim.jar"));
        assert_eq!(resolve_processor_arg("--task", &data, libs).unwrap(), "--task");
    }

    #[test]
    fn fml_toml_early_window_is_disabled_without_losing_other_keys() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::write(tmp.path().join("config/fml.toml"), "maxThreads = -1\nearlyWindowControl = true\n").unwrap();
        assert!(apply_forge_window_settings(tmp.path()));
        let t = std::fs::read_to_string(tmp.path().join("config/fml.toml")).unwrap();
        assert!(t.contains("maxThreads = -1"));
        assert!(t.contains("earlyWindowControl = false"));
        assert!(!t.contains("true"));
        let empty = tempfile::tempdir().unwrap();
        assert!(apply_forge_window_settings(empty.path()));
    }

    #[test]
    fn existing_runtime_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let platform = mojang_java_platform().unwrap_or("linux");
        let exe = tmp
            .path()
            .join(format!("runtime/jre-17.0.8-{platform}"))
            .join(java_exe_suffix());
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(find_existing_runtime(tmp.path(), platform), Some(exe));
    }
}
