# Launcher Geoventure (Rust / Tauri v2)

Launcher du serveur Geoventure : backend Rust (`src-tauri/`), interface HTML/CSS/JS sans framework
(`frontend/`). Il lit sa configuration sur le panel (dépôt `panel`). Adresse par défaut du panel :
`https://launcher.geoventure.fr/` (modifiable par le joueur dans Réglages › Avancé). Minecraft 1.20.1, Forge.
L'ancien launcher Electron est conservé sur la branche `electron` (dernière version : il installe celui-ci).

## Règles

- **100 % en français** (commits, releases, PR) : `Ajout :`, `Correction :`, `Amélioration :`, `Documentation :`, `Maintenance :`.
- Branche `master` (pas de `main`). Un push sur `master` publie une version
  pour tous les joueurs (workflow `rust-release.yml`, mise à jour automatique) : ne pousser que du travail fini et testé.
  Un commit qui ne touche que la documentation ou des images : `[skip ci]` dans le message, sinon il publie une version.
- Les versions Rust ne doivent JAMAIS être marquées « Latest » sur GitHub : « Latest » reste la dernière
  version Electron, qui fait passer les anciens launchers à celui-ci.
- Aucun secret dans le dépôt (clé de signature : secret `TAURI_SIGNING_PRIVATE_KEY`).

## Construire et tester

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # tests Rust
npm ci && npm run test:ui                         # tests de l'interface (Playwright, faux pont Tauri)
cargo build --manifest-path src-tauri/Cargo.toml  # binaire de dev (pas de mise à jour automatique en dev)
```
Après une modification de `frontend/`, toucher `src-tauri/src/lib.rs` avant `cargo build` (l'interface est embarquée).

## Où est quoi

`src-tauri/src/` : `commands.rs` (commandes appelées par l'interface), `install.rs` (Minecraft, Java, Forge),
`downloader.rs` (modpack), `config.rs` (panel), `azauth.rs` (connexion Azuriom), `crash.rs` (analyse des plantages),
`media.rs` (mods communauté, captures, packs), `discord.rs`, `tray.rs`, `secure.rs` (coffre-fort du système).
`frontend/` : `index.html`, `app.js`, `styles.css`, `i18n.js` (anglais). `tests/ui/` : tests de l'interface.

## Contrat avec le panel : à ne pas casser

Routes lues : `/utils/api`, `/utils/mods`, `/utils/notifications`, `/utils/servers-status`, `/utils/community-mods`,
`/utils/leaderboards`, `/utils/factions`, `/utils/achievements`, `/utils/seasons`, `/utils/wonder`, `/utils/game-events`,
`/utils/launcher-content`, `/utils/servers-history`, `/data`. Liste fermée dans `PANEL_READ_ROUTES` (`commands.rs`).
Détail historique du contrat : `docs/claude/reference.md`, « Contrat Panel ↔ Launcher » (écrit pour l'ancien launcher).

Dossier du jeu identique à l'ancien launcher (`%APPDATA%\.Nexus`, `~/.Nexus`…) : les joueurs gardent leurs fichiers.
