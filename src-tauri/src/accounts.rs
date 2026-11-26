//! Plusieurs comptes Azuriom dans le launcher.
//!
//! `accounts.json` (dossier de session) : la liste des comptes et le compte actif. Le jeton de
//! chaque compte est rangé dans le coffre-fort du système sous une entrée propre
//! (`azuriom_session:<clé>`) ; sans coffre (Linux), il reste dans le fichier, lisible par son seul
//! propriétaire. Clé d'un compte = site + id Azuriom (deux sites = deux comptes).
//!
//! Migration : l'ancien `azuriom_session.json` (un seul compte, jeton dans l'entrée
//! « azuriom_session » du coffre-fort) devient le premier compte actif. L'ancien format n'est
//! supprimé qu'une fois la liste écrite puis relue avec succès.

use crate::azauth::{account_session_path, AzAccount, AzRole};
use crate::error::{LauncherError, LauncherResult};
use crate::secure::{TokenVault, LEGACY_ENTRY};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// Plafond de comptes gardés dans le launcher.
pub const MAX_ACCOUNTS: usize = 10;
const INDEX_FILE: &str = "accounts.json";
/// Site supposé pour une ancienne session qui n'en mentionne pas.
const DEFAULT_SITE: &str = "https://geoventure.fr/";

/// Une seule modification de la liste à la fois (connexion, bascule et retrait peuvent se croiser).
static LOCK: Mutex<()> = Mutex::new(());
static SYSTEM_VAULT: crate::secure::SystemVault = crate::secure::SystemVault;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    key: String,
    /// Le site a refusé le jeton : le joueur doit retaper son mot de passe.
    #[serde(default)]
    needs_login: bool,
    /// `access_token` vide quand le jeton est dans le coffre-fort.
    account: AzAccount,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    version: u32,
    active: Option<String>,
    accounts: Vec<StoredAccount>,
}

/// Compte vu par l'interface : jamais de jeton.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AccountSummary {
    pub key: String,
    pub id: u64,
    pub username: String,
    pub skin_url: Option<String>,
    pub site_url: String,
    pub role: Option<AzRole>,
    pub active: bool,
    pub needs_login: bool,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct AccountList {
    pub accounts: Vec<AccountSummary>,
    pub active: Option<String>,
}

fn normalize_site(url: &str) -> String {
    let url = if url.trim().is_empty() { DEFAULT_SITE } else { url.trim() };
    let lower = url.to_lowercase();
    let bare = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    bare.trim_end_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || ".-_:/".contains(c) { c } else { '_' })
        .collect()
}

/// Clé stable d'un compte : site + id Azuriom. Une ancienne session sans id (0) se repère par son pseudo.
pub fn account_key(account: &AzAccount) -> String {
    let site = normalize_site(&account.site_url);
    if account.id > 0 {
        format!("{site}#{}", account.id)
    } else {
        let name: String = account
            .username
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
            .collect();
        format!("{site}#u:{name}")
    }
}

fn entry_name(key: &str) -> String {
    format!("{LEGACY_ENTRY}:{key}")
}

pub struct AccountStore<'a> {
    dir: &'a Path,
    vault: &'a dyn TokenVault,
}

impl<'a> AccountStore<'a> {
    pub fn new(dir: &'a Path, vault: &'a dyn TokenVault) -> Self {
        Self { dir, vault }
    }

