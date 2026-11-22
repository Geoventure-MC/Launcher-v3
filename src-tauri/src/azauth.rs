use crate::config::{secure_panel_url, HttpTimeouts};
use crate::error::{LauncherError, LauncherResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AzRole {
    pub name: String,
    pub color: Option<String>,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AzAccount {
    pub id: u64,
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub email_verified: bool,
    pub money: f64,
    pub banned: bool,
    pub role: Option<AzRole>,
    /// URL du site Azuriom utilisé pour cette session (ex: https://geoventure.fr/).
    #[serde(default)]
    pub site_url: String,
    /// URL de rendu skin/avatar 3D Azuriom.
    #[serde(default)]
    pub skin_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AzAuthResponse {
    Authenticated { account: AzAccount },
    Requires2fa,
    Rejected { reason: String, message: String },
}

#[derive(Debug, Serialize)]
struct AuthenticatePayload<'a> {
    email: &'a str,
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct TokenPayload<'a> {
    access_token: &'a str,
}

#[derive(Debug, Deserialize)]
struct RawAzAuthPayload {
    status: Option<String>,
    reason: Option<String>,
    message: Option<String>,
    id: Option<u64>,
    username: Option<String>,
    uuid: Option<String>,
    access_token: Option<String>,
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    money: f64,
    #[serde(default)]
    banned: bool,
    role: Option<AzRole>,
}

pub fn account_session_path(storage_dir: &Path) -> PathBuf {
    storage_dir.join("azuriom_session.json")
}

/// Construit l'URL de prévisualisation skin Azuriom (compatible plugin skin3d de Launcher-v3).
pub fn build_skin_url(base_url: &Url, username: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(username.as_bytes()).collect();
    format!("{base_url}api/skin-api/avatars/face/{encoded}")
}

/// Persiste la session Azuriom dans le répertoire privé du launcher. Le jeton part dans le
/// coffre-fort du système quand il y en a un ; le fichier n'en garde alors qu'une place vide.
pub async fn save_account_session(storage_dir: &Path, account: &AzAccount) -> LauncherResult<()> {
    tokio::fs::create_dir_all(storage_dir).await?;
    let path = account_session_path(storage_dir);
    let mut on_disk = account.clone();
    if crate::secure::store_token(&account.access_token) {
        on_disk.access_token = String::new();
    }
    let data = serde_json::to_vec_pretty(&on_disk)?;
    tokio::fs::write(&path, data).await?;
    crate::secure::restrict_permissions(&path);
    Ok(())
}

pub async fn load_account_session(storage_dir: &Path) -> LauncherResult<Option<AzAccount>> {
    let path = account_session_path(storage_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut acc: AzAccount = serde_json::from_slice(&bytes)?;
            if acc.access_token.is_empty() {
                match crate::secure::load_token() {
                    Some(t) => acc.access_token = t,
                    // Coffre-fort vidé ou inaccessible : la session n'est plus utilisable.
                    None => return Ok(None),
                }
            } else if crate::secure::store_token(&acc.access_token) {
                // Ancienne session en clair : on la déplace dans le coffre-fort.
                let mut on_disk = acc.clone();
                on_disk.access_token = String::new();
                if let Ok(data) = serde_json::to_vec_pretty(&on_disk) {
                    let _ = tokio::fs::write(&path, data).await;
                }
            }
            crate::secure::restrict_permissions(&path);
            Ok(Some(acc))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn clear_account_session(storage_dir: &Path) -> LauncherResult<()> {
    crate::secure::delete_token();
    let path = account_session_path(storage_dir);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Client natif pour l'API d'authentification Azuriom (`/api/auth/*`).
/// Mot de passe et jeton transitent uniquement en HTTPS (hors boucle locale pour les tests).
#[derive(Debug, Clone)]
pub struct AzAuthClient {
    base_url: Url,
    http: reqwest::Client,
}

impl AzAuthClient {
    pub fn new(site_url: &str, timeouts: HttpTimeouts) -> LauncherResult<Self> {
        let base_url = secure_panel_url(site_url)?;
        let http = reqwest::Client::builder()
            .user_agent("GeoventureLauncher/2.0 (AzAuth-Rust)")
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.request)
            .read_timeout(timeouts.read)
            .build()
            .map_err(|e| LauncherError::Internal(format!("Client AzAuth impossible : {e}")))?;
        Ok(Self { base_url, http })
    }

    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
        code_2fa: Option<&str>,
    ) -> LauncherResult<AzAuthResponse> {
        let endpoint = self
            .base_url
            .join("api/auth/authenticate")
            .map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;

        let payload = AuthenticatePayload {
            email: email.trim(),
            password,
            code: code_2fa.map(str::trim).filter(|s| !s.is_empty()),
        };

        let resp = self
            .http
            .post(endpoint.clone())
            .json(&payload)
            .send()
            .await
            .map_err(|e| map_reqwest_err(&endpoint, e))?;

        let status = resp.status();
        let raw: RawAzAuthPayload = resp.json().await.map_err(|e| LauncherError::Network {
            url: endpoint.to_string(),
            message: format!("Réponse Azuriom illisible (HTTP {}) : {e}", status.as_u16()),
        })?;

        if raw.status.as_deref() == Some("pending") && raw.reason.as_deref() == Some("2fa") {
            return Ok(AzAuthResponse::Requires2fa);
        }

        if raw.status.as_deref() == Some("error") || !status.is_success() {
            return Ok(AzAuthResponse::Rejected {
                reason: raw.reason.unwrap_or_else(|| format!("http_{}", status.as_u16())),
                message: raw
                    .message
                    .unwrap_or_else(|| "Authentification Azuriom refusée".to_string()),
            });
        }

        let account = parse_account(raw, &self.base_url)?;
        Ok(AzAuthResponse::Authenticated { account })
    }

    pub async fn verify(&self, access_token: &str) -> LauncherResult<AzAccount> {
        let endpoint = self
            .base_url
            .join("api/auth/verify")
            .map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;

        let resp = self
            .http
            .post(endpoint.clone())
            .json(&TokenPayload { access_token })
            .send()
            .await
            .map_err(|e| map_reqwest_err(&endpoint, e))?;

        if !resp.status().is_success() {
            return Err(LauncherError::HttpStatus {
                status: resp.status().as_u16(),
                url: endpoint.to_string(),
            });
        }

        let raw: RawAzAuthPayload = resp.json().await.map_err(|e| LauncherError::Network {
            url: endpoint.to_string(),
            message: format!("Réponse verify Azuriom illisible : {e}"),
        })?;
        parse_account(raw, &self.base_url)
    }

    pub async fn logout(&self, access_token: &str) -> LauncherResult<()> {
        let endpoint = self
            .base_url
            .join("api/auth/logout")
            .map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;

        let resp = self
            .http
            .post(endpoint.clone())
            .json(&TokenPayload { access_token })
            .send()
            .await
            .map_err(|e| map_reqwest_err(&endpoint, e))?;

        if !resp.status().is_success() {
            return Err(LauncherError::HttpStatus {
                status: resp.status().as_u16(),
                url: endpoint.to_string(),
            });
        }
        Ok(())
    }
}

fn parse_account(raw: RawAzAuthPayload, base_url: &Url) -> LauncherResult<AzAccount> {
    let username = raw
        .username
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::Internal("Champ username absent de la réponse Azuriom".into()))?;
    let uuid = raw
        .uuid
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::Internal("Champ uuid absent de la réponse Azuriom".into()))?;
    let access_token = raw
        .access_token
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::Internal("Champ access_token absent de la réponse Azuriom".into()))?;

    let skin_url = Some(build_skin_url(base_url, &username));
    Ok(AzAccount {
        id: raw.id.unwrap_or(0),
        username,
        uuid,
        access_token,
        email_verified: raw.email_verified,
        money: raw.money,
        banned: raw.banned,
        role: raw.role,
        site_url: base_url.to_string(),
        skin_url,
    })
}

