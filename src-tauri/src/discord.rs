//! Discord Rich Presence (« Joue à Geoventure » sur le profil Discord du joueur).
//! Réglages venus du panel (`rpc_*` de `utils/api`). Discord fermé : on ignore sans bruit.

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use serde::Deserialize;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Presence {
    pub client_id: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub large_image: String,
    #[serde(default)]
    pub large_text: String,
    #[serde(default)]
    pub small_image: String,
    #[serde(default)]
    pub small_text: String,
    #[serde(default)]
    pub buttons: Vec<PresenceButton>,
    /// Début de l'activité (millisecondes Unix), pour le « depuis 12 min ».
    pub start_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PresenceButton {
    pub label: String,
    pub url: String,
}

static WORKER: OnceLock<Mutex<Sender<Option<Presence>>>> = OnceLock::new();

/// Identifiant d'application Discord : uniquement des chiffres.
pub fn valid_client_id(id: &str) -> bool {
    (15..=22).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Boutons gardés : 2 au plus, libellé non vide (32 caractères max), lien https.
pub fn clean_buttons(buttons: &[PresenceButton]) -> Vec<PresenceButton> {
    buttons
        .iter()
        .filter(|b| !b.label.trim().is_empty() && b.url.starts_with("https://"))
        .take(2)
        .map(|b| PresenceButton { label: b.label.trim().chars().take(32).collect(), url: b.url.clone() })
        .collect()
}

fn clip(s: &str) -> String {
    s.chars().take(128).collect()
}

fn apply(client: &mut DiscordIpcClient, p: &Presence) -> bool {
    let buttons = clean_buttons(&p.buttons);
    let (details, state, lt, st) = (clip(&p.details), clip(&p.state), clip(&p.large_text), clip(&p.small_text));
    let mut a = activity::Activity::new();
    if !details.is_empty() {
        a = a.details(details.as_str());
    }
    if !state.is_empty() {
        a = a.state(state.as_str());
    }
    let mut assets = activity::Assets::new();
    if !p.large_image.is_empty() {
        assets = assets.large_image(p.large_image.as_str()).large_text(lt.as_str());
    }
    if !p.small_image.is_empty() {
        assets = assets.small_image(p.small_image.as_str()).small_text(st.as_str());
    }
    a = a.assets(assets);
    if let Some(start) = p.start_ms {
        a = a.timestamps(activity::Timestamps::new().start(start));
    }
    if !buttons.is_empty() {
        a = a.buttons(buttons.iter().map(|b| activity::Button::new(b.label.as_str(), b.url.as_str())).collect());
    }
    client.set_activity(a).is_ok()
}

fn worker() -> Sender<Option<Presence>> {
    let (tx, rx) = channel::<Option<Presence>>();
    std::thread::Builder::new()
        .name("discord-rpc".into())
        .spawn(move || {
            let mut client: Option<DiscordIpcClient> = None;
            while let Ok(msg) = rx.recv() {
                match msg {
                    None => {
                        if let Some(mut c) = client.take() {
                            let _ = c.clear_activity();
                            let _ = c.close();
                        }
                    }
                    Some(p) => {
                        if client.as_ref().map(|c| c.client_id != p.client_id).unwrap_or(true) {
                            if let Some(mut c) = client.take() {
                                let _ = c.close();
                            }
                            let mut c = DiscordIpcClient::new(&p.client_id);
                            if c.connect().is_ok() {
                                client = Some(c);
                            }
                        }
                        if let Some(c) = client.as_mut() {
                            // Discord redémarré entre-temps : une reconnexion, puis on abandonne.
                            if !apply(c, &p) && !(c.reconnect().is_ok() && apply(c, &p)) {
                                client = None;
                            }
                        }
                    }
                }
            }
        })
        .ok();
    tx
}

/// Met à jour (ou efface avec `None`) la présence Discord, sans jamais bloquer l'appelant.
pub fn set(p: Option<Presence>) {
    if let Some(p) = &p {
        if !valid_client_id(&p.client_id) {
            return;
        }
    }
    let tx = WORKER.get_or_init(|| Mutex::new(worker()));
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_client_id() {
        assert!(valid_client_id("1517422035033395230"));
        assert!(!valid_client_id("abc"));
        assert!(!valid_client_id("15174220350333952a0"));
    }

    #[test]
    fn keeps_two_https_buttons() {
        let b = |l: &str, u: &str| PresenceButton { label: l.into(), url: u.into() };
        let out = clean_buttons(&[b("Discord", "https://discord.gg/x"), b("", "https://a"), b("Site", "http://x"), b("Site", "https://geoventure.fr"), b("Trop", "https://z")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].label, "Site");
    }
}
