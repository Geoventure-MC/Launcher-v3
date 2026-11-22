//! Mods communauté, captures d'écran, packs de ressources et de shaders du dossier de jeu.

use crate::error::{LauncherError, LauncherResult};
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const COMMUNITY_FILE: &str = "launcher_config/community_mods.json";

/// Nom d'entrée d'un dossier (jamais un chemin).
pub fn safe_entry_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 200 || n.contains(['/', '\\', '\0', ':']) || n == "." || n.contains("..") {
        return None;
    }
    Some(n.to_string())
}

// ---------------------------------------------------------------- mods communauté

pub fn read_community(game_dir: &Path) -> Vec<String> {
    std::fs::read(game_dir.join(COMMUNITY_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|n| crate::commands::safe_mod_file_name(&n))
        .collect()
}

pub fn record_community(game_dir: &Path, name: &str, installed: bool) -> std::io::Result<()> {
    let mut set: std::collections::BTreeSet<String> = read_community(game_dir).into_iter().collect();
    if installed {
        set.insert(name.to_string());
    } else {
        set.remove(name);
    }
    let path = game_dir.join(COMMUNITY_FILE);
    std::fs::create_dir_all(path.parent().unwrap_or(game_dir))?;
    std::fs::write(path, serde_json::to_vec_pretty(&set.into_iter().collect::<Vec<_>>())?)
}

/// Mods communauté installés (notés ET présents dans `mods/`).
pub fn installed_community(game_dir: &Path) -> Vec<String> {
    read_community(game_dir)
        .into_iter()
        .filter(|n| game_dir.join("mods").join(n).is_file())
        .collect()
}

/// Vérifie un .jar téléchargé : archive zip, et empreinte du panel si donnée (sha256 ou sha1).
pub fn check_jar(bytes: &[u8], expected: &str) -> Result<(), String> {
    if bytes.len() < 4 || &bytes[..2] != b"PK" {
        return Err("fichier reçu invalide (pas un .jar)".into());
    }
    let e = expected.trim().to_ascii_lowercase();
    if e.is_empty() {
        return Ok(());
    }
    let got = match e.len() {
        64 => hex::encode(Sha256::digest(bytes)),
        40 => hex::encode(Sha1::digest(bytes)),
        _ => return Err("empreinte du panel illisible".into()),
    };
    if got != e {
        return Err("empreinte du fichier différente de celle du panel".into());
    }
    Ok(())
}

fn https_only(u: &reqwest::Url) -> bool {
    u.scheme() == "https" || matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// Télécharge un mod communauté dans `mods/` (https obligatoire, même après redirection).
pub async fn install_community(game_dir: &Path, url: &str, file_name: &str, expected_hash: &str) -> LauncherResult<()> {
    let name = crate::commands::safe_mod_file_name(file_name)
        .ok_or_else(|| LauncherError::InvalidUrl(format!("Nom de fichier refusé : {file_name}")))?;
    let u = reqwest::Url::parse(url).map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;
    if !https_only(&u) {
        return Err(LauncherError::InvalidUrl("téléchargement refusé : https obligatoire".into()));
    }
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() > 5 || !https_only(a.url()) {
                a.error("redirection refusée : https obligatoire")
            } else {
                a.follow()
            }
        }))
        .build()
        .map_err(|e| LauncherError::Internal(e.to_string()))?;
    let res = http.get(u).send().await.map_err(|e| LauncherError::Internal(format!("Téléchargement impossible : {e}")))?;
    if !res.status().is_success() {
        return Err(LauncherError::Internal(format!("Téléchargement refusé (HTTP {})", res.status().as_u16())));
    }
    let bytes = res.bytes().await.map_err(|e| LauncherError::Internal(e.to_string()))?;
    check_jar(&bytes, expected_hash).map_err(LauncherError::Internal)?;
    let mods = game_dir.join("mods");
    tokio::fs::create_dir_all(&mods).await?;
    let tmp = mods.join(format!("{name}.part"));
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, mods.join(&name)).await?;
    record_community(game_dir, &name, true)?;
    Ok(())
}

pub fn uninstall_community(game_dir: &Path, file_name: &str) -> LauncherResult<()> {
    let name = crate::commands::safe_mod_file_name(file_name)
        .ok_or_else(|| LauncherError::InvalidUrl(format!("Nom de fichier refusé : {file_name}")))?;
    // Seuls les mods installés depuis la liste communauté peuvent être retirés ici.
    if !read_community(game_dir).contains(&name) {
        return Err(LauncherError::InvalidLaunchConfig("Ce mod ne vient pas de la liste communauté.".into()));
    }
    let p = game_dir.join("mods").join(&name);
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    record_community(game_dir, &name, false)?;
    Ok(())
}

// ---------------------------------------------------------------- dossiers de contenu

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_ms: i64,
    pub is_dir: bool,
}

/// Dossiers que l'interface peut lister et nettoyer.
pub fn content_dir(game_dir: &Path, kind: &str) -> Option<PathBuf> {
    match kind {
        "screenshots" | "resourcepacks" | "shaderpacks" => Some(game_dir.join(kind)),
        _ => None,
    }
}

fn wanted(kind: &str, name: &str, is_dir: bool) -> bool {
    let l = name.to_ascii_lowercase();
    match kind {
        "screenshots" => !is_dir && (l.ends_with(".png") || l.ends_with(".jpg")),
        _ => is_dir || l.ends_with(".zip"),
    }
}

/// Contenu d'un dossier, le plus récent d'abord.
pub fn list_entries(dir: &Path, kind: &str, limit: usize) -> Vec<Entry> {
    let mut v: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().to_string();
            if !wanted(kind, &name, md.is_dir()) {
                return None;
            }
            let modified_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Some(Entry { path: e.path().to_string_lossy().to_string(), name, size: md.len(), modified_ms, is_dir: md.is_dir() })
        })
        .collect();
    v.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    v.truncate(limit);
    v
}

