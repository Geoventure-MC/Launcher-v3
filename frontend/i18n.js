// Traduction de l'interface (français par défaut, anglais au choix).
// L'interface est écrite en français ; en anglais, chaque texte affiché est traduit au vol :
// textes exacts d'abord, puis modèles avec variables. Les contenus du site et du panel
// (actualités, annonces, mods) et les journaux restent tels quels.
(() => {
  'use strict';

  const EXACT = {
    // Navigation, en-têtes
    'Accueil': 'Home', 'Mon profil': 'My profile', 'Profil': 'Profile', 'Performances': 'Performance', '⚡ Performances': '⚡ Performance',
    'Diagnostic': 'Diagnostics', '🩺 Diagnostic': '🩺 Diagnostics', 'Console': 'Console', 'Réglages': 'Settings', 'Navigation': 'Navigation',
    'Serveurs': 'Servers', 'Canal': 'Channel', 'Stable': 'Stable', 'Test': 'Test', 'Jouer': 'Play', 'Compte': 'Account',
    'ACTUALITÉ': 'NEWS', 'Raccourcis': 'Shortcuts', 'À découvrir': 'Discover', 'Dernières actualités': 'Latest news',
    'Chargement de l’actualité…': 'Loading news…', 'Chargement du launcher…': 'Loading launcher…', 'Chargement…': 'Loading…',
    'Connexion…': 'Connecting…', 'Actualiser le statut': 'Refresh status', 'Rejoindre le serveur': 'Join the server',
    '— joueurs': '— players', 'Affluence': 'Player count', 'Joueurs connectés heure par heure': 'Players online hour by hour',
    'Vidéo de la communauté': 'Community video', 'Lire la vidéo': 'Play video', 'Voir sur YouTube': 'Watch on YouTube',
    'Événements': 'Events', 'Aucun événement pour le moment.': 'No events right now.', 'Votre installation': 'Your installation',
    'Version': 'Version', 'Java': 'Java', 'Mémoire': 'Memory', 'Java 17 fourni': 'Bundled Java 17', 'Personnalisé': 'Custom',
    'Économe': 'Light', 'Équilibré': 'Balanced', 'Performant': 'High', 'Conseillé': 'Recommended', '⭐ Recommandé': '⭐ Recommended',
    'Fermer le jeu': 'Close the game', '✕ Fermer': '✕ Close', '✕ Fermer le jeu': '✕ Close the game', 'En jeu': 'In game',
    'Lancement…': 'Launching…', 'Le jeu est ouvert': 'The game is open',
    // Connexion
    'Bienvenue sur': 'Welcome to', 'Connectez-vous avec votre compte du site': 'Sign in with your account from',
    'E-mail ou pseudo': 'E-mail or username', 'Mot de passe': 'Password', 'Code de double authentification': 'Two-factor code',
    'Se connecter': 'Sign in', 'Créer un compte': 'Create an account', 'Mot de passe oublié ?': 'Forgot password?',
    'Se déconnecter': 'Sign out', 'Connectez-vous pour jouer.': 'Sign in to play.', 'Connexion refusée.': 'Sign-in refused.',
    'E-mail, pseudo ou mot de passe incorrect.': 'Wrong e-mail, username or password.',
    'Code de double authentification incorrect.': 'Wrong two-factor code.',
    'Entrez le code de votre application de double authentification.': 'Enter the code from your two-factor app.',
    'Ce compte est banni du serveur.': 'This account is banned from the server.',
    'Vérifiez d’abord votre adresse e-mail sur le site.': 'Verify your e-mail address on the website first.',
    'Vérifiez votre adresse e-mail sur le site pour jouer.': 'Verify your e-mail address on the website to play.',
    'Accès réservé (liste blanche).': 'Restricted access (whitelist).',
    // Profil
    'Profil et succès': 'Profile and achievements', 'Succès': 'Achievements', 'lancements': 'launches', 'de jeu': 'played',
    'points de succès': 'achievement points', 'Classement': 'Leaderboard', 'Saison': 'Season', 'Factions': 'Factions', 'Faction': 'Faction',
    'Membres': 'Members', 'Puissance': 'Power', 'Mis à jour toutes les 30 s': 'Updated every 30 s', 'Aucun succès disponible.': 'No achievements available.',
    'Aucune faction.': 'No factions.', 'Pas de saison en cours.': 'No season in progress.', 'Pas encore de classement.': 'No leaderboard yet.',
    'Succès secret': 'Secret achievement', 'À découvrir…': 'To be discovered…',
    // Performances
    'Processeur': 'Processor', 'Mémoire libre': 'Free memory', 'Système': 'System', 'Carte graphique': 'Graphics card', 'Non détectée': 'Not detected',
    'Optimisation automatique': 'Automatic optimisation', '⚡ Optimiser pour mon PC': '⚡ Optimise for my PC',
    'Le launcher choisit la mémoire et les réglages Java adaptés à votre PC.': 'The launcher picks the memory and Java settings that suit your PC.',
    'PC modeste, 8 Go ou moins': 'Modest PC, 8 GB or less', 'La plupart des PC': 'Most PCs', '16 Go et 6 cœurs ou plus': '16 GB and 6 cores or more',
    'Mémoire allouée au jeu': 'Memory given to the game', 'Minimum :': 'Minimum:', 'Maximum :': 'Maximum:', 'Mémoire insuffisante': 'Not enough memory',
    'Arguments Java (un par ligne)': 'Java arguments (one per line)', 'Arguments Java enregistrés.': 'Java arguments saved.',
    'Avec le profil Économe, alléger aussi les réglages du jeu (distance de vue 8, ombres, nuages et particules réduits). Vos réglages sont gardés de côté.':
      'With the Light profile, also lighten the game settings (render distance 8, fewer shadows, clouds and particles). Your settings are kept aside.',
    'Remettre mes réglages du jeu': 'Restore my game settings', 'Impossible de lire le matériel du PC.': 'Unable to read the PC hardware.',
    'Réglages légers appliqués au prochain lancement.': 'Light settings applied on next launch.',
    'Réglages légers : actifs quand le profil Économe est choisi.': 'Light settings: active when the Light profile is chosen.',
    'Le jeu garde vos réglages. « Remettre mes réglages du jeu » rétablit ceux d’avant.': 'The game keeps your settings. “Restore my game settings” brings back the previous ones.',
    'Vos réglages du jeu d’avant le profil Économe sont remis.': 'Your game settings from before the Light profile are restored.',
    'Rien à remettre : le launcher n’a pas modifié vos réglages du jeu.': 'Nothing to restore: the launcher has not changed your game settings.',
    '⚠️ Ce PC n’a pas assez de mémoire pour le modpack (3 Go utilisables minimum).': '⚠️ This PC does not have enough memory for the modpack (3 GB usable minimum).',
    '· réglages personnalisés': '· custom settings',
    // Jeu
    '🎮 Jeu': '🎮 Game', 'Résolution de la fenêtre': 'Window size', 'Par défaut': 'Default', 'Pendant la partie': 'While playing',
    'Garder le launcher ouvert': 'Keep the launcher open', 'Cacher le launcher (réaffiché à la fermeture du jeu)': 'Hide the launcher (shown again when the game closes)',
    'Fermer le launcher': 'Close the launcher', 'Se connecter directement au serveur au lancement': 'Join the server directly at launch',
    'Réduire les animations': 'Reduce animations', 'Afficher mon activité Geoventure sur Discord': 'Show my Geoventure activity on Discord',
    'Fermer la fenêtre range le launcher près de l’horloge (zone de notification)': 'Closing the window keeps the launcher next to the clock (system tray)',
    'Ouvrir le launcher au démarrage de l’ordinateur (réduit)': 'Open the launcher when the computer starts (minimised)',
    'Le launcher s’ouvrira au démarrage de l’ordinateur.': 'The launcher will open when the computer starts.',
    'Le launcher ne s’ouvrira plus au démarrage.': 'The launcher will no longer open at startup.',
    // Mods, contenu
    '🧩 Mods': '🧩 Mods', 'Mods optionnels': 'Optional mods', 'Aucun mod optionnel.': 'No optional mods.', 'Aucun mod optionnel pour ce serveur.': 'No optional mods for this server.',
    'Mods de la communauté': 'Community mods', 'Rechercher un mod…': 'Search for a mod…', 'Rechercher un mod de la communauté': 'Search community mods',
    'Des mods conseillés par l’équipe, à ajouter si vous le souhaitez. Ils restent installés aux mises à jour du modpack.':
      'Mods recommended by the team, to add if you like. They stay installed when the modpack updates.',
    'Tous': 'All', 'Installer': 'Install', 'Retirer': 'Remove', 'Page du mod': 'Mod page', 'Aucun mod ne correspond.': 'No matching mod.',
    'Aucun mod communauté pour le moment.': 'No community mods yet.', 'Liste des mods communauté injoignable pour le moment.': 'Community mod list unreachable right now.',
    '🖼️ Contenu': '🖼️ Content', 'Captures d’écran': 'Screenshots', '📂 Dossier': '📂 Folder', 'Aucune capture pour le moment.': 'No screenshots yet.',
    'Touche F2 en jeu. Cliquez sur une image pour l’ouvrir.': 'Press F2 in game. Click an image to open it.',
    'Packs de ressources': 'Resource packs', 'Déposez un pack (.zip) dans le dossier, puis activez-le en jeu (Options › Packs de ressources).':
      'Drop a pack (.zip) into the folder, then enable it in game (Options › Resource Packs).',
    'Shaders': 'Shaders', 'Aucun': 'None', 'Activer': 'Enable', 'Actif': 'Active', 'Supprimer': 'Delete', 'Confirmer ?': 'Confirm?', 'Dossier vide.': 'Empty folder.',
    'Désactiver les shaders': 'Disable shaders', 'Shaders désactivés': 'Shaders off',
    'Choisissez un pack de shaders. Les shaders demandent une bonne carte graphique ; en cas de lenteur, choisissez « Aucun ».':
      'Pick a shader pack. Shaders need a good graphics card; if the game is slow, choose “None”.',
    'Les shaders demandent le mod Oculus : installez « Oculus » dans Mods › Mods de la communauté, puis revenez ici.':
      'Shaders need the Oculus mod: install “Oculus” in Mods › Community mods, then come back here.',
    // Avancé
    '🛠️ Avancé': '🛠️ Advanced', 'Exécutable Java': 'Java executable', 'Automatique (Java 17 fourni)': 'Automatic (bundled Java 17)',
    'Automatique (Java 17 fourni par le launcher)': 'Automatic (Java 17 bundled with the launcher)', 'Panel': 'Panel',
    'Adresse du panel qui fournit la configuration, le modpack et les mods. Laissez vide pour l’adresse officielle.':
      'Address of the panel that provides the configuration, modpack and mods. Leave empty for the official address.',
    'Tester et utiliser': 'Test and use', 'Test du panel…': 'Testing the panel…', 'Maintenance': 'Maintenance',
    '📂 Ouvrir le dossier du jeu': '📂 Open the game folder', '🔧 Réparer l’installation': '🔧 Repair the installation',
    '⬆️ Chercher une mise à jour': '⬆️ Check for updates', 'Langue / Language': 'Langue / Language',
    'Vérifier tous les fichiers du modpack et supprimer ceux qui sont abîmés ? Ils seront re-téléchargés au prochain lancement. Vos réglages sont conservés.':
      'Check every modpack file and delete the damaged ones? They will be downloaded again on next launch. Your settings are kept.',
    'Vérification impossible pour le moment (serveur de mises à jour injoignable ou aucune version publiée).': 'Unable to check right now (update server unreachable or no version published).',
    'Mise à jour du launcher': 'Launcher update', 'Fermez d’abord le jeu.': 'Close the game first.',
    // Console, diagnostic, plantage
    'En direct': 'Live', 'Journal du jeu': 'Game log', 'Dernier plantage': 'Last crash', 'Journal affiché': 'Log shown',
    'Ouvrir le dossier des journaux': 'Open the logs folder', 'Défilement auto': 'Auto-scroll', 'Défilement figé': 'Scroll locked',
    'Copier': 'Copy', 'Effacer': 'Clear', 'Outils de développement (F12)': 'Developer tools (F12)', 'Journal copié.': 'Log copied.', 'Copie impossible.': 'Copy failed.',
    'Le jeu n’a pas encore écrit de journal.': 'The game has not written a log yet.', 'Aucun rapport de plantage. Tant mieux !': 'No crash report. Good news!',
    'Journal vide.': 'Empty log.', 'Ce rapport aide l’équipe à comprendre un problème. Les mots de passe, jetons, e-mails et chemins personnels sont masqués. Rien n’est envoyé sans votre clic.':
      'This report helps the team understand a problem. Passwords, tokens, e-mails and personal paths are hidden. Nothing is sent without your click.',
    'Analyse en cours…': 'Analysing…', 'Analyse en cours (système, installation, connexion aux serveurs)…': 'Analysing (system, installation, server connections)…',
    'Enregistrer': 'Save', 'Envoyer à l’équipe': 'Send to the team', 'Relancer l’analyse': 'Run again',
    '✅ Rapport envoyé à l’équipe. Merci !': '✅ Report sent to the team. Thank you!', 'Rapport copié : collez-le sur Discord ou dans un ticket.': 'Report copied: paste it on Discord or in a ticket.',
    'Le panel ne reçoit pas encore les rapports : utilisez « Copier » et envoyez-le sur Discord.': 'The panel does not accept reports yet: use “Copy” and send it on Discord.',
    '⚠️ Le jeu s’est fermé de façon inattendue': '⚠️ The game closed unexpectedly', 'Lancer un diagnostic': 'Run diagnostics', 'Voir le rapport': 'View the report', 'Fermer': 'Close',
    'Si le conseil ne suffit pas, le diagnostic rassemble les journaux utiles (sans données personnelles).': 'If the advice is not enough, diagnostics gathers the useful logs (no personal data).',
    'La cause n’a pas été reconnue : le diagnostic rassemble les journaux utiles (sans données personnelles).': 'The cause was not recognised: diagnostics gathers the useful logs (no personal data).',
    'Cliquez encore sur « Fermer le jeu » pour confirmer : ce qui n’est pas sauvegardé en jeu sera perdu.': 'Click “Close the game” again to confirm: anything not saved in game will be lost.',
    'Fermeture du jeu…': 'Closing the game…', 'Le jeu n’est plus ouvert.': 'The game is no longer open.',
    '🐢 Le jeu met longtemps à s’ouvrir. Patientez encore un peu ; s’il n’apparaît pas, lancez un diagnostic.': '🐢 The game is taking a long time to open. Wait a little longer; if it does not appear, run diagnostics.',
    // Lancement
    'Préparation…': 'Preparing…', 'Préparation': 'Preparing', 'Téléchargement…': 'Downloading…', 'Vérification…': 'Checking…',
    'Installation de Minecraft': 'Installing Minecraft', 'Ressources du jeu': 'Game assets', 'Installation de Forge': 'Installing Forge', 'Patch de Forge': 'Patching Forge',
    'Synchronisation du modpack': 'Syncing the modpack', 'Lancement de Minecraft': 'Launching Minecraft', 'Lancement impossible': 'Launch failed',
    '🎮 Ouverture du jeu… (le chargement des mods peut prendre une minute)': '🎮 Opening the game… (loading the mods can take a minute)',
    // Statut, panel
    'Panel en ligne': 'Panel online', 'Panel injoignable': 'Panel unreachable', 'Hors ligne': 'Offline', 'Hors ligne · dernière config connue': 'Offline · last known config',
    'Statut inconnu': 'Unknown status', 'Serveur en maintenance.': 'Server under maintenance.', 'Configuration indisponible.': 'Configuration unavailable.',
    'Configuration du serveur indisponible : réessayez dans un instant.': 'Server configuration unavailable: try again in a moment.',
    'Canal Stable': 'Stable channel', 'Canal Stable.': 'Stable channel.', 'Canal de test': 'Test channel', 'Canal Test : modpack de test, dossier séparé': 'Test channel: test modpack, separate folder',
    '🧪 Canal Test : modpack de test, installé à part.': '🧪 Test channel: test modpack, installed separately.', 'Ce serveur n’a que le canal Test.': 'This server only has the Test channel.',
    'Pas de canal Test pour ce serveur.': 'No Test channel for this server.', 'Aucune actualité disponible.': 'No news available.', 'Aucune actualité pour le moment': 'No news yet',
    'En savoir plus': 'Learn more', 'Suivez les nouveautés du serveur ici.': 'Follow the server news here.',
    'Aventure & Exploration': 'Adventure & Exploration', 'RPG & Fantaisie': 'RPG & Fantasy', 'Pokémon & Combat': 'Pokémon & Battles',
    '✨ Quoi de neuf ?': '✨ What’s new?', 'C’est parti': 'Let’s go', 'Corrections et améliorations.': 'Fixes and improvements.',
  };

  const PATTERNS = [
    [/^(.+) installé : il sera chargé au prochain lancement\.$/, '$1 installed: it will load on next launch.'],
    [/^(.+) retiré\.$/, '$1 removed.'],
    [/^Profil (.+) appliqué : (.+) Go\.$/, (m, p, r) => `${tr(p)} profile applied: ${r} GB.`],
    [/^Profil conseillé pour ce PC : (.+?)\. Le jeu n’utilisera jamais plus de 60 % de la mémoire \((.+) Go\)\.(.*)$/,
      (m, p, g, rest) => `Recommended profile for this PC: ${tr(p)}. The game will never use more than 60% of memory (${g} GB).${rest ? ' Integrated graphics: tick “lighten the game settings” for smoother play.' : ''}`],
    [/^⚡ Optimisé pour votre PC \((.+) Go, (\d+) cœurs\) : profil (.+), (.+) Go\.$/, (m, g, c, p, r) => `⚡ Optimised for your PC (${g} GB, ${c} cores): ${tr(p)} profile, ${r} GB.`],
    [/^⚡ Optimisé pour votre PC : profil (.+)\.$/, (m, p) => `⚡ Optimised for your PC: ${tr(p)} profile.`],
    [/^· profil (.+)$/, (m, p) => `· ${tr(p)} profile`],
    [/^⚠️ Seulement (.+) Go de mémoire libre pour (.+) Go demandés : .*$/, '⚠️ Only $1 GB of free memory for $2 GB requested: close other apps (browser, Discord…) before playing to avoid slowdowns.'],
    [/^Mémoire insuffisante : 3 Go utilisables minimum \((.+) Go disponibles\)\.$/, 'Not enough memory: 3 GB usable minimum ($1 GB available).'],
    [/^(\d+)–(\d+) Go$/, '$1–$2 GB'], [/^(\d+(?:[.,]\d+)?) Go$/, '$1 GB'], [/^(\d+) \/ (\d+(?:\.\d+)?) Go$/, '$1 / $2 GB'],
    [/^(.+) Go \(jeu : (.+) Go max\)$/, '$1 GB (game: $2 GB max)'], [/^(.+) · (\d+) cœurs$/, '$1 · $2 cores'],
    [/^(\d+) ?\/ ?(\d+) joueurs$/, '$1/$2 players'], [/^(\d+) joueurs?$/, '$1 player(s)'],
    [/^Heures de pointe : (.+)$/, (m, h) => `Peak hours: ${h.replace(/(\d+) h/g, '$1:00')}`],
    [/^Minecraft s’est arrêté \(code (.+?)\)\. (.*)$/, (m, c, rest) => `Minecraft stopped (code ${c}). ${tr(rest)}`],
    [/^Le lancement a échoué : (.+)$/, 'Launch failed: $1'], [/^Mise à jour impossible : (.+)$/, 'Update failed: $1'],
    [/^Installation de la version (.+)… Le launcher va redémarrer\.$/, 'Installing version $1… The launcher will restart.'],
    [/^Version (.+) disponible\.$/, 'Version $1 available.'], [/^Vous avez la dernière version \((.+)\)\.$/, 'You have the latest version ($1).'],
    [/^Fin du fichier · chemins et jeton masqués · (.+)$/, 'End of file · paths and token hidden · $1'],
    [/^(Lecture|Ouverture|Réparation|Envoi|Enregistrement|Diagnostic) impossible : (.+)$/, (m, w, e) => `${{ Lecture: 'Read', Ouverture: 'Open', Réparation: 'Repair', Envoi: 'Send', Enregistrement: 'Save', Diagnostic: 'Diagnostics' }[w]} failed: ${e}`],
    [/^Dossier impossible à ouvrir : (.+)$/, 'Unable to open folder: $1'], [/^Lien impossible à ouvrir : (.+)$/, 'Unable to open link: $1'],
    [/^Enregistré : (.+)$/, 'Saved: $1'], [/^✅ Panel utilisé : (.+)$/, '✅ Panel in use: $1'], [/^Le site ne répond pas : (.+)$/, 'The website does not respond: $1'],
    [/^Panel injoignable : (.+)$/, 'Panel unreachable: $1'], [/^Actualité indisponible : (.+)$/, 'News unavailable: $1'],
    [/^Canal Test réservé aux grades : (.+)$/, 'Test channel restricted to ranks: $1'], [/^Réservé aux grades : (.+)$/, 'Restricted to ranks: $1'],
    [/^En cours · fin dans (.+)$/, 'Live · ends in $1'], [/^Ouvre dans (.+)$/, 'Opens in $1'], [/^ · fin dans (.+)$/, ' · ends in $1'],
    [/^🏆 Succès débloqué : (.+) \(\+(\d+) pts\)$/, '🏆 Achievement unlocked: $1 (+$2 pts)'],
    [/^(\d+) fichiers vérifiés · (\d+) abîmé\(s\) supprimé\(s\) · (\d+) manquant\(s\)\. Cliquez sur Jouer pour terminer\.$/, '$1 files checked · $2 damaged removed · $3 missing. Click Play to finish.'],
    [/^Impossible de changer ce mod : (.+)$/, 'Unable to change this mod: $1'], [/^Impossible : (.+)$/, 'Failed: $1'],
    [/^(\d+) \/ (\d+) · (.*)$/, (m, a, b, rest) => `${a} / ${b} · ${rest.replace(/encore ([^·]+?)(\s*·|$)/, '$1 left$2').replace(/Mo\/s/, 'MB/s').replace(/Ko\/s/, 'KB/s')}`],
    [/^Processus (\d+)$/, 'Process $1'], [/^Actualité (\d+)$/, 'News $1'],
  ];

  const lang = (() => {
    try {
      const s = JSON.parse(localStorage.getItem('nexus_rust_settings') || '{}');
      if (s.language === 'fr' || s.language === 'en') return s.language;
    } catch (_) {}
    return /^fr\b/i.test(navigator.language || 'fr') ? 'fr' : 'en';
  })();

  function tr(text) {
    if (lang === 'fr' || typeof text !== 'string') return text;
    const trimmed = text.trim();
    if (!trimmed) return text;
    let out = EXACT[trimmed];
    if (out === undefined) {
      for (const [re, rep] of PATTERNS) {
        if (re.test(trimmed)) { out = trimmed.replace(re, rep); break; }
      }
    }
    if (out === undefined) return text;
    const lead = text.match(/^\s*/)[0];
    const trail = text.match(/\s*$/)[0];
    return lead + out + trail;
  }

  // Zones jamais traduites : journaux, rapports, contenus venus du site ou du panel.
  const SKIP = 'pre, textarea, #news-list, #news-hero .hero-body p, #notifications, #community-mods p, .whatsnew';
  const ATTRS = ['title', 'aria-label', 'placeholder'];

  function translateNode(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      const p = node.parentElement;
      if (!p || p.closest(SKIP)) return;
      const t = tr(node.nodeValue);
      if (t !== node.nodeValue) node.nodeValue = t;
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE || node.closest(SKIP)) return;
    for (const a of ATTRS) {
      const v = node.getAttribute(a);
      if (v) { const t = tr(v); if (t !== v) node.setAttribute(a, t); }
    }
    for (const c of node.childNodes) translateNode(c);
  }

  window.I18N = { lang, tr };
  document.documentElement.lang = lang;
  if (lang === 'fr') return;

  const start = () => {
    translateNode(document.body);
    new MutationObserver((records) => {
      for (const r of records) {
        if (r.type === 'characterData') translateNode(r.target);
        else if (r.type === 'attributes') translateNode(r.target);
        else r.addedNodes.forEach(translateNode);
      }
    }).observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
})();
