use crate::downloader::ManifestEntry;
use crate::error::{LauncherError, LauncherResult};
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue, CACHE_CONTROL};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

pub const DEFAULT_PANEL_URL: &str = "https://launcher.geoventure.fr/";

/// Timeouts stricts pour éliminer définitivement les écrans de chargement infinis.
#[derive(Debug, Clone, Copy)]
pub struct HttpTimeouts {
    pub connect: Duration,
    pub request: Duration,
    pub read: Duration,
}

impl Default for HttpTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            request: Duration::from_secs(12),
            read: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoaderConfig {
    #[serde(rename = "type")]
    pub loader_type: String,
    pub build: String,
    pub enable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerStatusTarget {
    #[serde(rename = "nameServer")]
    pub name_server: Option<String>,
    pub ip: Option<String>,
    pub port: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub ip: Option<String>,
    pub port: Option<serde_json::Value>,
    pub test_ip: Option<String>,
    pub test_port: Option<serde_json::Value>,
    pub theme_color: Option<String>,
    pub color: Option<String>,
    pub is_default: bool,
    pub azauth: Option<String>,
    /// Autres champs du panel (canal, grades autorisés…) transmis tels quels à l'interface.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Configuration renvoyée par `https://launcher.geoventure.fr/utils/api`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    pub maintenance: bool,
    pub maintenance_message: Option<String>,
    pub game_version: String,
    pub instance: Option<String>,
    pub channel: Option<String>,
    #[serde(rename = "dataDirectory")]
    pub data_directory: String,
    pub verify: bool,
    pub email_verified: bool,
    pub azauth: Option<String>,
    pub splash: Option<String>,
    pub splash_author: Option<String>,
    pub server_icon: Option<String>,
    pub theme_color: Option<String>,
    pub accent_color: Option<String>,
    pub ram_min: Option<u32>,
    pub ram_max: Option<u32>,
    pub loader: LoaderConfig,
    pub status: Option<ServerStatusTarget>,
    pub ignored: Vec<String>,
    pub servers: Vec<ServerSummary>,
    pub rpc_activation: bool,
    pub rpc_id: Option<String>,
    /// Autres champs du panel (alertes, liste blanche, grades des canaux…) transmis tels quels.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPanelConfig {
    saved_at: i64,
    config: PanelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFetchResult {
    pub config: PanelConfig,
    pub offline: bool,
    pub cached_at: Option<i64>,
    pub warning: Option<String>,
}

/// Vérifie et normalise l'URL de base du panel : HTTPS forcé hors boucle locale.
pub fn secure_panel_url(raw: &str) -> LauncherResult<Url> {
    let mut url = Url::parse(raw.trim()).map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;
    let is_local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => {}
        "http" if is_local => {}
        "http" => {
            url.set_scheme("https")
                .map_err(|_| LauncherError::InvalidUrl("Impossible de forcer HTTPS".into()))?;
        }
        other => {
            return Err(LauncherError::InvalidUrl(format!(
                "Schéma refusé : {other}"
            )));
        }
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let mut path = url.path().to_string();
        path.push('/');
        url.set_path(&path);
    }
    Ok(url)
}

/// Construit une URL du panel avec la convention `?instance=<slug>[&channel=test]`.
pub fn build_panel_url(
    base: &Url,
    endpoint: &str,
    instance: Option<&str>,
    channel: &str,
) -> LauncherResult<Url> {
    let clean_endpoint = endpoint.trim_start_matches('/');
    let mut url = base
        .join(clean_endpoint)
        .map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;

    let mut pairs = url.query_pairs_mut();
    if let Some(inst) = instance.map(str::trim).filter(|s| !s.is_empty()) {
        if !inst
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(LauncherError::InvalidUrl(format!(
                "Identifiant d'instance invalide : {inst}"
            )));
        }
        pairs.append_pair("instance", inst);
        // L'historique de fréquentation attend `?server=<id>`.
        if clean_endpoint == "utils/servers-history" {
            pairs.append_pair("server", inst);
        }
    }
    if channel.eq_ignore_ascii_case("test") {
        pairs.append_pair("channel", "test");
    }
    drop(pairs);
    Ok(url)
}