fn map_reqwest_err(url: &Url, err: reqwest::Error) -> LauncherError {
    if err.is_timeout() {
        LauncherError::Timeout(url.to_string())
    } else {
        LauncherError::Network {
            url: url.to_string(),
            message: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn azauth_handles_2fa_success_and_session_persistence() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut s1, _) = listener.accept().await.unwrap();
            let mut b1 = [0u8; 1024];
            let _ = s1.read(&mut b1).await;
            let body1 = r#"{"status":"pending","reason":"2fa","message":"2FA code required"}"#;
            let r1 = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body1.len(),
                body1
            );
            s1.write_all(r1.as_bytes()).await.unwrap();

            let (mut s2, _) = listener.accept().await.unwrap();
            let mut b2 = [0u8; 1024];
            let _ = s2.read(&mut b2).await;
            let body2 = r##"{"id":42,"username":"GeoventurePlayer","uuid":"11111111-2222-3333-4444-555555555555","email_verified":true,"money":250.5,"banned":false,"access_token":"az-tok-123","role":{"name":"Admin","color":"#4ade80","is_admin":true}}"##;
            let r2 = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body2.len(),
                body2
            );
            s2.write_all(r2.as_bytes()).await.unwrap();
        });

        let client = AzAuthClient::new(
            &format!("http://{addr}/"),
            HttpTimeouts {
                connect: Duration::from_secs(2),
                request: Duration::from_secs(2),
                read: Duration::from_secs(2),
            },
        )
        .unwrap();

        let step1 = client
            .authenticate("player@example.com", "secret", None)
            .await
            .unwrap();
        assert!(matches!(step1, AzAuthResponse::Requires2fa));

        let step2 = client
            .authenticate("player@example.com", "secret", Some("123456"))
            .await
            .unwrap();
        server.await.unwrap();

        let tmp = tempfile::tempdir().unwrap();
        match step2 {
            AzAuthResponse::Authenticated { account } => {
                assert_eq!(account.username, "GeoventurePlayer");
                assert_eq!(account.role.as_ref().unwrap().name, "Admin");
                assert!(account.skin_url.as_deref().unwrap().contains("GeoventurePlayer"));

                save_account_session(tmp.path(), &account).await.unwrap();
                let loaded = load_account_session(tmp.path()).await.unwrap().unwrap();
                assert_eq!(loaded.username, "GeoventurePlayer");

                clear_account_session(tmp.path()).await.unwrap();
                assert!(load_account_session(tmp.path()).await.unwrap().is_none());
            }
            other => panic!("Attendu Authenticated, reçu {other:?}"),
        }
    }
}
