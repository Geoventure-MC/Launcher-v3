# Geoventure Launcher (Rust + Tauri v2)

Réécriture moderne du launcher Geoventure en **Rust + Tauri v2**, remplaçant Electron (`Launcher-v3`) par un binaire natif léger, rapide et résilient.

## Architecture

- `src-tauri/src/config.rs` : Client HTTP résilient (`reqwest` + `rustls-tls`) avec **timeouts stricts** (connexion 5 s, requête 12 s, lecture 10 s) pour éliminer définitivement les écrans de chargement infinis, normalisation HTTPS et **cache disque hors-ligne** par instance/canal (`?instance=<id>&channel=test`).
- `src-tauri/src/azauth.rs` : Authentification **Azuriom** native (`/api/auth/authenticate`, `/api/auth/verify`, `/api/auth/logout`) avec support 2FA et HTTPS forcé.
- `src-tauri/src/downloader.rs` : Vérification différentielle et téléchargeur parallèle (`tokio::Semaphore`, retries backoff, streaming `.part` atomique) avec validation **SHA-1 + SHA-256** et protection contre le path traversal.
- `src-tauri/src/java.rs` : Détection multi-OS des JDK installés (priorité **JDK 17**), validation syntaxique sécurisée (blocage chemins relatifs/UNC) et sondage `java -version` sans shell.
- `src-tauri/src/launch.rs` : Profils de performance (`eco` / `balanced` / `performance`), plafonnement RAM à 60 %, injection `-Dfile.encoding=UTF-8`, rétrocompatibilité des dossiers d'instance `.geoventure`, et générateur d'arguments **Minecraft Forge 1.20.1** (`BootstrapLauncher`).
- `src-tauri/src/logs.rs` : Ring-buffer de logs structurés avec streaming temps réel vers l'interface (`launcher://log`).
- `src-tauri/src/commands.rs` : Pont IPC Tauri v2 exposant toutes les commandes au frontend + raccourci **F12** garanti (`devtools` activé).
- `frontend/` : Interface HTML/CSS/JS légère (connexion Azuriom, sélection d'instance/canal, barre de synchronisation, console de logs intégrée).

## Prérequis Windows

1. **Rust (rustup + cargo)** + **Visual Studio C++ Build Tools (MSVC)** — exécuter :
   ```powershell
   .\scripts\install-rust.ps1
   ```
2. **Microsoft Edge WebView2** (déjà présent sur Windows 10/11).

## Commandes

```powershell
# Vérification compilation Rust
npm run check:rust

# Tests unitaires Rust (config hors-ligne, Azuriom 2FA, downloader SHA-1/SHA-256, Java 17, Forge 1.20.1)
npm run test:rust

# Lancer en développement
npm run dev

# Générer les installateurs (NSIS Windows / AppImage Linux)
npm run build
```
