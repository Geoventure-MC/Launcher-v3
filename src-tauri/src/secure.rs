//! Coffre-fort du système pour les jetons de session Azuriom (une entrée par compte).
//!
//! Windows : Gestionnaire d'identification ; macOS : Trousseau. Ailleurs (Linux), aucun coffre
//! n'est garanti : le jeton reste dans le fichier de session, lisible par son seul propriétaire.

#[cfg(any(windows, target_os = "macos"))]
const SERVICE: &str = "fr.geoventure.launcher";

/// Entrée du coffre-fort de l'ancien format (un seul compte). Lue une fois pour la migration.
pub const LEGACY_ENTRY: &str = "azuriom_session";

/// Coffre-fort des jetons, une entrée par compte. Un trait pour que les tests n'aient jamais
/// à toucher au vrai coffre-fort du système.
pub trait TokenVault: Send + Sync {
    /// Range le jeton. `true` si c'est fait (sinon il reste dans le fichier d'index).
    fn store(&self, entry: &str, token: &str) -> bool;
    /// Relit le jeton.
    fn load(&self, entry: &str) -> Option<String>;
    /// Efface le jeton.
    fn delete(&self, entry: &str);
}

/// Coffre-fort du système (Windows, macOS). Ailleurs : aucun coffre, le jeton reste dans le fichier.
pub struct SystemVault;

impl TokenVault for SystemVault {
    fn store(&self, entry: &str, token: &str) -> bool {
        #[cfg(any(windows, target_os = "macos"))]
        {
            keyring::Entry::new(SERVICE, entry)
                .and_then(|e| e.set_password(token))
                .is_ok()
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (entry, token);
            false
        }
    }

    fn load(&self, entry: &str) -> Option<String> {
        #[cfg(any(windows, target_os = "macos"))]
        {
            keyring::Entry::new(SERVICE, entry)
                .and_then(|e| e.get_password())
                .ok()
                .filter(|t| !t.is_empty())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = entry;
            None
        }
    }

    fn delete(&self, entry: &str) {
        #[cfg(any(windows, target_os = "macos"))]
        {
            if let Ok(e) = keyring::Entry::new(SERVICE, entry) {
                let _ = e.delete_credential();
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = entry;
        }
    }
}

/// Fichier lisible par son seul propriétaire (Unix). Sans effet ailleurs.
pub fn restrict_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}