    /// Liste dans le dossier de session, jetons dans le coffre-fort du système.
    pub fn system(dir: &'a Path) -> Self {
        Self::new(dir, &SYSTEM_VAULT)
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- lecture / écriture du fichier --------------------------------------------------------

    fn read_index(&self) -> LauncherResult<Index> {
        let path = self.dir.join(INDEX_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Index>(&bytes) {
                Ok(idx) => {
                    self.finish_legacy_cleanup(&idx);
                    Ok(idx)
                }
                Err(_) => {
                    // Liste illisible : on la garde de côté (rien n'est perdu) et on repart de zéro.
                    let _ = std::fs::rename(&path, self.dir.join("accounts.json.illisible"));
                    self.migrate_legacy()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.migrate_legacy(),
            Err(e) => Err(e.into()),
        }
    }

    fn write_index(&self, idx: &Index) -> LauncherResult<()> {
        std::fs::create_dir_all(self.dir)?;
        let out = Index {
            version: 1,
            active: idx.active.clone(),
            accounts: idx.accounts.clone(),
        };
        let data = serde_json::to_vec_pretty(&out)?;
        let tmp = self.dir.join("accounts.json.tmp");
        std::fs::write(&tmp, data)?;
        crate::secure::restrict_permissions(&tmp);
        std::fs::rename(&tmp, self.dir.join(INDEX_FILE))?;
        Ok(())
    }

    /// Range le jeton dans le coffre-fort ; sans coffre il reste dans la fiche écrite sur le disque.
    fn for_disk(&self, key: &str, account: &AzAccount) -> AzAccount {
        let mut on_disk = account.clone();
        if self.vault.store(&entry_name(key), &account.access_token) {
            on_disk.access_token = String::new();
        }
        on_disk
    }

    fn token_of(&self, stored: &StoredAccount) -> Option<AzAccount> {
        let mut acc = stored.account.clone();
        if acc.access_token.is_empty() {
            acc.access_token = self.vault.load(&entry_name(&stored.key))?;
        }
        Some(acc)
    }

    // ---- migration de l'ancien format ---------------------------------------------------------

    fn read_legacy(&self) -> Option<AzAccount> {
        let bytes = std::fs::read(account_session_path(self.dir)).ok()?;
        let mut acc: AzAccount = serde_json::from_slice(&bytes).ok()?;
        if acc.access_token.is_empty() {
            acc.access_token = self.vault.load(LEGACY_ENTRY)?;
        }
        if acc.site_url.trim().is_empty() {
            acc.site_url = DEFAULT_SITE.to_string();
        }
        Some(acc)
    }

    fn remove_legacy(&self) {
        let _ = std::fs::remove_file(account_session_path(self.dir));
        self.vault.delete(LEGACY_ENTRY);
    }

    fn migrate_legacy(&self) -> LauncherResult<Index> {
        // Pas d'ancienne session, ou inutilisable (illisible, jeton perdu) : on ne touche à rien.
        let Some(acc) = self.read_legacy() else {
            return Ok(Index::default());
        };
        let key = account_key(&acc);
        let idx = Index {
            version: 1,
            active: Some(key.clone()),
            accounts: vec![StoredAccount {
                key: key.clone(),
                needs_login: false,
                account: self.for_disk(&key, &acc),
            }],
        };
        self.write_index(&idx)?;
        // L'ancien format n'est supprimé qu'après relecture de la liste : le jeton doit y être retrouvé.
        let reread = std::fs::read(self.dir.join(INDEX_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .and_then(|i| i.accounts.into_iter().find(|s| s.key == key))
            .and_then(|s| self.token_of(&s))
            .map(|a| a.access_token == acc.access_token)
            .unwrap_or(false);
        if !reread {
            let _ = std::fs::remove_file(self.dir.join(INDEX_FILE));
            self.vault.delete(&entry_name(&key));
            return Err(LauncherError::Internal(
                "Migration de la session impossible : l'ancienne session est conservée.".into(),
            ));
        }
        self.remove_legacy();
        Ok(idx)
    }

    /// Reste d'une migration interrompue (liste écrite, ancien fichier pas encore supprimé).
    fn finish_legacy_cleanup(&self, idx: &Index) {
        if !account_session_path(self.dir).exists() {
            return;
        }
        if let Some(legacy) = self.read_legacy() {
            if idx.accounts.iter().any(|s| s.key == account_key(&legacy)) {
                self.remove_legacy();
            }
        }
    }

    // ---- opérations ---------------------------------------------------------------------------

    pub fn list(&self) -> LauncherResult<AccountList> {
        let _g = Self::lock();
        Ok(summarize(&self.read_index()?))
    }

    pub fn active_key(&self) -> LauncherResult<Option<String>> {
        let _g = Self::lock();
        Ok(self.read_index()?.active)
    }

    /// Compte (avec son jeton). `Err` si la clé est inconnue ; `Ok(None)` si son jeton a disparu du coffre-fort.
    pub fn get(&self, key: &str) -> LauncherResult<Option<AzAccount>> {
        let _g = Self::lock();
        let idx = self.read_index()?;
        let stored = idx
            .accounts
            .iter()
            .find(|s| s.key == key)
            .ok_or_else(|| LauncherError::Account("Compte inconnu.".into()))?;
        Ok(self.token_of(stored))
    }

    /// Compte actif avec son jeton, `None` s'il n'y en a pas (ou si son jeton a disparu).
    pub fn active(&self) -> LauncherResult<Option<AzAccount>> {
        let _g = Self::lock();
        let idx = self.read_index()?;
        let Some(key) = idx.active.as_deref() else {
            return Ok(None);
        };
        Ok(idx
            .accounts
            .iter()
            .find(|s| s.key == key)
            .and_then(|s| self.token_of(s)))
    }

    /// Tous les comptes avec leur jeton (masquage des journaux).
    pub fn all(&self) -> LauncherResult<Vec<AzAccount>> {
        let _g = Self::lock();
        let idx = self.read_index()?;
        Ok(idx.accounts.iter().filter_map(|s| self.token_of(s)).collect())
    }

    /// Ajoute ou met à jour un compte (connexion réussie). Renvoie sa clé.
    pub fn upsert(&self, account: &AzAccount, make_active: bool) -> LauncherResult<String> {
        self.put(account, make_active, None)
    }

    /// Met à jour un compte après vérification en ligne. `old_key` : clé sous laquelle il était
    /// rangé (l'id a pu apparaître depuis une ancienne session sans id). Il devient/reste actif.
    pub fn refresh(&self, old_key: &str, fresh: &AzAccount) -> LauncherResult<String> {
        self.put(fresh, true, Some(old_key))
    }

    fn put(&self, account: &AzAccount, make_active: bool, replaces: Option<&str>) -> LauncherResult<String> {
        let _g = Self::lock();
        let mut idx = self.read_index()?;
        let key = account_key(account);
        let site = normalize_site(&account.site_url);
        // Même compte sous une autre clé : celle remplacée, ou une ancienne session sans id (0).
        let is_twin = |s: &StoredAccount| {
            s.key != key
                && (replaces == Some(s.key.as_str())
                    || (account.id > 0
                        && s.account.id == 0
                        && normalize_site(&s.account.site_url) == site
                        && s.account.username.eq_ignore_ascii_case(&account.username)))
        };
        let known = idx.accounts.iter().any(|s| s.key == key || is_twin(s));
        if !known && idx.accounts.len() >= MAX_ACCOUNTS {
            return Err(LauncherError::Account(format!(
                "Limite de {MAX_ACCOUNTS} comptes atteinte : retirez-en un avant d'en ajouter."
            )));
        }
        let stored = StoredAccount {
            key: key.clone(),
            needs_login: false,
            account: self.for_disk(&key, account),
        };
        let mut stale = Vec::new();
        let mut list = Vec::new();
        let mut placed = false;
        let mut active = idx.active.clone();
        for s in idx.accounts.drain(..) {
            let twin = is_twin(&s);
            if s.key == key || twin {
                if twin {
                    stale.push(s.key.clone());
                    if active.as_deref() == Some(s.key.as_str()) {
                        active = Some(key.clone());
                    }
                }
                if !placed {
                    list.push(stored.clone());
                    placed = true;
                }
            } else {
                list.push(s);
            }
        }
        if !placed {
            list.push(stored);
        }
        idx.accounts = list;
        idx.active = if make_active { Some(key.clone()) } else { active };
        self.write_index(&idx)?;
        for old in stale {
            self.vault.delete(&entry_name(&old));
        }
        Ok(key)
    }

    pub fn set_active(&self, key: &str) -> LauncherResult<()> {
        let _g = Self::lock();
        let mut idx = self.read_index()?;
        if !idx.accounts.iter().any(|s| s.key == key) {
            return Err(LauncherError::Account("Compte inconnu.".into()));
        }
        idx.active = Some(key.to_string());
        self.write_index(&idx)
    }

    /// Le site a refusé le jeton : le compte reste dans la liste, marqué « reconnexion requise »,
    /// et n'est plus le compte actif.
    pub fn mark_needs_login(&self, key: &str) -> LauncherResult<()> {
        let _g = Self::lock();
        let mut idx = self.read_index()?;
        if let Some(s) = idx.accounts.iter_mut().find(|s| s.key == key) {
            s.needs_login = true;
        }
        if idx.active.as_deref() == Some(key) {
            idx.active = None;
        }
        self.write_index(&idx)
    }

    /// Retire un compte et son jeton. Renvoie le compte (jeton compris, s'il est retrouvé) pour la
    /// déconnexion côté site. Si c'était le compte actif, aucun autre n'est activé.
    pub fn remove(&self, key: &str) -> LauncherResult<Option<AzAccount>> {
        let _g = Self::lock();
        let mut idx = self.read_index()?;
        let Some(pos) = idx.accounts.iter().position(|s| s.key == key) else {
            return Ok(None);
        };
        let stored = idx.accounts.remove(pos);
        let with_token = self.token_of(&stored);
        if idx.active.as_deref() == Some(key) {
            idx.active = None;
        }
        self.write_index(&idx)?;
        self.vault.delete(&entry_name(key));
        Ok(with_token)
    }
}

fn summarize(idx: &Index) -> AccountList {
    AccountList {
        accounts: idx
            .accounts
            .iter()
            .map(|s| AccountSummary {
                key: s.key.clone(),
                id: s.account.id,
                username: s.account.username.clone(),
                skin_url: s.account.skin_url.clone(),
                site_url: s.account.site_url.clone(),
                role: s.account.role.clone(),
                active: idx.active.as_deref() == Some(s.key.as_str()),
                needs_login: s.needs_login,
            })
            .collect(),
        active: idx.active.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Coffre-fort en mémoire : les tests ne touchent jamais au vrai coffre-fort.
    #[derive(Default)]
    struct MemVault {
        entries: Mutex<HashMap<String, String>>,
        /// Simule un système sans coffre-fort (Linux).
        unavailable: bool,
    }
    impl TokenVault for MemVault {
        fn store(&self, entry: &str, token: &str) -> bool {
            if self.unavailable {
                return false;
            }
            self.entries.lock().unwrap().insert(entry.into(), token.into());
            true
        }
        fn load(&self, entry: &str) -> Option<String> {
            self.entries.lock().unwrap().get(entry).cloned()
        }
        fn delete(&self, entry: &str) {
            self.entries.lock().unwrap().remove(entry);
        }
    }
    impl MemVault {
        fn names(&self) -> Vec<String> {
            let mut v: Vec<String> = self.entries.lock().unwrap().keys().cloned().collect();
            v.sort();
            v
        }
    }

    fn acc(id: u64, name: &str, site: &str) -> AzAccount {
        AzAccount {
            id,
            username: name.into(),
            uuid: format!("uuid-{name}"),
            access_token: format!("tok-{name}-{id}"),
            email_verified: true,
            money: 1.0,
            banned: false,
            role: None,
            site_url: site.into(),
            skin_url: None,
        }
    }

    const LEGACY_JSON: &str = r##"{"id":0,"username":"Bob","uuid":"u","access_token":"t","email_verified":true,"money":5,"banned":false,"role":{"name":"VIP","color":"#fff","is_admin":false},"site_url":"https://geoventure.fr/","skin_url":null}"##;

    fn legacy_path(dir: &Path) -> std::path::PathBuf {
        account_session_path(dir)
    }

    #[test]
    fn migration_clear_text_session_becomes_active_account() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);

        let list = store.list().unwrap();
        assert_eq!(list.accounts.len(), 1);
        assert_eq!(list.accounts[0].username, "Bob");
        assert!(list.accounts[0].active);
        let active = store.active().unwrap().unwrap();
        assert_eq!(active.access_token, "t");
        assert_eq!(active.role.unwrap().name, "VIP");
        // Ancien format supprimé, jeton dans une entrée propre du coffre-fort, pas dans accounts.json.
        assert!(!legacy_path(tmp.path()).exists());
        assert_eq!(vault.names(), vec![entry_name(&list.active.clone().unwrap())]);
        let on_disk = std::fs::read_to_string(tmp.path().join(INDEX_FILE)).unwrap();
        assert!(!on_disk.contains("\"t\""));
        assert!(!on_disk.contains("access_token\": \"t"));
    }

    #[test]
    fn migration_reads_token_from_legacy_vault_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        vault.store(LEGACY_ENTRY, "jeton-coffre");
        let json = LEGACY_JSON.replace("\"access_token\":\"t\"", "\"access_token\":\"\"");
        std::fs::write(legacy_path(tmp.path()), json).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);

        assert_eq!(store.active().unwrap().unwrap().access_token, "jeton-coffre");
        assert!(vault.load(LEGACY_ENTRY).is_none());
        assert!(!legacy_path(tmp.path()).exists());
    }

    #[test]
    fn migration_without_vault_keeps_token_in_file() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault { unavailable: true, ..Default::default() };
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);

        assert_eq!(store.active().unwrap().unwrap().access_token, "t");
        assert!(!legacy_path(tmp.path()).exists());
        // Un second lancement relit la même chose, sans nouvelle migration.
        assert_eq!(store.list().unwrap().accounts.len(), 1);
    }

