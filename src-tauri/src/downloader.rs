use crate::error::{LauncherError, LauncherResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use url::Url;

pub const DEFAULT_CONCURRENCY: usize = 12;
pub const MAX_RETRIES: usize = 2;

/// Entrée d'un fichier dans le manifeste du modpack (`launcher.geoventure.fr/data/`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// Hash SHA-1 hexadécimal (historique Azuriom / CentralCorp).
    #[serde(default)]
    pub hash: Option<String>,
    /// Hash SHA-256 hexadécimal optionnel (vérification renforcée).
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffScanReport {
    pub total: usize,
    pub valid: Vec<ManifestEntry>,
    pub missing: Vec<ManifestEntry>,
    pub corrupted: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub completed: usize,
    pub total: usize,
    pub current_file: String,
    pub downloaded_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSyncError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub total: usize,
    pub valid_count: usize,
    pub downloaded_count: usize,
    pub failed: Vec<FileSyncError>,
}

/// Résout un chemin relatif de manifeste à l'intérieur de `base_dir` en bloquant toute traversée (`..`, absolu, UNC).
pub fn resolve_safe_path(base_dir: &Path, relative: &str) -> LauncherResult<PathBuf> {
    let trimmed = relative.trim();
    if trimmed.is_empty()
        || trimmed.contains('\0')
        || trimmed.contains('\r')
        || trimmed.contains('\n')
        || trimmed.starts_with('/')
        || trimmed.starts_with('\\')
    {
        return Err(LauncherError::UnsafePath(relative.to_string()));
    }

    let normalized = trimmed.replace('\\', "/");
    let rel_path = Path::new(&normalized);
    let mut out = base_dir.to_path_buf();

    for comp in rel_path.components() {
        match comp {
            Component::Normal(part) => {
                let s = part.to_string_lossy();
                if s.contains(':') {
                    return Err(LauncherError::UnsafePath(relative.to_string()));
                }
                out.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(LauncherError::UnsafePath(relative.to_string()));
            }
        }
    }

    if out == base_dir {
        return Err(LauncherError::UnsafePath(relative.to_string()));
    }

    Ok(out)
}

pub fn is_safe_http_url(raw: &str) -> bool {
    Url::parse(raw)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

pub fn is_ignored_path(rel_path: &str, ignored: &[String]) -> bool {
    let norm = rel_path.replace('\\', "/");
    let norm = norm.trim_start_matches("./");
    ignored.iter().any(|pattern| {
        let p = pattern.replace('\\', "/");
        let p = p.trim_matches('/');
        if p.is_empty() {
            return false;
        }
        norm == p || norm.starts_with(&format!("{p}/"))
    })
}

pub async fn compute_sha1_file(path: &Path) -> LauncherResult<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub async fn compute_sha256_file(path: &Path) -> LauncherResult<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Niveau de contrôle d'un fichier déjà présent sur le disque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// Taille + empreintes (modpack, Java, librairies).
    Full,
    /// Taille seule : pour les ressources du jeu, rangées sous leur propre empreinte
    /// (`assets/objects/ab/abcdef…`) et trop nombreuses pour être relues à chaque lancement.
    SizeOnly,
}

/// Vérifie la taille, le SHA-256 (si présent) et le SHA-1 (si présent) d'un fichier local.
pub async fn verify_local_file(entry: &ManifestEntry, target: &Path) -> LauncherResult<bool> {
    verify_local_file_with(entry, target, VerifyMode::Full).await
}

pub async fn verify_local_file_with(
    entry: &ManifestEntry,
    target: &Path,
    mode: VerifyMode,
) -> LauncherResult<bool> {
    let meta = match tokio::fs::metadata(target).await {
        Ok(m) if m.is_file() => m,
        _ => return Ok(false),
    };

    if let Some(expected_size) = entry.size {
        if meta.len() != expected_size {
            return Ok(false);
        }
    }

    if mode == VerifyMode::SizeOnly && entry.size.is_some() {
        return Ok(true);
    }

    if let Some(expected_sha256) = entry.sha256.as_deref().filter(|s| !s.is_empty()) {
        let actual = compute_sha256_file(target).await?;
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            return Ok(false);
        }
    }

    if let Some(expected_sha1) = entry.hash.as_deref().filter(|s| !s.is_empty()) {
        let actual = compute_sha1_file(target).await?;
        if !actual.eq_ignore_ascii_case(expected_sha1) {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Compare le manifeste aux fichiers sur disque sans rien modifier (scan différentiel).
pub async fn diff_verify_manifest(
    manifest: &[ManifestEntry],
    game_dir: &Path,
    ignored: &[String],
) -> LauncherResult<DiffScanReport> {
    diff_verify_with(manifest, game_dir, &SyncOptions::modpack(ignored.to_vec())).await
}

/// Règles de synchronisation d'un lot de fichiers.
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub ignored: Vec<String>,
    pub verify: VerifyMode,
    /// Protège les réglages du joueur (options.txt, config/… déjà présents) et les mods
    /// désactivés (`<mod>.jar-disable`) : jamais remplacés par une mise à jour du modpack.
    pub protect_user_files: bool,
    pub concurrency: usize,
}

impl SyncOptions {
    pub fn modpack(ignored: Vec<String>) -> Self {
        Self {
            ignored,
            verify: VerifyMode::Full,
            protect_user_files: true,
            concurrency: DEFAULT_CONCURRENCY,
        }
    }

    pub fn game_files(verify: VerifyMode) -> Self {
        Self {
            ignored: Vec::new(),
            verify,
            protect_user_files: false,
            concurrency: 16,
        }
    }
}

/// Réglages du joueur (même liste que `utils/userdata.js` du launcher Electron).
pub const USER_FILES: &[&str] = &[
    "options.txt",
    "optionsof.txt",
    "optionsshaders.txt",
    "servers.dat",
    "servers.dat_old",
    "usercache.json",
    "realms_persistence.json",
    "hotbar.nbt",
];
pub const USER_DIRS: &[&str] = &[
    "saves",
    "screenshots",
    "resourcepacks",
    "shaderpacks",
    "logs",
    "crash-reports",
    "schematics",
    "journeymap",
    "xaero",
    "XaeroWorldMap",
    "XaeroWaypoints",
    "backups",
    "replay_recordings",
    "instances",
];

/// Ce chemin du modpack appartient-il au joueur (jamais remplacé s'il existe déjà) ?
pub fn is_user_path(rel: &str) -> bool {
    let p = rel.replace('\\', "/");
    let p = p.trim_start_matches('/');
    if USER_FILES.contains(&p) || p.starts_with("config/") {
        return true;
    }
    USER_DIRS
        .iter()
        .any(|d| p == *d || p.starts_with(&format!("{d}/")))
}

fn disabled_variant(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push("-disable");
    PathBuf::from(s)
}

pub async fn diff_verify_with(
    manifest: &[ManifestEntry],
    game_dir: &Path,
    opts: &SyncOptions,
) -> LauncherResult<DiffScanReport> {
    let mut valid = Vec::new();
    let mut missing = Vec::new();
    let mut corrupted = Vec::new();

    for entry in manifest {
        if is_ignored_path(&entry.path, &opts.ignored) {
            continue;
        }
        let target = resolve_safe_path(game_dir, &entry.path)?;
        if opts.protect_user_files {
            let exists = tokio::fs::try_exists(&target).await.unwrap_or(false);
            if (exists && is_user_path(&entry.path))
                || tokio::fs::try_exists(disabled_variant(&target)).await.unwrap_or(false)
            {
                valid.push(entry.clone());
                continue;
            }
        }
        match tokio::fs::metadata(&target).await {
            Ok(meta) if meta.is_file() => {
                if verify_local_file_with(entry, &target, opts.verify).await? {
                    valid.push(entry.clone());
                } else {
                    corrupted.push(entry.clone());
                }
            }
            _ => missing.push(entry.clone()),
        }
    }

    let total = valid.len() + missing.len() + corrupted.len();
    Ok(DiffScanReport {
        total,
        valid,
        missing,
        corrupted,
    })
}

async fn download_and_verify_one(
    client: &reqwest::Client,
    entry: &ManifestEntry,
    target: &Path,
) -> LauncherResult<u64> {
    if !is_safe_http_url(&entry.url) {
        return Err(LauncherError::InvalidUrl(entry.url.clone()));
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let part_path = target.with_extension(format!(
        "{}part",
        target
            .extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));

    let mut last_err: Option<LauncherError> = None;

    for attempt in 0..=MAX_RETRIES {
        match stream_to_part_and_verify(client, entry, &part_path).await {
            Ok(bytes) => {
                if tokio::fs::try_exists(target).await.unwrap_or(false) {
                    let _ = tokio::fs::remove_file(target).await;
                }
                tokio::fs::rename(&part_path, target).await?;
                return Ok(bytes);
            }
            Err(err) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                last_err = Some(err);
                if attempt < MAX_RETRIES {
                    let backoff_ms = 250 * (attempt as u64 + 1);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| LauncherError::Internal("Échec de téléchargement".into())))
}

async fn stream_to_part_and_verify(
    client: &reqwest::Client,
    entry: &ManifestEntry,
    part_path: &Path,
) -> LauncherResult<u64> {
    let resp = client.get(&entry.url).send().await.map_err(|e| {
        if e.is_timeout() {
            LauncherError::Timeout(entry.url.clone())
        } else {
            LauncherError::Network {
                url: entry.url.clone(),
                message: e.to_string(),
            }
        }
    })?;

    if !resp.status().is_success() {
        return Err(LauncherError::HttpStatus {
            status: resp.status().as_u16(),
            url: entry.url.clone(),
        });
    }

    let mut file = tokio::fs::File::create(part_path).await?;
    let mut sha1_hasher = Sha1::new();
    let mut sha256_hasher = Sha256::new();
    let mut stream = resp.bytes_stream();
    let mut written: u64 = 0;

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| LauncherError::Network {
            url: entry.url.clone(),
            message: e.to_string(),
        })?;
        file.write_all(&chunk).await?;
        sha1_hasher.update(&chunk);
        sha256_hasher.update(&chunk);
        written += chunk.len() as u64;
    }
    file.flush().await?;
    drop(file);

    if let Some(expected_sha256) = entry.sha256.as_deref().filter(|s| !s.is_empty()) {
        let actual = hex::encode(sha256_hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            return Err(LauncherError::ChecksumMismatch {
                algorithm: "SHA-256",
                path: entry.path.clone(),
                expected: expected_sha256.to_lowercase(),
                actual,
            });
        }
    }

    if let Some(expected_sha1) = entry.hash.as_deref().filter(|s| !s.is_empty()) {
        let actual = hex::encode(sha1_hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected_sha1) {
            return Err(LauncherError::ChecksumMismatch {
                algorithm: "SHA-1",
                path: entry.path.clone(),
                expected: expected_sha1.to_lowercase(),
                actual,
            });
        }
    }

    Ok(written)
}

/// Synchronise les fichiers manquants ou corrompus du modpack en parallèle avec vérification SHA-1 / SHA-256.
pub async fn sync_modpack_parallel<F>(
    client: reqwest::Client,
    manifest: &[ManifestEntry],
    game_dir: &Path,
    ignored: &[String],
    concurrency: usize,
    on_progress: F,
) -> LauncherResult<SyncReport>
where
    F: Fn(DownloadProgress) + Send + Sync + 'static,
{
    let mut opts = SyncOptions::modpack(ignored.to_vec());
    opts.concurrency = concurrency;
    sync_files(client, manifest, game_dir, &opts, on_progress).await
}

/// Télécharge en parallèle les fichiers manquants ou altérés d'un lot (modpack, jeu, Java…).
pub async fn sync_files<F>(
    client: reqwest::Client,
    manifest: &[ManifestEntry],
    game_dir: &Path,
    opts: &SyncOptions,
    on_progress: F,
) -> LauncherResult<SyncReport>
where
    F: Fn(DownloadProgress) + Send + Sync + 'static,
{
    let concurrency = opts.concurrency;
    let scan = Box::pin(diff_verify_with(manifest, game_dir, opts)).await?;
    let mut to_download = Vec::with_capacity(scan.missing.len() + scan.corrupted.len());
    to_download.extend(scan.missing);
    to_download.extend(scan.corrupted);

    let total_dl = to_download.len();
    if total_dl == 0 {
        return Ok(SyncReport {
            total: scan.total,
            valid_count: scan.valid.len(),
            downloaded_count: 0,
            failed: Vec::new(),
        });
    }

    let sem = Arc::new(Semaphore::new(concurrency.clamp(1, 32)));
    let completed = Arc::new(AtomicUsize::new(0));
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let on_progress = Arc::new(on_progress);
    let mut join_set = JoinSet::new();

    for entry in to_download {
        let target = resolve_safe_path(game_dir, &entry.path)?;
        let sem_cloned = Arc::clone(&sem);
        let client_cloned = client.clone();
        let completed_cloned = Arc::clone(&completed);
        let bytes_cloned = Arc::clone(&downloaded_bytes);
        let cb = Arc::clone(&on_progress);

        join_set.spawn(async move {
            let _permit = sem_cloned
                .acquire_owned()
                .await
                .map_err(|e| LauncherError::Internal(e.to_string()))?;

            let res = Box::pin(download_and_verify_one(&client_cloned, &entry, &target)).await;
            let done = completed_cloned.fetch_add(1, Ordering::SeqCst) + 1;
            let total_bytes = match &res {
                Ok(n) => bytes_cloned.fetch_add(*n, Ordering::SeqCst) + *n,
                Err(_) => bytes_cloned.load(Ordering::SeqCst),
            };
            cb(DownloadProgress {
                completed: done,
                total: total_dl,
                current_file: entry.path.clone(),
                downloaded_bytes: total_bytes,
            });
            Ok::<(String, LauncherResult<u64>), LauncherError>((entry.path, res))
        });
    }

    let mut downloaded_count = 0usize;
    let mut failed = Vec::new();

    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(Ok((_, Ok(_)))) => downloaded_count += 1,
            Ok(Ok((path, Err(err)))) => failed.push(FileSyncError {
                path,
                error: err.to_string(),
            }),
            Ok(Err(err)) => failed.push(FileSyncError {
                path: "<worker>".into(),
                error: err.to_string(),
            }),
            Err(join_err) => failed.push(FileSyncError {
                path: "<task>".into(),
                error: join_err.to_string(),
            }),
        }
    }

    Ok(SyncReport {
        total: scan.total,
        valid_count: scan.valid.len(),
        downloaded_count,
        failed,
    })
}

