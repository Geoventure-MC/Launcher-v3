//! Ancienne installation « Geoventure Launcher » (avant le renommage en « Nexus Launcher », 2.0.19).
//!
//! L'installeur NSIS de Tauri range le launcher dans `%LOCALAPPDATA%\<productName>` et retrouve l'installation
//! précédente par une clé de registre qui contient aussi `productName`. Le renommage a donc installé
//! « Nexus Launcher » À CÔTÉ de « Geoventure Launcher » au lieu de le remplacer : le joueur qui clique sur
//! son ancien raccourci relance l'ancienne version, qui refait la mise à jour à chaque lancement.
//!
//! Correctif : au démarrage, le nouveau launcher redirige les raccourcis « Geoventure Launcher » vers lui-même
//! (bureau, menu Démarrer, barre des tâches épinglée), en les renommant « Nexus Launcher ». Il ne lance PAS
//! le désinstalleur de l'ancienne version : celui-ci ferme tous les processus `geoventure-launcher.exe`,
//! nom commun aux deux versions, et fermerait donc le launcher en cours.

use std::path::{Path, PathBuf};

/// Nom de l'ancien produit, donc de son dossier d'installation et de ses raccourcis.
pub const LEGACY_PRODUCT: &str = "Geoventure Launcher";
/// Nom actuel, donné aux raccourcis redirigés.
pub const PRODUCT: &str = "Nexus Launcher";

/// Le launcher en cours vient-il d'ailleurs que l'ancienne installation, alors que celle-ci existe encore ?
/// Seul ce cas demande de rediriger les raccourcis (l'ancienne version, elle, n'a pas ce code).
pub fn needs_redirect(current_exe: &Path, legacy_dir: &Path, legacy_exists: bool) -> bool {
    if !legacy_exists {
        return false;
    }
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let exe = norm(current_exe);
    let mut dir = norm(legacy_dir);
    if !dir.ends_with('\\') {
        dir.push('\\');
    }
    !exe.starts_with(&dir)
}

/// Dossiers où l'installeur (ou le joueur) a pu poser un raccourci vers l'ancienne version.
fn shortcut_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        let a = PathBuf::from(appdata);
        out.push(a.join("Microsoft\\Windows\\Start Menu\\Programs"));
        out.push(a.join("Microsoft\\Internet Explorer\\Quick Launch\\User Pinned\\TaskBar"));
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        out.push(PathBuf::from(&profile).join("Desktop"));
        out.push(PathBuf::from(&profile).join("OneDrive\\Desktop"));
        out.push(PathBuf::from(&profile).join("OneDrive\\Bureau"));
    }
    out
}

/// Script PowerShell qui redirige chaque raccourci « Geoventure Launcher.lnk » vers `target`
/// et le renomme « Nexus Launcher.lnk » (sauf si ce nom existe déjà : l'ancien est alors retiré).
pub fn redirect_script(target: &Path, dirs: &[PathBuf]) -> String {
    let q = |s: &str| s.replace('\'', "''");
    let target = q(&target.to_string_lossy());
    let workdir = q(&Path::new(&*target).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    let list = dirs
        .iter()
        .map(|d| format!("'{}'", q(&d.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "$ErrorActionPreference='SilentlyContinue'; $sh = New-Object -ComObject WScript.Shell; \
         foreach ($d in @({list})) {{ \
           $old = Join-Path $d '{legacy}.lnk'; if (-not (Test-Path -LiteralPath $old)) {{ continue }}; \
           $new = Join-Path $d '{product}.lnk'; \
           if (Test-Path -LiteralPath $new) {{ Remove-Item -LiteralPath $old -Force; continue }}; \
           $l = $sh.CreateShortcut($old); $l.TargetPath = '{target}'; $l.WorkingDirectory = '{workdir}'; \
           $l.IconLocation = '{target},0'; $l.Save(); Rename-Item -LiteralPath $old -NewName '{product}.lnk' }}",
        legacy = LEGACY_PRODUCT,
        product = PRODUCT,
    )
}

/// Au démarrage (Windows) : redirige les raccourcis de l'ancienne installation. Sans effet ailleurs,
/// et jamais bloquant : tourne dans un fil à part, les erreurs sont ignorées.
pub fn redirect_legacy_shortcuts() {
    #[cfg(windows)]
    std::thread::spawn(|| {
        use std::os::windows::process::CommandExt;
        let Ok(exe) = std::env::current_exe() else { return };
        let Ok(local) = std::env::var("LOCALAPPDATA") else { return };
        let legacy_dir = PathBuf::from(local).join(LEGACY_PRODUCT);
        if !needs_redirect(&exe, &legacy_dir, legacy_dir.is_dir()) {
            return;
        }
        let script = redirect_script(&exe, &shortcut_dirs());
        let _ = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
            .creation_flags(0x0800_0000)
            .status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_only_from_the_new_install() {
        let legacy = Path::new(r"C:\Users\a\AppData\Local\Geoventure Launcher");
        let new_exe = Path::new(r"C:\Users\a\AppData\Local\Nexus Launcher\geoventure-launcher.exe");
        let old_exe = Path::new(r"C:\Users\a\AppData\Local\Geoventure Launcher\geoventure-launcher.exe");
        assert!(needs_redirect(new_exe, legacy, true));
        assert!(!needs_redirect(new_exe, legacy, false), "sans ancienne installation, rien à faire");
        assert!(!needs_redirect(old_exe, legacy, true), "l'ancienne version ne se redirige pas vers elle-même");
        let upper = Path::new(r"C:\USERS\A\APPDATA\LOCAL\GEOVENTURE LAUNCHER\geoventure-launcher.exe");
        assert!(!needs_redirect(upper, legacy, true), "comparaison insensible à la casse");
        let sibling = Path::new(r"C:\Users\a\AppData\Local\Geoventure Launcher 2\geoventure-launcher.exe");
        assert!(needs_redirect(sibling, legacy, true), "un dossier au nom voisin n'est pas l'ancienne installation");
    }

    #[test]
    fn script_targets_the_running_exe_and_escapes_quotes() {
        let exe = Path::new(r"C:\Users\O'Neil\AppData\Local\Nexus Launcher\geoventure-launcher.exe");
        let s = redirect_script(exe, &[PathBuf::from(r"C:\Users\O'Neil\Desktop")]);
        assert!(s.contains(r"'C:\Users\O''Neil\AppData\Local\Nexus Launcher\geoventure-launcher.exe'"));
        assert!(s.contains(r"'C:\Users\O''Neil\Desktop'"));
        assert!(s.contains("Geoventure Launcher.lnk") && s.contains("Nexus Launcher.lnk"));
    }
}