    #[test]
    fn migration_failure_keeps_legacy_session() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        // accounts.json ne peut pas être écrit : un dossier occupe sa place.
        std::fs::create_dir(tmp.path().join(INDEX_FILE)).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);

        assert!(store.list().is_err());
        assert!(legacy_path(tmp.path()).exists());
    }

    #[test]
    fn migration_ignores_unusable_legacy_session() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        // Jeton absent du fichier et du coffre-fort : rien à migrer, le fichier n'est pas touché.
        let json = LEGACY_JSON.replace("\"access_token\":\"t\"", "\"access_token\":\"\"");
        std::fs::write(legacy_path(tmp.path()), &json).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);
        assert!(store.list().unwrap().accounts.is_empty());
        assert!(legacy_path(tmp.path()).exists());
        // Fichier illisible : idem.
        std::fs::write(legacy_path(tmp.path()), "pas du json").unwrap();
        assert!(store.list().unwrap().accounts.is_empty());
        assert!(legacy_path(tmp.path()).exists());
    }

    #[test]
    fn interrupted_migration_is_finished() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);
        store.list().unwrap();
        // L'ancien fichier réapparaît (arrêt entre l'écriture de la liste et la suppression).
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        assert_eq!(store.list().unwrap().accounts.len(), 1);
        assert!(!legacy_path(tmp.path()).exists());
    }

    #[test]
    fn corrupted_index_is_set_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        std::fs::write(tmp.path().join(INDEX_FILE), "{ cassé").unwrap();
        let store = AccountStore::new(tmp.path(), &vault);
        assert!(store.list().unwrap().accounts.is_empty());
        assert!(tmp.path().join("accounts.json.illisible").exists());
        store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        assert_eq!(store.list().unwrap().accounts.len(), 1);
    }

    #[test]
    fn index_has_no_token_and_vault_has_one_entry_per_account() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        store.upsert(&acc(2, "B", "https://geoventure.fr/"), true).unwrap();
        let on_disk = std::fs::read_to_string(tmp.path().join(INDEX_FILE)).unwrap();
        assert!(!on_disk.contains("tok-A-1") && !on_disk.contains("tok-B-2"));
        assert_eq!(vault.names().len(), 2);
        assert_eq!(store.all().unwrap().len(), 2);
    }

    #[test]
    fn upsert_adds_then_updates_and_activates() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        let k1 = store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        let k2 = store.upsert(&acc(2, "B", "https://geoventure.fr/"), true).unwrap();
        assert_ne!(k1, k2);
        assert_eq!(store.list().unwrap().active.as_deref(), Some(k2.as_str()));

        // Même id : mise à jour (nouveau pseudo, nouveau jeton), pas de doublon.
        let mut updated = acc(1, "A2", "https://geoventure.fr/");
        updated.access_token = "nouveau".into();
        assert_eq!(store.upsert(&updated, true).unwrap(), k1);
        let list = store.list().unwrap();
        assert_eq!(list.accounts.len(), 2);
        assert_eq!(list.accounts[0].username, "A2");
        assert_eq!(list.active.as_deref(), Some(k1.as_str()));
        assert_eq!(store.active().unwrap().unwrap().access_token, "nouveau");

        // Sans activation : le compte actif ne change pas.
        let k3 = store.upsert(&acc(3, "C", "https://geoventure.fr/"), false).unwrap();
        assert_eq!(store.list().unwrap().active.as_deref(), Some(k1.as_str()));
        assert_ne!(k3, k1);
    }

    #[test]
    fn same_id_on_two_sites_makes_two_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        store.upsert(&acc(1, "A", "https://autre.example/"), true).unwrap();
        // Majuscules et « / » final ne créent pas de doublon.
        store.upsert(&acc(1, "A", "https://GEOVENTURE.fr"), true).unwrap();
        assert_eq!(store.list().unwrap().accounts.len(), 2);
    }

    #[test]
    fn switch_changes_active_account_only() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        let k1 = store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        let k2 = store.upsert(&acc(2, "B", "https://geoventure.fr/"), true).unwrap();
        store.set_active(&k1).unwrap();
        assert_eq!(store.active().unwrap().unwrap().username, "A");
        store.set_active(&k2).unwrap();
        assert_eq!(store.active().unwrap().unwrap().username, "B");
        assert!(store.set_active("inconnu").is_err());
        assert!(store.get("inconnu").is_err());
        assert_eq!(store.get(&k1).unwrap().unwrap().access_token, "tok-A-1");
    }

    #[test]
    fn needs_login_marks_without_deleting_and_deactivates() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        let k1 = store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        let k2 = store.upsert(&acc(2, "B", "https://geoventure.fr/"), true).unwrap();
        // Compte non actif refusé par le site : marqué, l'actif ne bouge pas.
        store.mark_needs_login(&k1).unwrap();
        let list = store.list().unwrap();
        assert!(list.accounts[0].needs_login && !list.accounts[1].needs_login);
        assert_eq!(list.active.as_deref(), Some(k2.as_str()));
        // Compte actif refusé : marqué et plus actif ; aucun autre n'est activé.
        store.mark_needs_login(&k2).unwrap();
        let list = store.list().unwrap();
        assert!(list.active.is_none());
        assert_eq!(list.accounts.len(), 2);
        assert!(store.active().unwrap().is_none());
        // Une nouvelle connexion lève la marque.
        store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        let list = store.list().unwrap();
        assert!(!list.accounts[0].needs_login);
        assert_eq!(list.active.as_deref(), Some(k1.as_str()));
    }

    #[test]
    fn remove_deletes_token_and_activates_nobody() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        let k1 = store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        let k2 = store.upsert(&acc(2, "B", "https://geoventure.fr/"), true).unwrap();
        // Retrait d'un compte non actif.
        let removed = store.remove(&k1).unwrap().unwrap();
        assert_eq!(removed.access_token, "tok-A-1");
        assert_eq!(vault.names(), vec![entry_name(&k2)]);
        assert_eq!(store.list().unwrap().active.as_deref(), Some(k2.as_str()));
        // Retrait de l'actif : plus aucun actif.
        store.upsert(&acc(3, "C", "https://geoventure.fr/"), false).unwrap();
        store.remove(&k2).unwrap();
        let list = store.list().unwrap();
        assert!(list.active.is_none());
        assert_eq!(list.accounts.len(), 1);
        assert_eq!(vault.names().len(), 1);
        // Clé inconnue : sans effet.
        assert!(store.remove("inconnu").unwrap().is_none());
    }

    #[test]
    fn account_limit_is_enforced_but_updates_still_work() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        for i in 1..=MAX_ACCOUNTS as u64 {
            store.upsert(&acc(i, &format!("J{i}"), "https://geoventure.fr/"), true).unwrap();
        }
        let err = store.upsert(&acc(99, "Trop", "https://geoventure.fr/"), true).unwrap_err();
        assert!(matches!(err, LauncherError::Account(_)));
        assert_eq!(store.list().unwrap().accounts.len(), MAX_ACCOUNTS);
        assert_eq!(vault.names().len(), MAX_ACCOUNTS);
        // Mettre à jour un compte existant reste possible au plafond.
        store.upsert(&acc(1, "J1", "https://geoventure.fr/"), true).unwrap();
        // Après un retrait, il y a de nouveau de la place.
        let key = account_key(&acc(2, "J2", "https://geoventure.fr/"));
        store.remove(&key).unwrap();
        store.upsert(&acc(99, "Trop", "https://geoventure.fr/"), true).unwrap();
    }

    #[test]
    fn refresh_rekeys_legacy_account_without_id() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);
        store.upsert(&acc(7, "Autre", "https://geoventure.fr/"), false).unwrap();
        let old = store.active_key().unwrap().unwrap();
        assert!(old.contains("#u:bob"));

        // La vérification en ligne révèle l'id : même compte, nouvelle clé, toujours actif, même place.
        let mut fresh = acc(5, "Bob", "https://geoventure.fr/");
        fresh.access_token = "t2".into();
        let new_key = store.refresh(&old, &fresh).unwrap();
        assert_ne!(new_key, old);
        let list = store.list().unwrap();
        assert_eq!(list.accounts.len(), 2);
        assert_eq!(list.accounts[0].username, "Bob");
        assert_eq!(list.active.as_deref(), Some(new_key.as_str()));
        assert_eq!(vault.names().len(), 2);
        assert!(vault.load(&entry_name(&old)).is_none());
        assert_eq!(store.active().unwrap().unwrap().access_token, "t2");
    }

    #[test]
    fn login_after_migration_does_not_duplicate_legacy_account() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        std::fs::write(legacy_path(tmp.path()), LEGACY_JSON).unwrap();
        let store = AccountStore::new(tmp.path(), &vault);
        store.list().unwrap();
        store.upsert(&acc(5, "bob", "https://geoventure.fr/"), true).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.accounts.len(), 1);
        assert_eq!(vault.names().len(), 1);
    }

    #[test]
    fn lost_vault_token_gives_no_session() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = MemVault::default();
        let store = AccountStore::new(tmp.path(), &vault);
        let k = store.upsert(&acc(1, "A", "https://geoventure.fr/"), true).unwrap();
        vault.delete(&entry_name(&k));
        assert!(store.active().unwrap().is_none());
        assert!(store.get(&k).unwrap().is_none());
        assert_eq!(store.list().unwrap().accounts.len(), 1);
    }

    #[test]
    fn keys_are_stable_and_safe() {
        let a = acc(42, "Joueur", "https://Geoventure.fr/");
        assert_eq!(account_key(&a), "geoventure.fr#42");
        assert_eq!(account_key(&acc(0, "Bob Le", "")), "geoventure.fr#u:bob_le");
    }
}