/// Supprime de `mods/` les anciens jars qui ne sont plus dans le modpack (sinon deux versions du
/// même mod se chargent et Forge plante). Ne touche jamais : un mod désactivé (`.jar-disable`),
/// un mod communauté installé par le joueur, un chemin de la liste `ignored`. Ne fait RIEN si le
/// manifeste est vide (panel en panne qui renverrait `[]` : on ne vide pas le dossier du joueur).
pub async fn cleanup_stale_mods(
    game_dir: &Path,
    manifest: &[ManifestEntry],
    ignored: &[String],
) -> LauncherResult<Vec<String>> {
    let mut removed = Vec::new();
    if !manifest.iter().any(|e| e.path.replace('\\', "/").starts_with("mods/")) {
        return Ok(removed);
    }
    let wanted: std::collections::HashSet<String> = manifest
        .iter()
        .map(|e| e.path.replace('\\', "/").trim_start_matches("./").to_string())
        .collect();
    let community: Vec<String> = tokio::fs::read(game_dir.join("launcher_config/community_mods.json"))
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default();

    let mods_dir = game_dir.join("mods");
    let mut rd = match tokio::fs::read_dir(&mods_dir).await {
        Ok(rd) => rd,
        Err(_) => return Ok(removed),
    };
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if !ft.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".jar") {
            continue;
        }
        let rel = format!("mods/{name}");
        if wanted.contains(&rel) || community.contains(&name) || is_ignored_path(&rel, ignored) {
            continue;
        }
        if tokio::fs::remove_file(entry.path()).await.is_ok() {
            removed.push(rel);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn resolve_safe_path_blocks_traversal_and_absolute_paths() {
        let base = Path::new("/tmp/geoventure");
        assert!(resolve_safe_path(base, "mods/create-1.20.1.jar").is_ok());
        assert!(resolve_safe_path(base, "../evil.jar").is_err());
        assert!(resolve_safe_path(base, "mods/../../evil.jar").is_err());
        assert!(resolve_safe_path(base, "/etc/passwd").is_err());
        assert!(resolve_safe_path(base, "C:\\Windows\\system32\\cmd.exe").is_err());
        assert!(resolve_safe_path(base, "").is_err());
    }

    #[tokio::test]
    async fn cleanup_removes_only_stale_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::create_dir_all(tmp.path().join("launcher_config")).unwrap();
        for f in ["keep.jar", "old.jar", "mine.jar", "opt.jar-disable", "notes.txt"] {
            std::fs::write(mods.join(f), b"x").unwrap();
        }
        std::fs::write(
            tmp.path().join("launcher_config/community_mods.json"),
            br#"["mine.jar"]"#,
        )
        .unwrap();
        let entry = |p: &str| ManifestEntry {
            path: p.into(),
            url: "https://x/y".into(),
            size: None,
            hash: None,
            sha256: None,
        };
        // Manifeste vide : rien n'est supprimé.
        assert!(cleanup_stale_mods(tmp.path(), &[], &[]).await.unwrap().is_empty());
        let removed = cleanup_stale_mods(tmp.path(), &[entry("mods/keep.jar")], &[])
            .await
            .unwrap();
        assert_eq!(removed, vec!["mods/old.jar".to_string()]);
        assert!(mods.join("keep.jar").exists());
        assert!(mods.join("mine.jar").exists());
        assert!(mods.join("opt.jar-disable").exists());
    }

    #[tokio::test]
    async fn user_files_and_disabled_mods_are_never_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::create_dir_all(tmp.path().join("mods")).unwrap();
        std::fs::write(tmp.path().join("config/geo.toml"), b"perso").unwrap();
        std::fs::write(tmp.path().join("mods/map.jar-disable"), b"x").unwrap();
        let entry = |p: &str| ManifestEntry {
            path: p.into(),
            url: "https://x/y".into(),
            size: Some(999),
            hash: Some("00".into()),
            sha256: None,
        };
        let manifest = vec![entry("config/geo.toml"), entry("mods/map.jar"), entry("mods/new.jar")];
        let scan = diff_verify_manifest(&manifest, tmp.path(), &[]).await.unwrap();
        assert_eq!(scan.valid.len(), 2);
        assert_eq!(scan.missing.len(), 1);
        assert_eq!(scan.missing[0].path, "mods/new.jar");
    }

    #[test]
    fn ignored_paths_respect_folder_boundaries() {
        let ignored = vec!["launcher_config".to_string(), "options.txt".to_string()];
        assert!(is_ignored_path("launcher_config/community_mods.json", &ignored));
        assert!(is_ignored_path("options.txt", &ignored));
        assert!(!is_ignored_path("launcher_config_other/foo.jar", &ignored));
        assert!(!is_ignored_path("mods/geoventure.jar", &ignored));
    }

    #[tokio::test]
    async fn parallel_sync_downloads_missing_and_verifies_sha1_and_sha256() {
        let payload = b"geoventure-mod-binary-payload-v1";
        let expected_sha1 = hex::encode(Sha1::digest(payload));
        let expected_sha256 = hex::encode(Sha256::digest(payload));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = socket.read(&mut req_buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(payload).await.unwrap();
        });

        let tmp = tempfile::tempdir().unwrap();
        let manifest = vec![ManifestEntry {
            path: "mods/test-mod.jar".to_string(),
            url: format!("http://{addr}/mods/test-mod.jar"),
            size: Some(payload.len() as u64),
            hash: Some(expected_sha1),
            sha256: Some(expected_sha256),
        }];

        let client = reqwest::Client::new();
        let report = sync_modpack_parallel(client, &manifest, tmp.path(), &[], 4, |_| {})
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(report.total, 1);
        assert_eq!(report.downloaded_count, 1);
        assert!(report.failed.is_empty());

        let scan_after = diff_verify_manifest(&manifest, tmp.path(), &[])
            .await
            .unwrap();
        assert_eq!(scan_after.valid.len(), 1);
        assert!(scan_after.missing.is_empty());
        assert!(scan_after.corrupted.is_empty());
    }
}
