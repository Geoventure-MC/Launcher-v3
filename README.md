# Geoventure Launcher (Rust + Tauri v2)

Nouveau launcher Geoventure, en Rust + Tauri v2, destiné à remplacer le launcher Electron
(`master`). Binaire natif de quelques Mo, et il installe le jeu tout seul sur un PC neuf.

## Ce que fait le bouton « Jouer »

1. **Minecraft 1.20.1** : JSON de version, librairies, client, ~3 600 ressources (Mojang).
2. **Java 17** : le Java fourni par Mojang (`java-runtime-gamma`) dans `runtime/`. Un Java choisi
   dans les réglages est utilisé à la place s'il est valide.
3. **Forge** : installeur officiel, librairies, puis les processeurs qui patchent le client.
   Délai de 3 min sans sortie par processeur : arrêt et message clair au lieu d'un écran figé.
4. **Modpack** du panel (`/data?instance=…`), vérifié par empreinte. Les anciens jars retirés du
   modpack sont supprimés de `mods/`. Les réglages du joueur (`options.txt`, `config/…` déjà
   présents) et les mods désactivés (`.jar-disable`) ne sont jamais écrasés.
5. **Lancement** (connexion directe au serveur de l'instance), `earlyWindowControl=false` dans
   `config/fml.toml`.

Les dossiers sont ceux du launcher Electron (`%APPDATA%\.Nexus`, `instances/<slug>`,
`instances/<slug>-test`, `runtime/`, `loader/forge/`…) : un joueur qui change de launcher ne
re-télécharge rien. Deuxième lancement : quelques secondes de vérification.

Testé ici (Linux, Xvfb) : dossier vide → écran titre avec Forge et le modpack Geoventure en ~75 s.

## Interface

Même esprit que le launcher Electron : fond du serveur choisi (zoom lent), barre latérale vitrée
(tête du joueur, logos des serveurs, canal S/T, bouton Jouer lumineux), animations (écran de
chargement, apparition des cartes, transitions, progression scintillante) ; option « Réduire les
animations ».

- **Connexion Azuriom** par le site du panel (`azauth`), double authentification, messages
  d'erreur en français, liens « Créer un compte » et « Mot de passe oublié ».
- **Accueil** : actualité du site (flux RSS `/api/rss`) en carrousel + liste, raccourcis et
  « À découvrir » du panel (`/utils/launcher-content`), statut du serveur (joueurs, ping),
  annonces, événements avec compte à rebours, maintenance, liste blanche, e-mail vérifié.
- **Profil** : succès, classement rafraîchi toutes les 30 s, saison, factions.
- **Performances** : processeur, mémoire totale et libre, système ; bouton « Optimiser pour mon
  PC » (profil conseillé, RAM plafonnée à 60 %) ; profils Économe / Équilibré / Performant ;
  alerte si la mémoire libre est trop faible.
- **Diagnostic** : système, Java, installation, sondes (panel, Azuriom, Mojang, Forge), journaux
  du launcher et du jeu, dernier rapport de plantage, tout masqué (jetons, e-mails, chemins
  personnels). Copier, enregistrer, ou envoyer au panel (`POST /utils/diagnostic`, facultatif).
  Proposé automatiquement si le jeu se ferme anormalement.
- **Réglages** : résolution, comportement pendant la partie, connexion directe au serveur, mods
  optionnels, Java, **adresse du panel** (testée avant d'être utilisée), réparation 1 clic,
  dossier du jeu, mise à jour, déconnexion.
- **Console** colorée, défilement automatique, copie.

## Mises à jour automatiques

Au démarrage, le launcher lit `latest.json` dans la release GitHub **`rust-latest`**, vérifie la
signature et s'installe. Le workflow `.github/workflows/rust-release.yml` s'en charge à chaque
push sur la branche `rust` : tests, build Windows (NSIS) + Linux (AppImage, deb), **pré-version**
`rust-v2.0.<n>`, recopie de `latest.json`. Les pré-versions n'apparaissent pas au launcher Electron.

**À faire une fois :** ajouter le secret `TAURI_SIGNING_PRIVATE_KEY` (Settings → Secrets and
variables → Actions), avec la clé privée qui correspond à la clé publique de
`src-tauri/tauri.conf.json`. Sans lui, le workflow s'arrête avec un message clair. Ne jamais
committer cette clé ; si elle est perdue, générer une nouvelle paire (`npx tauri signer generate`)
et remplacer la clé publique : les launchers déjà installés devront alors être réinstallés à la main.

## Pas encore fait

- **Passage automatique Electron → Rust** : le launcher actuel ne sait pas encore installer
  celui-ci. À décider avant de le remplacer (par ex. une dernière version Electron qui télécharge
  l'installateur Rust).
- macOS (non construit par le workflow, pas de signature Apple).
- Discord Rich Presence, mods communauté, captures d'écran, vidéo, affluence, nouveautés
  (changelog), télémétrie, anglais : présents dans le launcher Electron, pas encore ici.
- Connexion Azuriom avec un vrai compte et partie sur le vrai serveur : non testées (refus
  d'identifiants vérifié contre geoventure.fr, mais pas de compte de test).

## Commandes

```bash
npm ci
npm test                       # tests Rust
npx tauri dev                  # lancer en développement
npx tauri build                # installateurs (signature : TAURI_SIGNING_PRIVATE_KEY)

# Essai complet sans interface : installe tout dans un dossier, puis lance le jeu
cargo run --manifest-path src-tauri/Cargo.toml --example install_e2e -- /tmp/geo-test --launch
```

Linux : `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`. Windows : `scripts/install-rust.ps1`
(Rust + outils MSVC) ; WebView2 est déjà présent sur Windows 10/11.