pub fn cache_file_path(cache_dir: &Path, instance: Option<&str>, channel: &str) -> PathBuf {
    let slug: String = instance
        .unwrap_or("default")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let suffix = if channel.eq_ignore_ascii_case("test") {
        "_test"
    } else {
        ""
    };
    cache_dir.join(format!("config_cache_{slug}{suffix}.json"))
}

/// Client HTTP résilient pour le panel Geoventure (`launcher.geoventure.fr`).
#[derive(Debug, Clone)]
pub struct PanelClient {
    base_url: Url,
    http: reqwest::Client,
}

impl PanelClient {
    pub fn new(base_url: &str, timeouts: HttpTimeouts) -> LauncherResult<Self> {
        let base_url = secure_panel_url(base_url)?;
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));

        let http = reqwest::Client::builder()
            .user_agent("GeoventureLauncher/2.0 (Rust/Tauri)")
            .default_headers(headers)
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.request)
            .read_timeout(timeouts.read)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| LauncherError::Internal(format!("Initialisation HTTP impossible : {e}")))?;

        Ok(Self { base_url, http })
    }

    pub fn default_panel() -> LauncherResult<Self> {
        Self::new(DEFAULT_PANEL_URL, HttpTimeouts::default())
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    /// Client des gros téléchargements (jeu, Java, Forge, mods) : délais de connexion et
    /// d'inactivité stricts, mais PAS de délai total (un jar de 30 Mo sur une connexion lente
    /// dépasserait les 12 s du client du panel). Une connexion qui ne reçoit plus rien
    /// pendant 30 s est coupée et le fichier est retenté.
    pub fn download_client() -> LauncherResult<reqwest::Client> {
        reqwest::Client::builder()
            .user_agent("GeoventureLauncher/2.0 (Rust/Tauri)")
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| LauncherError::Internal(format!("Initialisation HTTP impossible : {e}")))
    }

    /// Lit une route JSON du panel (`utils/notifications`, `utils/leaderboards`…) telle quelle.
    pub async fn fetch_panel_json(
        &self,
        endpoint: &str,
        instance: Option<&str>,
        channel: &str,
    ) -> LauncherResult<serde_json::Value> {
        let url = build_panel_url(&self.base_url, endpoint, instance, channel)?;
        self.fetch_json::<serde_json::Value>(&url).await
    }

    /// Lit une URL JSON quelconque (Mojang, Forge) avec le client du panel.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> LauncherResult<T> {
        let u = Url::parse(url).map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;
        self.fetch_json(&u).await
    }

    /// Récupère `utils/api` avec repli automatique sur le cache disque hors-ligne.
    pub async fn fetch_config_with_cache(
        &self,
        instance: Option<&str>,
        channel: &str,
        cache_dir: &Path,
    ) -> LauncherResult<ConfigFetchResult> {
        let url = build_panel_url(&self.base_url, "utils/api", instance, channel)?;
        let cache_path = cache_file_path(cache_dir, instance, channel);

        match self.fetch_json::<PanelConfig>(&url).await {
            Ok(config) => {
                let saved_at = Utc::now().timestamp();
                let payload = CachedPanelConfig {
                    saved_at,
                    config: config.clone(),
                };
                if let Some(parent) = cache_path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                if let Ok(serialized) = serde_json::to_vec_pretty(&payload) {
                    let _ = tokio::fs::write(&cache_path, serialized).await;
                }
                Ok(ConfigFetchResult {
                    config,
                    offline: false,
                    cached_at: Some(saved_at),
                    warning: None,
                })
            }
            Err(err) => {
                if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                    if let Ok(cached) = serde_json::from_slice::<CachedPanelConfig>(&bytes) {
                        return Ok(ConfigFetchResult {
                            config: cached.config,
                            offline: true,
                            cached_at: Some(cached.saved_at),
                            warning: Some(format!(
                                "Panel injoignable ({err}) : mode hors-ligne activé depuis le cache local."
                            )),
                        });
                    }
                }
                Err(err)
            }
        }
    }

    /// Récupère le manifeste des fichiers du modpack (`data/`) en contournant les caches HTTP.
    pub async fn fetch_modpack_manifest(
        &self,
        instance: Option<&str>,
        channel: &str,
    ) -> LauncherResult<Vec<ManifestEntry>> {
        let mut url = build_panel_url(&self.base_url, "data/", instance, channel)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("_", &Utc::now().timestamp_millis().to_string());
        }
        self.fetch_json::<Vec<ManifestEntry>>(&url).await
    }

    async fn fetch_json<T: serde::de::DeserializeOwned>(&self, url: &Url) -> LauncherResult<T> {
        let resp = self
            .http
            .get(url.clone())
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LauncherError::Timeout(url.to_string())
                } else {
                    LauncherError::Network {
                        url: url.to_string(),
                        message: e.to_string(),
                    }
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(LauncherError::HttpStatus {
                status: status.as_u16(),
                url: url.to_string(),
            });
        }

        resp.json::<T>().await.map_err(|e| LauncherError::Network {
            url: url.to_string(),
            message: format!("Réponse JSON invalide : {e}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn secure_panel_url_upgrades_remote_http_and_allows_localhost() {
        let remote = secure_panel_url("http://launcher.geoventure.fr").unwrap();
        assert_eq!(remote.as_str(), "https://launcher.geoventure.fr/");

        let local = secure_panel_url("http://127.0.0.1:8080/panel?foo=1#bar").unwrap();
        assert_eq!(local.as_str(), "http://127.0.0.1:8080/panel/");

        assert!(secure_panel_url("ftp://launcher.geoventure.fr").is_err());
    }

    #[test]
    fn build_panel_url_adds_instance_and_test_channel() {
        let base = secure_panel_url("https://launcher.geoventure.fr").unwrap();
        let u1 = build_panel_url(&base, "utils/api", Some("geoventure"), "stable").unwrap();
        assert_eq!(
            u1.as_str(),
            "https://launcher.geoventure.fr/utils/api?instance=geoventure"
        );

        let u2 = build_panel_url(&base, "utils/api", Some("elandor"), "test").unwrap();
        assert_eq!(
            u2.as_str(),
            "https://launcher.geoventure.fr/utils/api?instance=elandor&channel=test"
        );

        assert!(build_panel_url(&base, "utils/api", Some("../bad"), "stable").is_err());
    }

    #[tokio::test]
    async fn fetch_config_saves_cache_and_falls_back_offline_on_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let body = r#"{"maintenance":false,"game_version":"1.20.1","dataDirectory":"geoventure","verify":true,"loader":{"type":"forge","build":"47.3.0","enable":true},"test_roles":["Staff"]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let tmp = tempfile::tempdir().unwrap();
        let client = PanelClient::new(
            &format!("http://{addr}/"),
            HttpTimeouts {
                connect: Duration::from_secs(2),
                request: Duration::from_secs(2),
                read: Duration::from_secs(2),
            },
        )
        .unwrap();

        // 1. Premier appel en ligne -> remplit le cache disque
        let online = client
            .fetch_config_with_cache(Some("geoventure"), "stable", tmp.path())
            .await
            .unwrap();
        server.await.unwrap();
        assert!(!online.offline);
        assert_eq!(online.config.game_version, "1.20.1");
        assert_eq!(online.config.loader.build, "47.3.0");
        assert_eq!(online.config.extra.get("test_roles"), Some(&serde_json::json!(["Staff"])));

        // 2. Serveur arrêté -> bascule automatique sur le cache hors-ligne sans bloquer
        let offline = client
            .fetch_config_with_cache(Some("geoventure"), "stable", tmp.path())
            .await
            .unwrap();
        assert!(offline.offline);
        assert_eq!(offline.config.game_version, "1.20.1");
        assert!(offline.warning.is_some());
    }
}
