//! Actualité du site (flux RSS Azuriom), contenu de l'accueil (raccourcis, « À découvrir ») et
//! rapport de diagnostic (masqué avant affichage ou envoi).

use crate::error::{LauncherError, LauncherResult};
use regex::Regex;
use serde::Serialize;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const MASK: &str = "[masqué]";

// ---------------------------------------------------------------------------------------------
// Actualité
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NewsItem {
    pub title: String,
    pub link: Option<String>,
    pub summary: String,
    pub author: Option<String>,
    /// Date RFC 2822 telle que donnée par le site.
    pub date: Option<String>,
    /// Timestamp (ms) si la date a pu être lue.
    pub timestamp: Option<i64>,
    pub image: Option<String>,
}

fn https_only(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    (u.scheme() == "https").then(|| u.to_string())
}

/// Texte brut d'un fragment HTML (balises retirées, entités courantes décodées, espaces réduits).
pub fn html_to_text(html: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static BLOCKS: OnceLock<Regex> = OnceLock::new();
    let blocks = BLOCKS.get_or_init(|| Regex::new(r"(?i)<\s*(br|/p|/div|/li|/h[1-6])[^>]*>").unwrap());
    let tags = TAGS.get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let with_breaks = blocks.replace_all(html, "\n");
    let text = tags.replace_all(&with_breaks, "");
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
        .replace("&rsquo;", "’")
        .replace("&laquo;", "«")
        .replace("&raquo;", "»");
    decoded
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    if let Some(i) = out.rfind(' ') {
        out.truncate(i);
    }
    out.push('…');
    out
}

/// Lit le flux RSS d'Azuriom (`/api/rss`). Les articles sont renvoyés en texte brut : rien du
/// site n'est injecté tel quel dans l'interface.
pub fn parse_rss(xml: &str) -> LauncherResult<Vec<NewsItem>> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| LauncherError::Internal(format!("Flux d'actualité illisible : {e}")))?;
    let mut out = Vec::new();
    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let child = |name: &str| -> Option<String> {
            item.children()
                .find(|c| c.tag_name().name() == name)
                .and_then(|c| c.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };
        let title = html_to_text(&child("title").unwrap_or_default());
        if title.is_empty() {
            continue;
        }
        let summary_src = child("description").or_else(|| child("encoded")).unwrap_or_default();
        let summary = truncate_chars(&html_to_text(&summary_src), 320);
        let image = item
            .children()
            .find(|c| c.tag_name().name() == "enclosure")
            .and_then(|c| c.attribute("url"))
            .and_then(https_only);
        let date = child("pubDate");
        let timestamp = date
            .as_deref()
            .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok())
            .map(|d| d.timestamp_millis());
        out.push(NewsItem {
            title,
            link: child("link").and_then(|l| https_only(&l)),
            summary,
            author: child("creator"),
            date,
            timestamp,
            image,
        });
    }
    Ok(out)
}

