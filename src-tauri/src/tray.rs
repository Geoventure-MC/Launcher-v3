//! Icône dans la zone de notification (Ouvrir, Jouer, Quitter) et démarrage avec l'ordinateur.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

/// L'icône existe (sinon on ne cache jamais la fenêtre : le joueur n'aurait aucun moyen de la rouvrir).
pub static TRAY_OK: AtomicBool = AtomicBool::new(false);
/// Fermer la fenêtre la range dans la zone de notification au lieu de quitter.
pub static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);

/// Argument passé par le démarrage automatique : le launcher démarre réduit.
pub const MINIMIZED_ARG: &str = "--minimized";

/// Linux : l'icône a besoin de la bibliothèque appindicator ; sans elle, pas d'icône.
#[cfg(target_os = "linux")]
fn tray_supported() -> bool {
    let names = ["libayatana-appindicator3.so.1", "libappindicator3.so.1"];
    let mut dirs: Vec<std::path::PathBuf> = ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/lib/x86_64-linux-gnu"]
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    if let Some(appdir) = std::env::var_os("APPDIR") {
        let a = std::path::PathBuf::from(appdir);
        dirs.push(a.join("usr/lib"));
        dirs.push(a.join("usr/lib/x86_64-linux-gnu"));
    }
    dirs.iter().any(|d| names.iter().any(|n| d.join(n).exists()))
}

#[cfg(not(target_os = "linux"))]
fn tray_supported() -> bool {
    true
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    if !tray_supported() {
        return Ok(());
    }
    let open = MenuItem::with_id(app, "open", "Ouvrir le launcher", true, None::<&str>)?;
    let play = MenuItem::with_id(app, "play", "Jouer", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &play, &sep, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Nexus Launcher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "play" => {
                show_main(app);
                let _ = app.emit("launcher://tray-play", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    TRAY_OK.store(true, Ordering::SeqCst);
    Ok(())
}