pub fn delete_entry(dir: &Path, name: &str) -> LauncherResult<()> {
    let n = safe_entry_name(name).ok_or_else(|| LauncherError::InvalidUrl(format!("Nom refusé : {name}")))?;
    let p = dir.join(n);
    if p.is_dir() {
        std::fs::remove_dir_all(p)?;
    } else if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

/// Pack de shaders actif (Oculus) : `config/oculus.properties`.
pub fn shader_state(game_dir: &Path) -> (bool, Option<String>) {
    let txt = std::fs::read_to_string(game_dir.join("config/oculus.properties")).unwrap_or_default();
    let get = |k: &str| txt.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')).map(|v| v.trim().to_string()));
    (get("enableShaders").as_deref() == Some("true"), get("shaderPack").filter(|s| !s.is_empty()))
}

/// Choisit le pack de shaders (ou aucun) dans `config/oculus.properties`, autres clés gardées.
pub fn set_shader(game_dir: &Path, pack: Option<&str>) -> LauncherResult<()> {
    let path = game_dir.join("config/oculus.properties");
    let txt = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = txt
        .lines()
        .filter(|l| !l.starts_with("enableShaders=") && !l.starts_with("shaderPack="))
        .map(String::from)
        .collect();
    match pack {
        Some(p) => {
            let n = safe_entry_name(p).ok_or_else(|| LauncherError::InvalidUrl(format!("Nom refusé : {p}")))?;
            lines.push("enableShaders=true".into());
            lines.push(format!("shaderPack={n}"));
        }
        None => lines.push("enableShaders=false".into()),
    }
    std::fs::create_dir_all(path.parent().unwrap_or(game_dir))?;
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

/// Oculus (shaders) fait-il partie des mods installés ?
pub fn has_shader_loader(game_dir: &Path) -> bool {
    std::fs::read_dir(game_dir.join("mods"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.ends_with(".jar") && (n.starts_with("oculus") || n.starts_with("iris"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve_once(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf).await;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
        });
        format!("http://127.0.0.1:{}/mods/Oculus.jar", addr.port())
    }

    #[tokio::test]
    async fn installs_and_rejects_downloads() {
        let tmp = tempfile::tempdir().unwrap();
        let jar = b"PK\x03\x04oculus".to_vec();
        let url = serve_once(jar.clone()).await;
        let sha = hex::encode(Sha256::digest(&jar));
        install_community(tmp.path(), &url, "Oculus.jar", &sha).await.unwrap();
        assert_eq!(std::fs::read(tmp.path().join("mods/Oculus.jar")).unwrap(), jar);
        assert_eq!(installed_community(tmp.path()), vec!["Oculus.jar".to_string()]);

        let url = serve_once(b"<html>erreur</html>".to_vec()).await;
        assert!(install_community(tmp.path(), &url, "Bad.jar", "").await.is_err());
        assert!(!tmp.path().join("mods/Bad.jar").exists());
        assert!(install_community(tmp.path(), "http://example.com/x.jar", "x.jar", "").await.is_err(), "http refusé");
        assert!(install_community(tmp.path(), "https://example.com/x.jar", "../x.jar", "").await.is_err());
    }

    #[test]
    fn entry_names_are_confined() {
        assert!(safe_entry_name("Pack 1.zip").is_some());
        assert!(safe_entry_name("../x").is_none());
        assert!(safe_entry_name("a/b").is_none());
        assert!(safe_entry_name("C:x").is_none());
    }

    #[test]
    fn jar_checks() {
        let jar = b"PK\x03\x04rest";
        assert!(check_jar(jar, "").is_ok());
        assert!(check_jar(b"<html>", "").is_err());
        let sha = hex::encode(Sha256::digest(jar));
        assert!(check_jar(jar, &sha).is_ok());
        assert!(check_jar(jar, &"0".repeat(64)).is_err());
    }

    #[test]
    fn community_record_and_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("mods")).unwrap();
        std::fs::write(tmp.path().join("mods/a.jar"), b"PK").unwrap();
        std::fs::write(tmp.path().join("mods/pack.jar"), b"PK").unwrap();
        record_community(tmp.path(), "a.jar", true).unwrap();
        assert_eq!(installed_community(tmp.path()), vec!["a.jar".to_string()]);
        assert!(uninstall_community(tmp.path(), "pack.jar").is_err(), "un mod du modpack n'est jamais retiré ici");
        uninstall_community(tmp.path(), "a.jar").unwrap();
        assert!(!tmp.path().join("mods/a.jar").exists());
        assert!(installed_community(tmp.path()).is_empty());
    }

    #[test]
    fn lists_newest_first_and_filters() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("old.png"), b"x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(tmp.path().join("new.png"), b"x").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"x").unwrap();
        let v = list_entries(tmp.path(), "screenshots", 10);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["new.png", "old.png"]);
    }

    #[test]
    fn shader_choice_keeps_other_keys() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::write(tmp.path().join("config/oculus.properties"), "maxShadowRenderDistance=32\nenableShaders=false\n").unwrap();
        set_shader(tmp.path(), Some("BSL.zip")).unwrap();
        assert_eq!(shader_state(tmp.path()), (true, Some("BSL.zip".into())));
        let txt = std::fs::read_to_string(tmp.path().join("config/oculus.properties")).unwrap();
        assert!(txt.contains("maxShadowRenderDistance=32"));
        set_shader(tmp.path(), None).unwrap();
        assert!(!shader_state(tmp.path()).0);
    }
}