pub async fn fetch_news(http: &reqwest::Client, site_url: &str) -> LauncherResult<Vec<NewsItem>> {
    let base = crate::config::secure_panel_url(site_url)?;
    let url = base
        .join("api/rss")
        .map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;
    let resp = http.get(url.clone()).send().await.map_err(|e| LauncherError::Network {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    if !resp.status().is_success() {
        return Err(LauncherError::HttpStatus {
            status: resp.status().as_u16(),
            url: url.to_string(),
        });
    }
    let text = resp.text().await.map_err(|e| LauncherError::Network {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    let mut items = parse_rss(&text)?;
    items.truncate(10);
    Ok(items)
}

// ---------------------------------------------------------------------------------------------
// Diagnostic
// ---------------------------------------------------------------------------------------------

/// Masque ce qui ne doit pas quitter le PC du joueur : jetons connus, dossier personnel,
/// arguments sensibles, en-têtes d'authentification, JWT, e-mails, longues empreintes hexadécimales.
/// Mêmes règles que `utils/diagnostic.js` du launcher Electron.
pub fn redact(text: &str, homes: &[String], secrets: &[String]) -> String {
    let mut out = text.to_string();
    let mut secrets: Vec<&String> = secrets.iter().filter(|s| s.len() >= 6).collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for s in secrets {
        out = out.replace(s.as_str(), MASK);
    }

    let mut variants: Vec<String> = Vec::new();
    for h in homes {
        let clean = h.trim_end_matches(['/', '\\']);
        if clean.len() < 3 {
            continue;
        }
        variants.push(clean.to_string());
        variants.push(clean.replace('\\', "/"));
        variants.push(clean.replace('/', "\\"));
    }
    variants.sort_by_key(|v| std::cmp::Reverse(v.len()));
    variants.dedup();
    for v in variants {
        if let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(&v))) {
            out = re.replace_all(&out, "~").to_string();
        }
    }

    static RULES: OnceLock<Vec<(Regex, String)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        vec![
            (Regex::new(r#"(?i)([A-Za-z]:[\\/]+Users[\\/]+)[^\\/\s"']+"#).unwrap(), "${1}<user>".to_string()),
            (Regex::new(r#"(/home/)[^/\s"']+"#).unwrap(), "${1}<user>".to_string()),
            (Regex::new(r#"(/Users/)[^/\s"']+"#).unwrap(), "${1}<user>".to_string()),
            (
                Regex::new(r"(?i)(--(?:accessToken|session|password|uuid|xuid|clientId|userProperties)[\s,]+)[^\s,\]]+").unwrap(),
                format!("${{1}}{MASK}"),
            ),
            (
                Regex::new(r#"(?i)(["']?(?:access[_-]?token|client[_-]?token|refresh[_-]?token|id[_-]?token|token|password|passwd|pwd|mot[_ ]de[_ ]passe|secret|api[_-]?key|authorization|cookie)["']?\s*[:=]\s*)(?:Bearer\s+|Basic\s+)?(?:"[^"]*"|'[^']*'|[^\s"',;&}]+)"#).unwrap(),
                format!("${{1}}{MASK}"),
            ),
            (Regex::new(r"\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{6,}").unwrap(), format!("${{1}} {MASK}")),
            (Regex::new(r"\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*").unwrap(), MASK.to_string()),
            (Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap(), "[e-mail masqué]".to_string()),
            (Regex::new(r"\b[A-Fa-f0-9]{32,}\b").unwrap(), MASK.to_string()),
        ]
    });
    for (re, rep) in rules {
        out = re.replace_all(&out, rep.as_str()).to_string();
    }
    out
}

pub fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Dernières lignes d'un fichier (lit au plus les 512 derniers Ko).
pub fn tail_file(path: &Path, n: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(512 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(tail_lines(&String::from_utf8_lossy(&buf), n))
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub name: String,
    pub url: String,
    pub ok: bool,
    pub status: Option<u16>,
    pub millis: u128,
    pub error: Option<String>,
}

/// Sonde une adresse (6 s maximum) : statut HTTP et temps de réponse.
pub async fn probe(http: &reqwest::Client, name: &str, url: &str) -> ProbeResult {
    let t = Instant::now();
    let res = tokio::time::timeout(Duration::from_secs(6), http.get(url).send()).await;
    let millis = t.elapsed().as_millis();
    match res {
        Ok(Ok(resp)) => ProbeResult {
            name: name.into(),
            url: url.into(),
            ok: resp.status().as_u16() < 500,
            status: Some(resp.status().as_u16()),
            millis,
            error: None,
        },
        Ok(Err(e)) => ProbeResult {
            name: name.into(),
            url: url.into(),
            ok: false,
            status: None,
            millis,
            error: Some(e.to_string()),
        },
        Err(_) => ProbeResult {
            name: name.into(),
            url: url.into(),
            ok: false,
            status: None,
            millis,
            error: Some("délai de 6 s dépassé".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_items_are_plain_text_with_https_images() {
        let xml = r#"<?xml version="1.0"?><rss xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel>
          <item><title>Mise à jour &amp; colonies</title><link>https://geoventure.fr/news/a</link>
            <description>&lt;b&gt;GeoNTM&lt;/b&gt; arrive.</description>
            <pubDate>Sat, 21 Nov 2026 19:27:19 +0000</pubDate><dc:creator>Peter</dc:creator>
            <enclosure url="https://geoventure.fr/img.jpg" length="1" type="image/*"/></item>
          <item><title>Deux</title><link>javascript:alert(1)</link><description>x</description>
            <enclosure url="http://insecure/img.jpg"/></item>
        </channel></rss>"#;
        let items = parse_rss(xml).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Mise à jour & colonies");
        assert_eq!(items[0].summary, "GeoNTM arrive.");
        assert_eq!(items[0].author.as_deref(), Some("Peter"));
        assert_eq!(items[0].image.as_deref(), Some("https://geoventure.fr/img.jpg"));
        assert!(items[0].timestamp.is_some());
        assert!(items[1].link.is_none());
        assert!(items[1].image.is_none());
    }

    #[test]
    fn redact_masks_tokens_paths_and_emails() {
        let text = "user /home/alice/.Nexus --accessToken abcdef123456 x\n\
                    C:\\Users\\Bob\\AppData token=s3cr3tvalue Authorization: Bearer abc.def.ghi\n\
                    mail joueur@example.com hash 0123456789abcdef0123456789abcdef0 secretjeton9";
        let out = redact(text, &["/home/alice".into()], &["secretjeton9".into()]);
        assert!(out.contains("~/.Nexus"));
        assert!(out.contains(&format!("--accessToken {MASK}")));
        assert!(out.contains("C:\\Users\\<user>"));
        assert!(!out.contains("s3cr3tvalue"));
        assert!(!out.contains("joueur@example.com"));
        assert!(!out.contains("0123456789abcdef0123456789abcdef0"));
        assert!(!out.contains("secretjeton9"));
        assert!(!out.contains("abc.def.ghi"));
    }

    #[test]
    fn tail_keeps_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
    }
}
