//! Coffre-fort du système pour le jeton de session Azuriom.
//!
//! Windows : Gestionnaire d'identification ; macOS : Trousseau. Ailleurs (Linux), aucun coffre
//! n'est garanti : le jeton reste dans le fichier de session, lisible par son seul propriétaire.

#[cfg(any(windows, target_os = "macos"))]
const SERVICE: &str = "fr.geoventure.launcher";
#[cfg(any(windows, target_os = "macos"))]
const ACCOUNT: &str = "azuriom_session";

/// Range le jeton dans le coffre-fort. `true` si c'est fait (sinon il reste dans le fichier).
pub fn store_token(token: &str) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        keyring::Entry::new(SERVICE, ACCOUNT)
            .and_then(|e| e.set_password(token))
            .is_ok()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = token;
        false
    }
}

/// Relit le jeton du coffre-fort.
pub fn load_token() -> Option<String> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        keyring::Entry::new(SERVICE, ACCOUNT)
            .and_then(|e| e.get_password())
            .ok()
            .filter(|t| !t.is_empty())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Efface le jeton du coffre-fort (déconnexion).
pub fn delete_token() {
    #[cfg(any(windows, target_os = "macos"))]
    {
        if let Ok(e) = keyring::Entry::new(SERVICE, ACCOUNT) {
            let _ = e.delete_credential();
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
