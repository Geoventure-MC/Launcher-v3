(() => {
  'use strict';

  // ===========================================================================
  // Pont Tauri
  // ===========================================================================
  const T = window.__TAURI__ || {};
  const invoke = T.core
    ? (cmd, args) => T.core.invoke(cmd, args)
    : async (cmd) => { throw new Error(`Pont Tauri indisponible pour ${cmd}`); };
  const $ = (id) => document.getElementById(id);
  const errMsg = (err) => (err && (err.message || err.code)) || String(err);

  // Serveurs connus (comme le launcher Electron), complétés par le panel.
  const DEFAULT_SERVERS = [
    { id: 'geoventure', name: 'Geoventure', description: 'Aventure & Exploration', theme_color: '#4ade80', is_default: true },
    { id: 'elandor', name: 'Elandor', description: 'RPG & Fantaisie', theme_color: '#a78bfa' },
    { id: 'pokeland', name: 'Pokeland', description: 'Pokémon & Combat', theme_color: '#fb923c' },
  ];
  const KNOWN_ART = ['geoventure', 'elandor', 'pokeland'];
  const LOCALE = window.I18N && window.I18N.lang === 'en' ? 'en-GB' : 'fr-FR';
  const PROFILE_NAMES = { eco: 'Économe', balanced: 'Équilibré', performance: 'Performant' };

  // ===========================================================================
  // Stockage local : préférences et compteurs des succès (propres à ce PC)
  // ===========================================================================
  const store = {
    get(key, fallback) {
      try { const raw = localStorage.getItem(key); return raw == null ? fallback : JSON.parse(raw); } catch (_) { return fallback; }
    },
    set(key, value) { try { localStorage.setItem(key, JSON.stringify(value)); } catch (_) {} },
  };
  const settings = Object.assign({
    instance: 'geoventure', channel: 'stable', profile: null, ramMin: null, ramMax: null, jvmArgs: null,
    javaPath: '', resolution: 'auto', afterLaunch: 'keep', joinServer: true, reduceMotion: false, optionalMods: {}, lightGame: false, discordRpc: true, closeToTray: false, language: null, telemetry: null, desktopNotifs: true,
  }, store.get('nexus_rust_settings', {}));
  const saveSettings = () => store.set('nexus_rust_settings', settings);
  // Dossier de jeu de l'instance choisie (arguments communs des commandes Rust).
  const gameArgs = () => ({ dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel });
  const stats = Object.assign({ launch_count: 0, playtime_minutes: 0, instances: [], first_launch: null, unlocked: [] },
    store.get('nexus_rust_stats', {}));
  const saveStats = () => store.set('nexus_rust_stats', stats);

  const state = {
    config: null, offline: false, account: null, accounts: { accounts: [], active: null }, addingAccount: false, loginSite: null, system: null, plans: {},
    launching: false, gameRunning: false, gameStartedAt: null,
    servers: DEFAULT_SERVERS.slice(), statuses: [], achievements: [], news: [], heroIndex: 0, page: 'home',
  };

  // ===========================================================================
  // Outils d'affichage — tout ce qui vient du panel ou du site est mis en TEXTE
  // ===========================================================================
  function el(tag, props, children) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (v == null || v === false) continue;
      if (k === 'class') node.className = v;
      else if (k === 'text') node.textContent = v;
      else if (k === 'dataset') Object.assign(node.dataset, v);
      else if (k === 'style') Object.assign(node.style, v);
      else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v);
    }
    for (const c of [].concat(children ?? [])) {
      if (c == null || c === false) continue;
      node.append(c instanceof Node ? c : document.createTextNode(String(c)));
    }
    return node;
  }
  const safeColor = (c) => (typeof c === 'string' && /^#[0-9a-fA-F]{6}$/.test(c) ? c : null);
  const safeHttps = (u) => { try { const x = new URL(String(u)); return x.protocol === 'https:' ? x.href : null; } catch (_) { return null; } };
  const cssUrl = (u) => `url("${String(u).replace(/["\\\n\r]/g, encodeURIComponent)}")`;
  const stripTags = (s) => String(s ?? '').replace(/<[^>]*>/g, '');
  const fmtNum = (n) => (Number.isFinite(Number(n)) ? Number(n).toLocaleString(LOCALE) : String(n ?? ''));

  let toastTimer = null;
  function toast(text) {
    const t = $('toast');
    t.classList.add('hidden');
    void t.offsetWidth;
    t.textContent = text;
    t.classList.remove('hidden');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => t.classList.add('hidden'), 5000);
  }

  function setAccent(hex) {
    const c = safeColor(hex) || '#4ade80';
    const n = parseInt(c.slice(1), 16);
    document.documentElement.style.setProperty('--accent', c);
    document.documentElement.style.setProperty('--accent-rgb', `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`);
  }

  function openUrl(url) {
    if (!url) return;
    invoke('open_url', { url }).catch((e) => toast(`Lien impossible à ouvrir : ${errMsg(e)}`));
  }

  // ===========================================================================
  // Console
  // ===========================================================================
  let autoScroll = true;
  function appendLogLine(entry) {
    const out = $('log-output');
    const lvl = String(entry.level || 'info').toLowerCase();
    const line = el('div', { class: `l-${lvl}` }, [
      `[${entry.timestamp}] `,
      el('span', { class: 'src', text: `[${entry.source}]` }),
      ` ${entry.message}`,
    ]);
    out.append(line);
    while (out.childNodes.length > 3000) out.removeChild(out.firstChild);
    if (autoScroll && state.page === 'console') out.scrollTop = out.scrollHeight;
  }
  const uiLog = (level, message) => appendLogLine({ timestamp: new Date().toTimeString().slice(0, 8), level, source: 'ui', message });

  $('btn-autoscroll').addEventListener('click', () => {
    autoScroll = !autoScroll;
    $('btn-autoscroll').setAttribute('aria-pressed', String(autoScroll));
    $('btn-autoscroll').textContent = autoScroll ? 'Défilement auto' : 'Défilement figé';
  });
  $('btn-clear-logs').addEventListener('click', async () => { $('log-output').replaceChildren(); await invoke('clear_logs').catch(() => {}); });
  $('btn-copy-logs').addEventListener('click', async () => {
    const text = logKind === 'live' ? await invoke('export_logs_text').catch(() => $('log-output').textContent) : $('log-file').textContent;
    try { await navigator.clipboard.writeText(text); toast('Journal copié.'); } catch (_) { toast('Copie impossible.'); }
  });
  $('btn-devtools').addEventListener('click', () => invoke('toggle_devtools').catch(() => {}));

  // Journaux sur le disque : jeu (latest.log), dernier rapport de plantage, launcher.log.
  let logKind = 'live';
  async function showLogKind(kind) {
    logKind = kind;
    document.querySelectorAll('#log-kinds .seg-btn').forEach((b) => {
      b.classList.toggle('active', b.dataset.kind === kind);
      b.setAttribute('aria-selected', String(b.dataset.kind === kind));
    });
    const live = kind === 'live';
    $('log-output').classList.toggle('hidden', !live);
    $('log-file').classList.toggle('hidden', live);
    ['btn-autoscroll', 'btn-clear-logs'].forEach((id) => $(id).classList.toggle('hidden', !live));
    $('log-file-path').textContent = '';
    if (live) return;
    $('log-file').textContent = 'Lecture…';
    try {
      const r = await invoke('read_game_log', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel, kind });
      if (logKind !== kind) return;
      $('log-file').textContent = r.text || ({ latest: 'Le jeu n’a pas encore écrit de journal.', crash: 'Aucun rapport de plantage. Tant mieux !', launcher: 'Journal vide.' })[kind];
      $('log-file-path').textContent = r.path ? `Fin du fichier · chemins et jeton masqués · ${r.path}` : '';
      $('log-file').scrollTop = $('log-file').scrollHeight;
    } catch (err) {
      $('log-file').textContent = `Lecture impossible : ${errMsg(err)}`;
    }
  }
  document.querySelectorAll('#log-kinds .seg-btn').forEach((b) => b.addEventListener('click', () => showLogKind(b.dataset.kind)));
  $('btn-log-folder').addEventListener('click', () => {
    const p = logKind === 'launcher' || logKind === 'live'
      ? invoke('open_launcher_logs')
      : invoke('open_game_dir', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel });
    p.catch((err) => toast(`Ouverture impossible : ${errMsg(err)}`));
  });
  window.addEventListener('keydown', (e) => {
    if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.key.toUpperCase() === 'I')) {
      e.preventDefault();
      invoke('toggle_devtools').catch(() => {});
    }
  });

  // ===========================================================================
  // Navigation
  // ===========================================================================
  function showPage(page, tab) {
    state.page = page;
    document.querySelectorAll('.nav-btn[data-page]').forEach((b) => {
      const same = b.dataset.page === page && (!b.dataset.tab || !tab || b.dataset.tab === tab);
      b.classList.toggle('active', same && (page !== 'settings' || b.dataset.tab === (tab || 'game')));
    });
    document.querySelectorAll('.page').forEach((p) => {
      const show = p.id === `page-${page}`;
      if (show && p.classList.contains('hidden')) {
        p.classList.remove('hidden');
        p.style.animation = 'none'; void p.offsetWidth; p.style.animation = '';
      } else if (!show) p.classList.add('hidden');
    });
    $('content').scrollTop = 0;
    if (page === 'profile') loadProfile();
    if (page === 'settings') { showTab(tab || 'game'); renderSettings(); }
    if (page === 'console') $('log-output').scrollTop = $('log-output').scrollHeight;
  }
  document.querySelectorAll('.nav-btn[data-page]').forEach((b) => b.addEventListener('click', () => showPage(b.dataset.page, b.dataset.tab)));
  $('player-head').addEventListener('click', () => showPage('profile'));
  $('btn-diag-nav').addEventListener('click', () => openDiagnostic());

  function showTab(tab) {
    if (tab === 'content') renderContent();
    document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('active', t.dataset.tab === tab));
    document.querySelectorAll('.tab-panel').forEach((p) => p.classList.toggle('hidden', p.dataset.panel !== tab));
    document.querySelectorAll('.nav-btn[data-page="settings"]').forEach((b) => b.classList.toggle('active', b.dataset.tab === tab));
  }
  document.querySelectorAll('.tab').forEach((t) => t.addEventListener('click', () => showTab(t.dataset.tab)));

  // ===========================================================================
  // Serveurs, canaux, grades
  // ===========================================================================
  const currentServer = () => state.servers.find((s) => s.id === settings.instance) || state.servers[0];
  const roleName = () => (state.account && state.account.role && state.account.role.name) || '';
  function roleIn(list) {
    if (!Array.isArray(list) || list.length === 0) return true;
    const r = roleName().toLowerCase();
    return !!r && list.some((x) => String(x).toLowerCase() === r);
  }
  function channelAccess(channel) {
    const srv = currentServer() || {};
    const cfg = state.config || {};
    const mode = srv.channel || 'both';
    if (channel === 'test') {
      if (mode === 'stable') return { ok: false, why: 'Pas de canal Test pour ce serveur.' };
      const roles = srv.test_roles || cfg.test_roles || [];
      return roleIn(roles) ? { ok: true } : { ok: false, why: `Canal Test réservé aux grades : ${roles.join(', ')}` };
    }
    if (mode === 'test') return { ok: false, why: 'Ce serveur n’a que le canal Test.' };
    const roles = srv.allowed_roles || cfg.allowed_roles || [];
    return roleIn(roles) ? { ok: true } : { ok: false, why: `Réservé aux grades : ${roles.join(', ')}` };
  }

  function artFor(id, kind) {
    const known = KNOWN_ART.includes(id) ? id : 'geoventure';
    if (kind === 'bg') return `assets/img/bg_${known}.jpg`;
    if (kind === 'logo') return KNOWN_ART.includes(id) ? `assets/img/${id}_logo.png` : null;
    return `assets/img/${known}.jpg`;
  }

  let lastBg = null;
  // Logo d'un serveur : celui du panel s'il y en a un (l'équipe peut le changer sans nouvelle version),
  // sinon celui fourni avec le launcher.
  function logoFor(srv) {
    return (srv && safeHttps(srv.icon)) || artFor(srv && srv.id, 'logo');
  }
  // Fond d'écran du grade du joueur (page « Fonds » du panel, role_data), sinon celui du serveur.
  function roleBackground() {
    const role = state.account && state.account.role && state.account.role.name;
    const data = state.config && state.config.role_data;
    if (!role || !data || typeof data !== 'object') return null;
    const hit = Object.values(data).find((r) => r && String(r.name || '').toLowerCase() === String(role).toLowerCase());
    return hit ? safeHttps(hit.background) : null;
  }
  function renderBackground() {
    const src = roleBackground() || artFor(settings.instance, 'bg');
    if (src === lastBg) return;
    lastBg = src;
    const bg = $('backdrop-img');
    bg.classList.add('swap');
    const img = new Image();
    img.onload = () => { bg.style.backgroundImage = cssUrl(src); bg.classList.remove('swap'); };
    img.onerror = () => bg.classList.remove('swap');
    img.src = src;
  }

  function renderServers() {
    const box = $('server-pills');
    box.replaceChildren();
    for (const srv of state.servers) {
      const status = state.statuses.find((s) => s.id === srv.id);
      const logo = logoFor(srv);
      const color = safeColor(srv.theme_color || srv.color) || '#4ade80';
      const tip = status && status.online ? `${srv.name} · ${status.players ?? 0} en ligne` : `${srv.name}${status ? ' · hors ligne' : ''}`;
      const btn = el('button', {
        class: `server-pill${srv.id === settings.instance ? ' active' : ''}`,
        role: 'tab', 'aria-selected': String(srv.id === settings.instance), 'aria-label': tip,
        style: { '--c': color },
        onclick: () => selectInstance(srv.id),
      }, [
        logo ? el('img', { src: logo, alt: '' }) : el('span', { class: 'initial', text: (srv.name || srv.id).slice(0, 1).toUpperCase() }),
        el('span', { class: `online-dot${status && status.online ? ' on' : ''}` }),
        el('span', { class: 'tip', text: tip }),
      ]);
      btn.style.setProperty('--c', color);
      box.append(btn);
    }
    const srv = currentServer() || {};
    setAccent(srv.theme_color || srv.color);
    $('server-name').textContent = srv.name || 'Geoventure';
    $('server-card-name').textContent = srv.name || 'Geoventure';
    $('server-card-desc').textContent = srv.description || '';
    const logo = logoFor(srv);
    $('server-logo').src = logo || 'assets/img/icon.png';
    $('server-img').src = artFor(srv.id, 'bg');
    renderBackground();
  }

  function renderChannel() {
    const testAccess = channelAccess('test');
    if (settings.channel === 'test' && !testAccess.ok) { settings.channel = 'stable'; saveSettings(); }
    document.querySelectorAll('#channel-switch .channel-pill').forEach((b) => {
      const active = b.dataset.channel === settings.channel;
      b.classList.toggle('active', active);
      b.classList.toggle('test', b.dataset.channel === 'test');
      b.setAttribute('aria-checked', String(active));
      if (b.dataset.channel === 'test') {
        b.disabled = !testAccess.ok;
        b.title = testAccess.ok ? 'Canal Test : modpack de test, dossier séparé' : testAccess.why;
      } else b.title = 'Canal Stable';
    });
  }
  document.querySelectorAll('#channel-switch .channel-pill').forEach((b) => b.addEventListener('click', () => {
    if (state.launching || state.gameRunning || b.disabled || settings.channel === b.dataset.channel) return;
    settings.channel = b.dataset.channel;
    saveSettings();
    toast(settings.channel === 'test' ? '🧪 Canal Test : modpack de test, installé à part.' : 'Canal Stable.');
    loadConfig();
  }));

  function selectInstance(id) {
    if (state.launching || state.gameRunning || id === settings.instance) return;
    settings.instance = id;
    saveSettings();
    renderServers();
    renderStatus();
    loadConfig().then(() => { loadNotifications(); loadLauncherContent(); loadAffluence(); renderVideo(); loadWonder(); updatePresence('launcher'); });
  }

  // ===========================================================================
  // Configuration du panel
  // ===========================================================================
  function setBadge(text, cls) { const b = $('status-badge'); b.textContent = text; b.className = `badge ${cls || ''}`; }

  async function loadConfig() {
    setBadge('Chargement…');
    renderChannel();
    try {
      const res = await invoke('fetch_panel_config', { instance: settings.instance, channel: settings.channel });
      state.config = res.config;
      state.offline = !!res.offline;
      mergeServers(res.config.servers);
      const l = res.config.loader;
      $('info-version').textContent = `Minecraft ${res.config.game_version || '1.20.1'}${l && l.enable && l.build ? ` · Forge ${String(l.build).replace(/^1\.20\.1-/, '')}` : ''}`;
      $('splash').textContent = res.config.splash
        ? `« ${stripTags(res.config.splash)} »${res.config.splash_author ? ` — ${stripTags(res.config.splash_author)}` : ''}`
        : '';
      if (res.offline) setBadge('Hors ligne · dernière config connue', 'warn');
      else if (res.config.maintenance) setBadge('Maintenance', 'warn');
      else setBadge('Panel en ligne', 'ok');
      updateLoginSite();
    } catch (err) {
      state.config = null;
      setBadge('Panel injoignable', 'err');
      uiLog('error', `Panel injoignable : ${errMsg(err)}`);
    }
    renderServers();
    renderChannel();
    renderAlert();
    if (state.account) renderAccount(state.account);
    updatePlayButton();
  }

  function mergeServers(panelServers) {
    if (!Array.isArray(panelServers) || panelServers.length === 0) return;
    const merged = DEFAULT_SERVERS.map((d) => Object.assign({}, d, panelServers.find((p) => p.id === d.id) || {}));
    for (const p of panelServers) if (p && p.id && !merged.some((m) => m.id === p.id)) merged.push(p);
    state.servers = merged;
  }

  function renderAlert() {
    const cfg = state.config || {};
    const alert = $('alert-banner');
    if (cfg.alert_activate && cfg.alert_msg) { alert.textContent = stripTags(cfg.alert_msg); alert.classList.remove('hidden'); } else alert.classList.add('hidden');
    const m = $('maintenance-box');
    if (cfg.maintenance) { m.textContent = `🛠️ ${stripTags(cfg.maintenance_message || 'Maintenance en cours.')}`; m.classList.remove('hidden'); } else m.classList.add('hidden');
  }

  function playBlockReason() {
    const cfg = state.config;
    if (!state.account) return 'Connectez-vous pour jouer.';
    if (!cfg) return 'Configuration du serveur indisponible : réessayez dans un instant.';
    if (cfg.maintenance) return 'Serveur en maintenance.';
    if (cfg.email_verified && !state.account.email_verified) return 'Vérifiez votre adresse e-mail sur le site pour jouer.';
    if (cfg.whitelist_activate) {
      const names = (cfg.whitelist || []).map((n) => String(n).toLowerCase());
      const roles = cfg.whitelist_roles || [];
      if (!names.includes(String(state.account.username).toLowerCase()) && !(roles.length && roleIn(roles))) return 'Accès réservé (liste blanche).';
    }
    const access = channelAccess(settings.channel);
    if (!access.ok) return access.why;
    if (state.system && state.system.plan && state.system.plan.insufficient) return `Mémoire insuffisante : 3 Go utilisables minimum (${state.system.plan.cap_gb} Go disponibles).`;
    return '';
  }

  function updatePlayButton() {
    const busy = state.launching || state.gameRunning;
    const reason = busy ? '' : playBlockReason();
    const btn = $('btn-play');
    btn.disabled = busy || !!reason;
    btn.classList.toggle('busy', state.launching);
    $('play-label').textContent = state.launching ? 'Lancement…' : (state.gameRunning ? 'En jeu' : 'Jouer');
    btn.title = reason || (state.gameRunning ? 'Le jeu est ouvert' : 'Jouer');
    // Jeu ouvert : le bouton du serveur devient « Fermer le jeu ».
    $('btn-join').disabled = state.launching || (!state.gameRunning && !!reason);
    $('btn-join').textContent = state.gameRunning ? '✕ Fermer le jeu' : (state.launching ? 'Lancement…' : 'Rejoindre le serveur');
    $('btn-join').classList.toggle('btn-danger', state.gameRunning);
    $('btn-kill').classList.toggle('hidden', !state.gameRunning);
    $('play-block-reason').textContent = reason;
    document.querySelectorAll('.account-lock, #btn-add-account').forEach((b) => { b.disabled = busy; });
  }

  // ===========================================================================
  // Statut des serveurs, annonces, événements
  // ===========================================================================
  async function loadStatuses() {
    try {
      const list = await invoke('panel_get', { endpoint: 'utils/servers-status', instance: null, channel: settings.channel });
      state.statuses = Array.isArray(list) ? list : [];
    } catch (_) { state.statuses = []; }
    renderServers();
    renderStatus();
  }

  function renderStatus() {
    const st = state.statuses.find((s) => s.id === settings.instance);
    const dot = $('status-dot');
    dot.className = `status-dot${st ? (st.online ? ' on' : ' off') : ''}`;
    $('server-players').textContent = !st ? 'Statut inconnu' : (st.online ? `${st.players ?? 0}/${st.max_players ?? '?'} joueurs` : 'Hors ligne');
    const lat = $('server-latency');
    lat.textContent = st && st.online && st.latency != null ? `${st.latency} ms` : '-- ms';
    lat.className = `chip${st && st.online && st.latency != null ? (st.latency < 80 ? ' good' : st.latency < 180 ? ' mid' : ' bad') : ''}`;
  }
  $('btn-refresh-status').addEventListener('click', () => loadStatuses());

  async function loadNotifications() {
    const box = $('notifications');
    let list = [];
    try { list = await invoke('panel_get', { endpoint: 'utils/notifications', instance: settings.instance, channel: settings.channel }); } catch (_) {}
    notifyNewAnnouncements(list);
    box.replaceChildren();
    const dismissed = store.get('nexus_rust_dismissed', []);
    const now = Date.now();
    for (const n of Array.isArray(list) ? list : []) {
      if (!n || dismissed.includes(n.id) || (n.expiresAt && Number(n.expiresAt) < now)) continue;
      const type = ['info', 'warning', 'maintenance', 'event'].includes(n.type) ? n.type : 'info';
      const item = el('div', { class: `notif ${type}` }, [
        el('span', { text: { info: 'ℹ️', warning: '⚠️', maintenance: '🛠️', event: '🎉' }[type] }),
        el('span', { class: 'msg', text: stripTags(n.message) }),
      ]);
      const url = n.url && /^https?:\/\//i.test(n.url) ? n.url : null;
      if (url) item.append(el('button', { class: 'btn', text: 'En savoir plus', onclick: () => openUrl(url) }));
      item.append(el('button', {
        class: 'link', text: 'Masquer',
        onclick: () => { dismissed.push(n.id); store.set('nexus_rust_dismissed', dismissed.slice(-200)); item.remove(); },
      }));
      box.append(item);
    }
  }

  let gameEvents = [];
  async function loadGameEvents() {
    try {
      const list = await invoke('panel_get', { endpoint: 'utils/game-events', instance: null, channel: 'stable' });
      gameEvents = Array.isArray(list) ? list : [];
    } catch (_) { gameEvents = []; }
    renderGameEvents();
  }
  function formatDuration(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
    if (d > 0) return `${d} j ${h} h`;
    if (h > 0) return `${h} h ${m} min`;
    return `${m} min ${s % 60} s`;
  }
  function renderGameEvents() {
    const box = $('game-events');
    const now = Date.now();
    const items = gameEvents.filter((e) => e && e.enabled !== false).map((e) => {
      let phase = e.phase;
      if (phase === 'upcoming' && e.opensAt && e.opensAt <= now) phase = 'open';
      if (e.endsAt && e.endsAt <= now) phase = 'ended';
      return Object.assign({}, e, { phase });
    }).filter((e) => e.phase === 'open' || e.phase === 'upcoming')
      .sort((a, b) => (a.phase === b.phase ? (a.opensAt || 0) - (b.opensAt || 0) : a.phase === 'open' ? -1 : 1)).slice(0, 2);
    if (items.length === 0) {
      if (!box.querySelector('.empty')) box.replaceChildren(el('p', { class: 'muted small empty', text: 'Aucun événement pour le moment.' }));
      return;
    }
    box.replaceChildren(...items.map((e) => el('div', { class: 'event', dataset: { slug: String(e.slug || '') } }, [
      el('strong', { text: `${e.phase === 'open' ? '🔥 ' : '⏳ '}${e.name || e.slug}` }),
      el('span', { class: 'count', text: e.phase === 'open' ? (e.endsAt ? `En cours · fin dans ${formatDuration(e.endsAt - now)}` : 'En cours') : `Ouvre dans ${formatDuration((e.opensAt || now) - now)}` }),
    ])));
  }

  // ===========================================================================
  // Actualité (flux RSS du site Azuriom) et contenu de l'accueil (panel)
  // ===========================================================================
  let heroTimer = null;
  async function loadNews() {
    try {
      state.news = await invoke('fetch_news', { siteUrl: siteUrl() });
    } catch (err) {
      state.news = [];
      uiLog('warn', `Actualité indisponible : ${errMsg(err)}`);
    }
    state.heroIndex = 0;
    renderHero(true);
    renderNewsList();
    clearInterval(heroTimer);
    if (state.news.length > 1) heroTimer = setInterval(() => { if (state.page === 'home') nextHero(1); }, 8000);
  }

  function formatDate(ts) {
    if (!ts) return '';
    try { return new Date(ts).toLocaleDateString(LOCALE, { day: 'numeric', month: 'long', year: 'numeric' }); } catch (_) { return ''; }
  }

  function nextHero(step) {
    const n = Math.min(state.news.length, 5);
    if (n < 2) return;
    state.heroIndex = (state.heroIndex + step + n) % n;
    renderHero();
  }

  function renderHero(immediate) {
    const item = state.news[state.heroIndex];
    const body = document.querySelector('.hero-body');
    const bg = $('news-hero-bg');
    const apply = () => {
      if (!item) {
        $('news-hero-title').textContent = 'Aucune actualité pour le moment';
        $('news-hero-desc').textContent = 'Suivez les nouveautés du serveur ici.';
        $('news-hero-meta').textContent = '';
        bg.style.backgroundImage = cssUrl(artFor(settings.instance, 'bg'));
      } else {
        $('news-hero-title').textContent = item.title;
        $('news-hero-desc').textContent = item.summary || '';
        $('news-hero-meta').textContent = [formatDate(item.timestamp), item.author].filter(Boolean).join(' · ');
        const img = safeHttps(item.image);
        bg.style.backgroundImage = img ? cssUrl(img) : cssUrl(artFor(settings.instance, 'bg'));
      }
      body.classList.remove('fade');
      bg.classList.remove('fade');
    };
    if (immediate) apply();
    else { body.classList.add('fade'); bg.classList.add('fade'); setTimeout(apply, 320); }

    const nav = $('news-hero-nav');
    const n = Math.min(state.news.length, 5);
    nav.replaceChildren();
    if (n > 1) {
      for (let i = 0; i < n; i += 1) {
        nav.append(el('button', {
          class: `hero-dot${i === state.heroIndex ? ' active' : ''}`, 'aria-label': `Actualité ${i + 1}`,
          onclick: (e) => { e.stopPropagation(); state.heroIndex = i; renderHero(); },
        }));
      }
    }
  }
  $('news-hero').addEventListener('click', () => { const it = state.news[state.heroIndex]; if (it && it.link) openUrl(it.link); });
  $('news-hero').addEventListener('keydown', (e) => {
    if (e.key === 'ArrowRight') nextHero(1);
    if (e.key === 'ArrowLeft') nextHero(-1);
    if (e.key === 'Enter') { const it = state.news[state.heroIndex]; if (it && it.link) openUrl(it.link); }
  });

  function renderNewsList() {
    const box = $('news-list');
    box.replaceChildren();
    const items = state.news.slice(0, 6);
    if (items.length === 0) { box.append(el('p', { class: 'muted small', text: 'Aucune actualité disponible.' })); return; }
    for (const it of items) {
      const img = safeHttps(it.image);
      box.append(el('button', { class: 'news-item', onclick: () => openUrl(it.link) }, [
        el('div', { class: 'news-thumb', style: img ? { backgroundImage: cssUrl(img) } : null }),
        el('div', null, [
          el('strong', { text: it.title }),
          el('p', { text: it.summary || '' }),
          el('span', { class: 'date', text: formatDate(it.timestamp) }),
        ]),
      ]));
    }
  }

  async function loadLauncherContent() {
    let data = null;
    try { data = await invoke('panel_get', { endpoint: 'utils/launcher-content', instance: settings.instance, channel: 'stable' }); } catch (_) {}
    const shortcuts = $('shortcuts');
    shortcuts.replaceChildren();
    const sc = (data && Array.isArray(data.shortcuts) ? data.shortcuts : []).slice().sort((a, b) => (a.sortOrder || 0) - (b.sortOrder || 0));
    const mapUrl = state.config && safeHttps(state.config.map_url);
    if (mapUrl && !sc.some((s) => /carte|map/i.test(String(s.title)) || safeHttps(s.url) === mapUrl)) sc.push({ title: 'Carte', icon: '🗺️', url: mapUrl });
    for (const s of sc) {
      const url = safeHttps(s.url);
      if (!url) continue;
      shortcuts.append(el('button', { class: 'shortcut', title: s.description || s.title, onclick: () => openUrl(url) }, [
        el('span', { class: 'shortcut-icon', text: s.icon && [...String(s.icon)].length <= 3 ? s.icon : '🔗' }),
        el('strong', { text: s.title }),
      ]));
    }
    shortcuts.parentElement.classList.toggle('hidden', shortcuts.childElementCount === 0);

    const disc = $('discover');
    disc.replaceChildren();
    const dc = (data && Array.isArray(data.discover) ? data.discover : []).slice().sort((a, b) => (a.sortOrder || 0) - (b.sortOrder || 0));
    for (const d of dc) {
      const url = safeHttps(d.url);
      const img = safeHttps(d.imageUrl);
      disc.append(el('button', { class: 'discover-card', onclick: () => openUrl(url) }, [
        el('div', { class: 'discover-bg', style: img ? { backgroundImage: cssUrl(img) } : { backgroundImage: cssUrl(artFor(settings.instance, 'card')) } }),
        el('div', { class: 'discover-body' }, [el('strong', { text: d.title }), el('span', { text: d.description || '' })]),
      ]));
    }
    disc.classList.toggle('hidden', disc.childElementCount === 0);
    disc.previousElementSibling.classList.toggle('hidden', disc.childElementCount === 0);
  }

  // ===========================================================================
  // Compte Azuriom (connexion via le site)
  // ===========================================================================
  function siteUrl() {
    const cfg = state.config || {};
    const srv = currentServer() || {};
    const raw = srv.azauth || cfg.azauth || 'https://geoventure.fr/';
    return /\/$/.test(raw) ? raw : `${raw}/`;
  }

  const AUTH_REASONS = {
    invalid_credentials: 'E-mail, pseudo ou mot de passe incorrect.',
    invalid_2fa: 'Code de double authentification incorrect.',
    invalid_code: 'Code de double authentification incorrect.',
    banned: 'Ce compte est banni du serveur.',
    email_not_verified: 'Vérifiez d’abord votre adresse e-mail sur le site.',
    user_banned: 'Ce compte est banni du serveur.',
  };

  function renderAccount(account) {
    state.account = account;
    if (!account) state.addingAccount = false;
    $('login-screen').classList.toggle('hidden', !!account && !state.addingAccount);
    $('btn-login-cancel').classList.toggle('hidden', !account);
    if (account) {
      const avatar = safeHttps(account.skin_url) || 'assets/img/icon.png';
      $('az-avatar').src = avatar;
      $('profile-avatar').src = avatar;
      $('az-avatar').onerror = () => { $('az-avatar').src = 'assets/img/icon.png'; };
      $('profile-avatar').onerror = () => { $('profile-avatar').src = 'assets/img/icon.png'; };
      $('az-role').textContent = account.role ? account.role.name : 'Joueur';
      const rc = account.role && safeColor(account.role.color);
      $('az-role').style.color = rc || '';
      $('profile-name').textContent = account.username;
      // Réglages « Afficher le grade » et « Afficher l'argent » du panel (page Général).
      const cfg = state.config || {};
      $('az-role').classList.toggle('hidden', cfg.role === false);
      $('profile-role').classList.toggle('hidden', cfg.role === false);
      $('stat-money-wrap').classList.toggle('hidden', cfg.money === false || !Number.isFinite(Number(account.money)));
      $('stat-money').textContent = fmtNum(Math.floor(Number(account.money) || 0));
      $('profile-role').textContent = account.role ? account.role.name : 'Joueur';
      $('az-status').textContent = '';
    } else {
      $('az-2fa-wrap').classList.add('hidden');
      setTimeout(() => $('az-email').focus(), 50);
    }
    renderChannel();
    renderBackground();
    updatePlayButton();
  }

  $('az-login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const status = $('az-status');
    const btn = $('btn-login');
    status.textContent = '';
    btn.disabled = true;
    btn.textContent = 'Connexion…';
    try {
      const res = await invoke('azuriom_login', {
        siteUrl: state.loginSite || siteUrl(), email: $('az-email').value.trim(), password: $('az-password').value, code2fa: $('az-2fa').value.trim() || null,
      });
      if (res.outcome === 'requires2fa') {
        $('az-2fa-wrap').classList.remove('hidden');
        status.textContent = 'Entrez le code de votre application de double authentification.';
        $('az-2fa').focus();
      } else if (res.outcome === 'authenticated') {
        $('az-password').value = '';
        $('az-2fa').value = '';
        $('az-2fa-wrap').classList.add('hidden');
        state.addingAccount = false;
        state.loginSite = null;
        await refreshAccounts();
        renderAccount(res.account);
        toast(`Bienvenue, ${res.account.username} !`);
        loadConfig();
      } else {
        status.textContent = AUTH_REASONS[res.reason] || res.message || 'Connexion refusée.';
        shakeLogin();
      }
    } catch (err) {
      status.textContent = `Le site ne répond pas : ${errMsg(err)}`;
      shakeLogin();
    } finally {
      btn.disabled = false;
      btn.textContent = 'Se connecter';
    }
  });
  function shakeLogin() { const c = $('az-login-form'); c.classList.remove('shake'); void c.offsetWidth; c.classList.add('shake'); }
  $('link-register').addEventListener('click', () => openUrl(`${siteUrl()}user/register`));
  $('link-forgot').addEventListener('click', () => openUrl(`${siteUrl()}user/password/reset`));
  // Déconnecte le compte ACTIF seulement. Les autres restent : aucun n'est activé d'office,
  // l'écran de connexion propose la liste.
  $('btn-logout').addEventListener('click', async () => {
    if (state.launching || state.gameRunning) return toast('Fermez d’abord le jeu.');
    try {
      applyAccountList(await invoke('azuriom_logout'));
    } catch (err) { return toast(errMsg(err)); }
    renderAccount(null);
    showPage('home');
  });

  // ---- Plusieurs comptes -------------------------------------------------------------------
  const MAX_ACCOUNTS = 10; // même plafond que le backend (accounts.rs)
  const siteHost = (u) => String(u || '').replace(/^https?:\/\//, '').replace(/\/$/, '');
  function updateLoginSite() { $('login-site').textContent = siteHost(state.loginSite || siteUrl()); }

  function applyAccountList(list) {
    state.accounts = list && Array.isArray(list.accounts) ? list : { accounts: [], active: null };
    renderAccountList();
    renderLoginAccounts();
    $('btn-add-account').title = state.accounts.accounts.length >= MAX_ACCOUNTS ? `Limite de ${MAX_ACCOUNTS} comptes atteinte : retirez-en un avant d’en ajouter.` : '';
  }
  async function refreshAccounts() { applyAccountList(await invoke('azuriom_list_accounts').catch(() => null)); }

  function accountRow(acc, manage) {
    const several = new Set(state.accounts.accounts.map((a) => siteHost(a.site_url))).size > 1;
    const img = el('img', { class: 'account-avatar', alt: '', src: safeHttps(acc.skin_url) || 'assets/img/icon.png' });
    img.onerror = () => { img.src = 'assets/img/icon.png'; };
    const info = el('div', { class: 'account-info' }, [el('strong', { text: acc.username }), several ? el('span', { class: 'muted small', text: siteHost(acc.site_url) }) : null]);
    const badges = [acc.active ? el('span', { class: 'badge', text: 'Actif' }) : null, acc.needs_login ? el('span', { class: 'badge warn-badge', text: 'Reconnexion requise' }) : null];
    const row = el('li', { class: `account${acc.active ? ' active' : ''}${acc.needs_login ? ' stale' : ''}` }, [img, info, ...badges]);
    if (manage) {
      const actions = el('div', { class: 'account-actions' });
      if (!acc.active) actions.append(el('button', { class: 'btn btn-small account-lock', text: acc.needs_login ? 'Se reconnecter' : 'Utiliser', onclick: () => useAccount(acc.key) }));
      const rm = confirmButton('Retirer', () => removeAccount(acc.key));
      if (acc.active) rm.classList.add('account-lock');
      actions.append(rm);
      row.append(actions);
    } else {
      row.classList.add('pickable');
      row.tabIndex = 0;
      row.addEventListener('click', () => useAccount(acc.key));
      row.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); useAccount(acc.key); } });
    }
    return row;
  }
  function renderAccountList() {
    $('account-list').replaceChildren(...state.accounts.accounts.map((a) => accountRow(a, true)));
    updatePlayButton();
  }
  function renderLoginAccounts() {
    const list = state.accounts.accounts;
    $('login-accounts').classList.toggle('hidden', list.length === 0);
    $('login-account-list').replaceChildren(...list.map((a) => accountRow(a, false)));
  }

  // Ouvre l'écran de connexion (compte à ajouter, ou à reconnecter si `acc` est donné).
  function showLogin(acc) {
    state.addingAccount = true;
    state.loginSite = acc ? acc.site_url : null;
    $('az-email').value = acc ? acc.username : '';
    $('az-password').value = '';
    $('az-2fa').value = '';
    $('az-2fa-wrap').classList.add('hidden');
    renderAccount(state.account);
    $('az-status').textContent = acc ? `Reconnexion requise pour ${acc.username} : entrez son mot de passe.` : '';
    updateLoginSite();
    setTimeout(() => $(acc ? 'az-password' : 'az-email').focus(), 80);
  }
  $('btn-add-account').addEventListener('click', () => {
    if (state.launching || state.gameRunning) return toast('Fermez d’abord le jeu avant de changer de compte.');
    showLogin(null);
  });
  $('btn-login-cancel').addEventListener('click', () => {
    state.addingAccount = false;
    state.loginSite = null;
    updateLoginSite();
    renderAccount(state.account);
  });

  async function useAccount(key) {
    if (state.launching || state.gameRunning) return toast('Fermez d’abord le jeu avant de changer de compte.');
    const acc = state.accounts.accounts.find((a) => a.key === key);
    if (!acc) return;
    if (acc.active && state.account) { state.addingAccount = false; return renderAccount(state.account); }
    if (acc.needs_login) return showLogin(acc);
    try {
      const res = await invoke('azuriom_switch_account', { key });
      applyAccountList(res.accounts);
      if (res.reconnect_required) {
        const stale = res.accounts.accounts.find((a) => a.key === key);
        toast('Session expirée : reconnexion requise pour ce compte.');
        // Le compte actif (s'il y en a un) ne change pas.
        return showLogin(stale || acc);
      }
      state.addingAccount = false;
      state.loginSite = null;
      renderAccount(res.account);
      toast(`Compte actif : ${res.account.username}`);
      loadConfig();
    } catch (err) { toast(errMsg(err)); }
  }

  async function removeAccount(key) {
    try {
      const wasActive = state.accounts.active === key;
      applyAccountList(await invoke('azuriom_remove_account', { key }));
      if (wasActive) { renderAccount(null); showPage('home'); }
    } catch (err) { toast(errMsg(err)); }
  }

  // ===========================================================================
  // Lancement
  // ===========================================================================
  const PHASES = { minecraft: 'Installation de Minecraft', assets: 'Ressources du jeu', java: 'Java 17', forge: 'Installation de Forge', 'forge-patch': 'Patch de Forge', modpack: 'Synchronisation du modpack', launch: 'Lancement de Minecraft' };

  function showDock(show) { $('dock').classList.toggle('hidden', !show); }
  function onInstallProgress(p) {
    showDock(true);
    const fill = $('progress-fill');
    const pct = p.total > 0 ? Math.round((p.completed / p.total) * 100) : null;
    fill.classList.toggle('indeterminate', pct === null);
    if (pct !== null) fill.style.width = `${pct}%`;
    $('dock-phase').textContent = p.phase === 'forge-patch' ? p.label : (PHASES[p.phase] || p.label);
    $('dock-pct').textContent = pct !== null ? `${pct} %` : '';
    const file = p.current_file && p.phase !== 'forge-patch' ? p.current_file.split('/').pop() : '';
    const rate = progressRate(p);
    $('progress-text').textContent = p.total > 0
      ? `${p.completed} / ${p.total}${rate ? ` · ${rate}` : ''}${file ? ` · ${file}` : ''}`
      : 'Vérification…';
  }

  // Vitesse (moyenne glissante sur ~3 s) et temps restant estimé par étape.
  const rateState = { phase: '', start: 0, samples: [] };
  function progressRate(p) {
    const now = Date.now();
    if (p.phase !== rateState.phase || p.completed === 0) {
      Object.assign(rateState, { phase: p.phase, start: now, samples: [] });
      return '';
    }
    rateState.samples.push([now, p.bytes || 0]);
    while (rateState.samples.length > 2 && now - rateState.samples[0][0] > 3000) rateState.samples.shift();
    const parts = [];
    const [t0, b0] = rateState.samples[0];
    if (now - t0 > 500 && p.bytes > b0) parts.push(`${formatSpeed(((p.bytes - b0) * 1000) / (now - t0))}`);
    const elapsed = now - rateState.start;
    if (elapsed > 3000 && p.total > p.completed) {
      const left = (elapsed / p.completed) * (p.total - p.completed);
      parts.push(`encore ${formatLeft(left)}`);
    }
    return parts.join(' · ');
  }
  function formatSpeed(bps) {
    if (bps >= 1048576) return `${(bps / 1048576).toFixed(1).replace('.', ',')} Mo/s`;
    return `${Math.max(1, Math.round(bps / 1024))} Ko/s`;
  }
  function formatLeft(ms) {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${Math.max(1, s)} s`;
    const m = Math.round(s / 60);
    return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
  }

  function launchRam() {
    const plan = (state.system && state.system.plan) || {};
    const cfg = state.config || {};
    const max = settings.ramMax || plan.ram_max_gb || cfg.ram_max || 4;
    return { min: Math.min(settings.ramMin || plan.ram_min_gb || cfg.ram_min || 2, max), max };
  }
  function launchJvmArgs() {
    if (Array.isArray(settings.jvmArgs)) return settings.jvmArgs;
    return ((state.system && state.system.plan) || {}).jvm_args || [];
  }

  async function applyOptionalModDefaults() {
    let data;
    try { data = await invoke('panel_get', { endpoint: 'utils/mods', instance: settings.instance, channel: settings.channel }); } catch (_) { return; }
    const list = (data && data.optionalMods) || [];
    const meta = (data && data.mods) || {};
    const key = `${settings.instance}:${settings.channel}`;
    const saved = settings.optionalMods[key] || {};
    for (const file of list) {
      if (Object.prototype.hasOwnProperty.call(saved, file)) continue;
      const recommended = !!(meta[file] && meta[file].recommanded);
      if (!recommended) {
        await invoke('set_optional_mod', { dataDirectory: state.config.dataDirectory, instance: settings.instance, channel: settings.channel, file, enabled: false }).catch(() => {});
      }
      saved[file] = recommended;
    }
    settings.optionalMods[key] = saved;
    saveSettings();
  }

  let pendingLaunch = null;
  function onLaunchResult(r) {
    if (!pendingLaunch) return;
    if (r && r.ok) pendingLaunch.resolve(r.pid);
    else pendingLaunch.reject((r && r.error) || new Error('Lancement impossible'));
  }

  async function play(joinServer) {
    if (state.launching || state.gameRunning || playBlockReason()) return;
    const launchStartedAt = Date.now();
    const cfg = state.config;
    const srv = currentServer() || {};
    state.launching = true;
    updatePlayButton();
    onInstallProgress({ phase: 'minecraft', label: 'Préparation', completed: 0, total: 0, current_file: '' });
    $('dock-phase').textContent = 'Préparation…';
    try {
      await applyOptionalModDefaults();
      const ram = launchRam();
      const isTest = settings.channel === 'test';
      const host = isTest ? (srv.test_ip || srv.ip) : (srv.ip || (cfg.status && cfg.status.ip));
      const port = Number.parseInt(isTest ? (srv.test_port || srv.port) : (srv.port || (cfg.status && cfg.status.port)), 10);
      let width = null, height = null;
      if (settings.resolution && settings.resolution !== 'auto') [width, height] = settings.resolution.split('x').map((n) => Number.parseInt(n, 10));
      const ignored = Array.isArray(cfg.ignored) ? cfg.ignored : Object.values(cfg.ignored || {});
      const result = new Promise((resolve, reject) => { pendingLaunch = { resolve, reject }; });
      await invoke('launch_instance', {
        req: {
          data_directory: cfg.dataDirectory || 'Nexus', instance: settings.instance, channel: settings.channel,
          mc_version: cfg.game_version || '1.20.1', loader_build: (cfg.loader && cfg.loader.build) || '1.20.1-47.4.28',
          loader_type: (cfg.loader && cfg.loader.type) || 'forge', loader_enable: cfg.loader ? cfg.loader.enable !== false : true,
          custom_java_path: settings.javaPath || null,
          username: state.account.username, uuid: state.account.uuid, access_token: state.account.access_token,
          ram_min_gb: ram.min, ram_max_gb: ram.max, extra_jvm_args: launchJvmArgs(), ignored: ignored.map(String),
          server_host: joinServer && host ? String(host) : null, server_port: joinServer && Number.isFinite(port) ? port : null,
          width, height, after_launch: settings.afterLaunch,
          light_game_options: !!settings.lightGame && activeProfile() === 'eco',
        },
      });
      const pid = await result;
      state.gameRunning = true;
      state.presenceStart = Date.now();
      updatePresence('game');
      track('game_launch', { prepare_s: Math.round((Date.now() - launchStartedAt) / 1000) });
      $('dock-phase').textContent = '🎮 Ouverture du jeu… (le chargement des mods peut prendre une minute)';
      $('dock-pct').textContent = '';
      $('progress-fill').classList.remove('indeterminate');
      $('progress-fill').style.width = '100%';
      $('progress-text').textContent = `Processus ${pid}`;
      setTimeout(() => showDock(false), 6000);
      recordLaunch();
    } catch (err) {
      showDock(false);
      toast(`Le lancement a échoué : ${errMsg(err)}`);
      track('launch_error');
      uiLog('error', errMsg(err));
    } finally {
      pendingLaunch = null;
      state.launching = false;
      updatePlayButton();
    }
  }
  $('btn-play').addEventListener('click', () => play(settings.joinServer));
  $('btn-join').addEventListener('click', () => (state.gameRunning ? killGame() : play(true)));
  $('btn-kill').addEventListener('click', killGame);

  // Deux clics pour fermer le jeu (le premier prévient), sans boîte de dialogue native.
  let killArmedUntil = 0;
  async function killGame() {
    if (!state.gameRunning) return;
    if (Date.now() > killArmedUntil) {
      killArmedUntil = Date.now() + 5000;
      toast('Cliquez encore sur « Fermer le jeu » pour confirmer : ce qui n’est pas sauvegardé en jeu sera perdu.');
      return;
    }
    killArmedUntil = 0;
    const ok = await invoke('kill_game').catch(() => false);
    toast(ok ? 'Fermeture du jeu…' : 'Le jeu n’est plus ouvert.');
  }

  function activeProfile() { return settings.ramMin ? null : (settings.profile || (state.system && state.system.recommended_profile)); }

  function onGameExit(code) {
    state.gameRunning = false;
    state.presenceStart = Date.now();
    updatePresence('launcher');
    updatePlayButton();
    if (state.gameStartedAt) {
      const minutes = Math.round((Date.now() - state.gameStartedAt) / 60000);
      stats.playtime_minutes += minutes;
      state.gameStartedAt = null;
      saveStats();
      checkAchievements(true);
    }
  }

  // Fermeture anormale : le launcher a analysé le rapport de plantage et la fin du journal.
  function onGameCrash(a) {
    a = a || {};
    const cause = $('crash-cause');
    if (a.cause) {
      cause.replaceChildren(el('strong', { text: a.cause.title }), document.createTextNode(a.cause.advice));
      cause.classList.remove('hidden');
    } else cause.classList.add('hidden');
    $('crash-text').textContent = `Minecraft s’est arrêté (code ${a.exit_code == null ? '?' : a.exit_code}). `
      + (a.cause ? 'Si le conseil ne suffit pas, le diagnostic rassemble les journaux utiles (sans données personnelles).'
        : 'La cause n’a pas été reconnue : le diagnostic rassemble les journaux utiles (sans données personnelles).');
    $('crash-excerpt').textContent = a.excerpt || '';
    $('crash-excerpt').classList.toggle('hidden', !a.excerpt);
    state.lastCrashHasReport = !!a.crash_report;
    track(`game_crash:${(a.cause && a.cause.id) || 'unknown'}`, { exit_code: Number(a.exit_code) || 0 });
    notifyDesktop('Le jeu s’est fermé de façon inattendue', a.cause ? a.cause.title : 'Ouvrez le launcher pour lancer un diagnostic.');
    $('crash-overlay').classList.remove('hidden');
  }
  $('crash-close').addEventListener('click', () => $('crash-overlay').classList.add('hidden'));
  $('crash-console').addEventListener('click', () => {
    $('crash-overlay').classList.add('hidden');
    showPage('console');
    showLogKind(state.lastCrashHasReport ? 'crash' : 'latest');
  });
  $('crash-diag').addEventListener('click', () => { $('crash-overlay').classList.add('hidden'); openDiagnostic(); });

  // ===========================================================================
  // Succès
  // ===========================================================================
  function recordLaunch() {
    stats.launch_count += 1;
    if (!stats.first_launch) stats.first_launch = Date.now();
    if (!stats.instances.includes(settings.instance)) stats.instances.push(settings.instance);
    state.gameStartedAt = Date.now();
    saveStats();
    checkAchievements(true);
  }
  function achievementProgress(a) {
    const target = Number(a.condition_value) || 1;
    let value;
    switch (a.condition_type) {
      case 'first_launch': return { value: stats.launch_count > 0 ? 1 : 0, target: 1, done: stats.launch_count > 0 };
      case 'launch_count': value = stats.launch_count; break;
      case 'playtime_hours': value = Math.floor(stats.playtime_minutes / 60); break;
      case 'instances_tried': value = stats.instances.length; break;
      default: return { value: 0, target, done: stats.unlocked.includes(a.code), manual: true };
    }
    return { value: Math.min(value, target), target, done: value >= target };
  }
  function checkAchievements(notify) {
    for (const a of state.achievements) {
      if (achievementProgress(a).done && !stats.unlocked.includes(a.code)) {
        stats.unlocked.push(a.code);
        if (notify) toast(`🏆 Succès débloqué : ${a.name} (+${a.points || 0} pts)`);
      }
    }
    saveStats();
  }
  const BI = {
    'bi-star-fill': '⭐', 'bi-star': '⭐', 'bi-arrow-repeat': '🔁', 'bi-hourglass-split': '⏳', 'bi-hourglass': '⏳', 'bi-trophy': '🏆',
    'bi-trophy-fill': '🏆', 'bi-map': '🗺️', 'bi-controller': '🎮', 'bi-rocket': '🚀', 'bi-rocket-takeoff': '🚀', 'bi-fire': '🔥',
    'bi-gem': '💎', 'bi-heart': '❤️', 'bi-clock': '🕒', 'bi-people': '👥', 'bi-people-fill': '👥', 'bi-flag': '🚩', 'bi-flag-fill': '🚩',
    'bi-compass': '🧭', 'bi-calendar': '📅', 'bi-lightning': '⚡', 'bi-lightning-fill': '⚡', 'bi-shield': '🛡️', 'bi-shield-fill': '🛡️',
    'bi-award': '🏅', 'bi-globe': '🌍', 'bi-globe2': '🌍', 'bi-moon': '🌙', 'bi-sun': '☀️', 'bi-box': '📦', 'bi-camera': '📷',
    'bi-crosshair': '🎯', 'bi-bug': '🐛', 'bi-tree': '🌳', 'bi-gear': '⚙️', 'bi-hammer': '🔨', 'bi-bank': '🏛️', 'bi-cash': '💰',
  };
  const iconFor = (a) => { const r = typeof a.icon === 'string' ? a.icon.trim() : ''; return r.startsWith('bi-') ? (BI[r] || '🏆') : (r && [...r].length <= 3 ? r : '🏆'); };

  async function loadAchievements() {
    try {
      const list = await invoke('panel_get', { endpoint: 'utils/achievements', instance: null, channel: 'stable' });
      state.achievements = Array.isArray(list) ? list : [];
    } catch (_) { state.achievements = []; }
    checkAchievements(false);
  }
  function renderAchievements() {
    const box = $('achievements');
    box.replaceChildren();
    $('stat-launches').textContent = fmtNum(stats.launch_count);
    $('stat-playtime').textContent = `${Math.floor(stats.playtime_minutes / 60)} h`;
    if (state.achievements.length === 0) { box.append(el('p', { class: 'muted', text: 'Aucun succès disponible.' })); $('ach-score').textContent = '0'; return; }
    let points = 0;
    const sorted = state.achievements.slice().sort((a, b) => Number(achievementProgress(b).done) - Number(achievementProgress(a).done));
    for (const a of sorted) {
      const p = achievementProgress(a);
      const unlocked = p.done;
      if (unlocked) points += Number(a.points) || 0;
      const hidden = a.secret && !unlocked;
      const rarity = ['rare', 'epic', 'legendary'].includes(a.rarity) ? ` ${a.rarity}` : '';
      const bar = !unlocked && !hidden && !p.manual ? el('div', { class: 'bar' }, el('div', { class: 'bar-fill', style: { width: `${Math.round((p.value / (p.target || 1)) * 100)}%` } })) : null;
      box.append(el('div', { class: `ach${unlocked ? ' unlocked' : ''}${rarity}` }, [
        el('div', { class: 'icon', text: hidden ? '❔' : iconFor(a) }),
        el('div', null, [
          el('strong', { text: hidden ? 'Succès secret' : a.name }),
          el('div', { class: 'desc', text: hidden ? 'À découvrir…' : (a.description || '') }),
          el('div', { class: 'meta', text: `${a.points || 0} pts${a.category ? ` · ${a.category}` : ''}${bar ? ` · ${p.value}/${p.target}` : ''}${p.manual && !unlocked && !hidden ? ' · en jeu' : ''}` }),
          bar,
        ]),
      ]));
    }
    $('ach-score').textContent = fmtNum(points);
  }

  // ===========================================================================
  // Profil : classement, saison, factions
  // ===========================================================================
  let leaderboardTimer = null;
  let lastRanks = {};
  async function loadProfile() {
    await loadAchievements();
    renderAchievements();
    loadLeaderboard(); loadSeason(); loadFactions();
    clearInterval(leaderboardTimer);
    leaderboardTimer = setInterval(() => { if (state.page === 'profile') loadLeaderboard(); else clearInterval(leaderboardTimer); }, 30000);
  }
  async function loadLeaderboard() {
    const box = $('leaderboard');
    let list = [];
    try { list = await invoke('panel_get', { endpoint: 'utils/leaderboards', instance: settings.instance, channel: 'stable' }); } catch (_) {}
    box.replaceChildren();
    if (!Array.isArray(list) || list.length === 0) { box.append(el('li', { class: 'muted', text: 'Pas encore de classement.' })); return; }
    const me = state.account ? String(state.account.username).toLowerCase() : '';
    const ranks = {};
    for (const row of list.slice(0, 20)) {
      const value = row.coins ?? row.points ?? row.score ?? row.value ?? '';
      const name = String(row.name || '?');
      ranks[name] = row.rank;
      const moved = lastRanks[name] != null && lastRanks[name] !== row.rank;
      box.append(el('li', { class: `${name.toLowerCase() === me ? 'me' : ''}${moved ? ' flash' : ''}` }, [
        el('span', { class: 'rank', text: `#${row.rank ?? ''}` }), el('span', { class: 'name', text: name }), el('span', { class: 'muted', text: fmtNum(value) }),
      ]));
    }
    lastRanks = ranks;
  }
  async function loadSeason() {
    const box = $('season');
    let data = null;
    try { data = await invoke('panel_get', { endpoint: 'utils/seasons', instance: settings.instance, channel: 'stable' }); } catch (_) {}
    box.replaceChildren();
    const cur = data && data.current;
    if (!cur) { box.append(el('p', { class: 'muted small', text: 'Pas de saison en cours.' })); return; }
    box.append(el('p', null, [el('strong', { text: cur.name || 'Saison' }), el('span', { class: 'muted small', text: cur.endsAt ? ` · fin dans ${formatDuration(cur.endsAt - Date.now())}` : '' })]));
    const ol = el('ol', { class: 'leaderboard' });
    (cur.standings || []).slice(0, 10).forEach((s, i) => ol.append(el('li', null, [
      el('span', { class: 'rank', text: `#${i + 1}` }), el('span', { class: 'name', text: s.name || '?' }), el('span', { class: 'muted', text: `${fmtNum(s.points)} pts` }),
    ])));
    box.append(ol);
  }
  async function loadFactions() {
    const body = $('factions');
    let list = [];
    try { list = await invoke('panel_get', { endpoint: 'utils/factions', instance: settings.instance, channel: 'stable' }); } catch (_) {}
    body.replaceChildren();
    if (!Array.isArray(list) || list.length === 0) { body.append(el('tr', null, el('td', { colspan: '3', class: 'muted', text: 'Aucune faction.' }))); return; }
    for (const f of list.slice(0, 15)) {
      body.append(el('tr', null, [
        el('td', { text: f.tag && f.tag !== f.name ? `${f.name} [${f.tag}]` : f.name }), el('td', { text: fmtNum(f.members) }), el('td', { text: fmtNum(f.power) }),
      ]));
    }
  }

  // ===========================================================================
  // Performances (RAM adaptée au PC)
  // ===========================================================================
  async function loadSystem() {
    try { state.system = await invoke('get_system_overview', { profile: settings.profile || null }); } catch (_) { state.system = null; }
    const ram = launchRam();
    $('info-ram').textContent = `${ram.min}–${ram.max} Go`;
    const prof = settings.profile || (state.system && state.system.recommended_profile);
    $('info-profile').textContent = settings.ramMin || settings.jvmArgs ? 'Personnalisé' : (PROFILE_NAMES[prof] || '—');
    updatePlayButton();
  }

  async function renderPerformance() {
    await loadSystem();
    const sys = state.system;
    if (!sys) return;
    $('hw-cpu').textContent = `${sys.cpu_name || 'Processeur'} · ${sys.cpu_cores} cœurs`;
    $('hw-ram').textContent = `${sys.total_ram_gb} Go (jeu : ${sys.plan.cap_gb} Go max)`;
    $('hw-free').textContent = `${sys.free_ram_gb} Go`;
    $('hw-os').textContent = sys.os;
    if (!state.gpus) state.gpus = await invoke('detect_gpus').catch(() => ({ gpus: [], only_integrated: false }));
    $('hw-gpu').textContent = state.gpus.gpus.length ? state.gpus.gpus.join(' · ') : 'Non détectée';
    $('light-game').checked = !!settings.lightGame;
    const rec = sys.recommended_profile;
    $('optimize-text').textContent = `Profil conseillé pour ce PC : ${PROFILE_NAMES[rec]}. Le jeu n’utilisera jamais plus de 60 % de la mémoire (${sys.plan.cap_gb} Go).`
      + (state.gpus && state.gpus.only_integrated ? ' Carte graphique intégrée : cochez « alléger les réglages du jeu » pour gagner en fluidité.' : '');

    const ids = ['eco', 'balanced', 'performance'];
    const plans = await Promise.all(ids.map((id) => invoke('get_system_overview', { profile: id }).then((s) => s.plan).catch(() => null)));
    ids.forEach((id, i) => { state.plans[id] = plans[i]; });
    const active = settings.profile || rec;
    document.querySelectorAll('.profile-card').forEach((c) => {
      const id = c.dataset.profile;
      const plan = state.plans[id];
      c.classList.toggle('active', id === active && !settings.ramMin);
      c.querySelector('.tag')?.remove();
      if (id === rec) c.append(el('span', { class: 'tag', text: 'Conseillé' }));
      c.querySelector('.pc-ram').textContent = plan ? (plan.insufficient ? 'Mémoire insuffisante' : `${plan.ram_min_gb}–${plan.ram_max_gb} Go`) : '';
    });
    const ram = launchRam();
    const warn = $('perf-warning');
    if (sys.plan.insufficient) {
      warn.textContent = '⚠️ Ce PC n’a pas assez de mémoire pour le modpack (3 Go utilisables minimum).';
      warn.classList.remove('hidden');
    } else if (sys.free_ram_gb < ram.max) {
      warn.textContent = `⚠️ Seulement ${sys.free_ram_gb} Go de mémoire libre pour ${ram.max} Go demandés : fermez des applications (navigateur, Discord…) avant de jouer pour éviter les ralentissements.`;
      warn.classList.remove('hidden');
    } else warn.classList.add('hidden');
    $('profile-hint').textContent = settings.ramMin || settings.jvmArgs ? '· réglages personnalisés' : `· profil ${PROFILE_NAMES[active]}`;
    renderRam();
    $('jvm-args').value = launchJvmArgs().join('\n');
  }

  function renderRam() {
    const sys = state.system || { plan: {}, total_ram_gb: 8 };
    const cap = Math.max(1, sys.plan.cap_gb || 8);
    const ram = launchRam();
    $('ram-min').max = String(cap);
    $('ram-max').max = String(cap);
    $('ram-min').value = String(Math.min(ram.min, cap));
    $('ram-max').value = String(Math.min(ram.max, cap));
    $('ram-min-val').textContent = `${$('ram-min').value} Go`;
    $('ram-max-val').textContent = `${$('ram-max').value} Go`;
    $('ram-bar').style.width = `${Math.min(100, (Number($('ram-max').value) / (sys.total_ram_gb || 8)) * 100)}%`;
    $('ram-bar-text').textContent = `${$('ram-max').value} / ${sys.total_ram_gb} Go`;
    $('info-ram').textContent = `${ram.min}–${ram.max} Go`;
  }

  async function applyProfile(id, message) {
    settings.profile = id;
    settings.ramMin = null; settings.ramMax = null; settings.jvmArgs = null;
    saveSettings();
    await renderPerformance();
    const ram = launchRam();
    toast(message || `Profil ${PROFILE_NAMES[id]} appliqué : ${ram.min}–${ram.max} Go.`);
  }
  document.querySelectorAll('.profile-card').forEach((c) => c.addEventListener('click', () => applyProfile(c.dataset.profile)));
  $('btn-optimize').addEventListener('click', async () => {
    const sys = state.system || await invoke('get_system_overview', { profile: null }).catch(() => null);
    if (!sys) return toast('Impossible de lire le matériel du PC.');
    const rec = sys.recommended_profile;
    await applyProfile(rec, `⚡ Optimisé pour votre PC : profil ${PROFILE_NAMES[rec]}.`);
    const ram = launchRam();
    toast(`⚡ Optimisé pour votre PC (${sys.total_ram_gb} Go, ${sys.cpu_cores} cœurs) : profil ${PROFILE_NAMES[rec]}, ${ram.min}–${ram.max} Go.`);
  });
  function onRamInput(which) {
    if (which === 'min' && Number($('ram-min').value) > Number($('ram-max').value)) $('ram-max').value = $('ram-min').value;
    if (which === 'max' && Number($('ram-max').value) < Number($('ram-min').value)) $('ram-min').value = $('ram-max').value;
    settings.ramMin = Number($('ram-min').value);
    settings.ramMax = Number($('ram-max').value);
    saveSettings();
    document.querySelectorAll('.profile-card').forEach((c) => c.classList.remove('active'));
    $('profile-hint').textContent = '· réglages personnalisés';
    $('info-profile').textContent = 'Personnalisé';
    renderRam();
  }
  $('light-game').addEventListener('change', () => {
    settings.lightGame = $('light-game').checked;
    saveSettings();
    toast(settings.lightGame
      ? (activeProfile() === 'eco' ? 'Réglages légers appliqués au prochain lancement.' : 'Réglages légers : actifs quand le profil Économe est choisi.')
      : 'Le jeu garde vos réglages. « Remettre mes réglages du jeu » rétablit ceux d’avant.');
  });
  $('btn-restore-options').addEventListener('click', async () => {
    try {
      const done = await invoke('restore_game_options', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel });
      toast(done ? 'Vos réglages du jeu d’avant le profil Économe sont remis.' : 'Rien à remettre : le launcher n’a pas modifié vos réglages du jeu.');
    } catch (err) { toast(`Impossible : ${errMsg(err)}`); }
  });
  $('ram-min').addEventListener('input', () => onRamInput('min'));
  $('ram-max').addEventListener('input', () => onRamInput('max'));
  $('jvm-args').addEventListener('change', () => {
    settings.jvmArgs = $('jvm-args').value.split('\n').map((s) => s.trim()).filter((s) => s.startsWith('-') && !/^-Xm[sx]/i.test(s));
    saveSettings();
    toast('Arguments Java enregistrés.');
  });

  // ===========================================================================
  // Réglages
  // ===========================================================================
  async function renderSettings() {
    renderPerformance();
    $('resolution').value = settings.resolution;
    $('after-launch').value = settings.afterLaunch;
    $('join-server').checked = !!settings.joinServer;
    $('reduce-motion').checked = !!settings.reduceMotion;
    $('discord-rpc').checked = settings.discordRpc !== false;
    $('telemetry').checked = settings.telemetry === true;
    $('desktop-notifs').checked = settings.desktopNotifs !== false;
    renderTraySettings();
    $('language').value = (window.I18N && window.I18N.lang) || 'fr';
    renderOptionalMods();
    loadCommunity();
    try { const p = await invoke('get_panel_url'); $('panel-url').value = p.current === p.default ? '' : p.current; } catch (_) {}
    const sel = $('java-select');
    try {
      const jdks = await invoke('detect_jdks', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus' });
      sel.replaceChildren(el('option', { value: '', text: 'Automatique (Java 17 fourni par le launcher)' }));
      for (const j of jdks) sel.append(el('option', { value: j.path, text: `${j.label} — ${j.path}` }));
    } catch (_) {}
    if (settings.javaPath && ![...sel.options].some((o) => o.value === settings.javaPath)) sel.append(el('option', { value: settings.javaPath, text: settings.javaPath }));
    sel.value = settings.javaPath || '';
  }
  $('java-select').addEventListener('change', () => { settings.javaPath = $('java-select').value; saveSettings(); });
  $('resolution').addEventListener('change', () => { settings.resolution = $('resolution').value; saveSettings(); });
  $('after-launch').addEventListener('change', () => { settings.afterLaunch = $('after-launch').value; saveSettings(); });
  $('join-server').addEventListener('change', () => { settings.joinServer = $('join-server').checked; saveSettings(); });
  $('reduce-motion').addEventListener('change', () => { settings.reduceMotion = $('reduce-motion').checked; saveSettings(); applyMotion(); });
  const applyMotion = () => document.body.classList.toggle('reduce-motion', !!settings.reduceMotion);

  $('btn-panel-url').addEventListener('click', async () => {
    const st = $('panel-url-status');
    st.textContent = 'Test du panel…';
    try {
      const base = await invoke('set_panel_url', { url: $('panel-url').value });
      st.textContent = `✅ Panel utilisé : ${base}`;
      await loadConfig();
      loadStatuses(); loadNotifications(); loadLauncherContent(); loadGameEvents();
    } catch (err) { st.textContent = `❌ ${errMsg(err)}`; }
  });

  async function renderOptionalMods() {
    const box = $('optional-mods');
    const cfg = state.config;
    let data = null;
    try { data = await invoke('panel_get', { endpoint: 'utils/mods', instance: settings.instance, channel: settings.channel }); } catch (_) {}
    box.replaceChildren();
    const list = (data && data.optionalMods) || [];
    if (!cfg || list.length === 0) { box.append(el('p', { class: 'muted small', text: 'Aucun mod optionnel pour ce serveur.' })); return; }
    const meta = data.mods || {};
    const key = `${settings.instance}:${settings.channel}`;
    const saved = settings.optionalMods[key] || {};
    let onDisk = {};
    try { onDisk = await invoke('optional_mods_state', { dataDirectory: cfg.dataDirectory, instance: settings.instance, channel: settings.channel, files: list }); } catch (_) {}
    for (const file of list) {
      const info = meta[file] || {};
      const input = el('input', { type: 'checkbox' });
      input.checked = !!(Object.prototype.hasOwnProperty.call(saved, file) ? saved[file] : (onDisk[file] ?? info.recommanded));
      input.addEventListener('change', async () => {
        try {
          await invoke('set_optional_mod', { dataDirectory: cfg.dataDirectory, instance: settings.instance, channel: settings.channel, file, enabled: input.checked });
          saved[file] = input.checked;
          settings.optionalMods[key] = saved;
          saveSettings();
        } catch (err) { input.checked = !input.checked; toast(`Impossible de changer ce mod : ${errMsg(err)}`); }
      });
      box.append(el('label', { class: 'mod' }, [input, el('div', null, [
        el('strong', { text: info.name || file }),
        info.description ? el('div', { class: 'desc', text: info.description }) : null,
        info.recommanded ? el('div', { class: 'desc', text: '⭐ Recommandé' }) : null,
      ])]));
    }
  }

  $('btn-repair').addEventListener('click', async () => {
    const cfg = state.config;
    if (!cfg) return toast('Configuration indisponible.');
    if (!window.confirm('Vérifier tous les fichiers du modpack et supprimer ceux qui sont abîmés ? Ils seront re-téléchargés au prochain lancement. Vos réglages sont conservés.')) return;
    $('repair-status').textContent = 'Analyse en cours…';
    $('btn-repair').disabled = true;
    try {
      const r = await invoke('repair_instance', { dataDirectory: cfg.dataDirectory, instance: settings.instance, channel: settings.channel, ignored: (cfg.ignored || []).map(String) });
      $('repair-status').textContent = `${r.total} fichiers vérifiés · ${r.corrupted.length} abîmé(s) supprimé(s) · ${r.missing.length} manquant(s). Cliquez sur Jouer pour terminer.`;
    } catch (err) { $('repair-status').textContent = `Réparation impossible : ${errMsg(err)}`; } finally { $('btn-repair').disabled = false; }
  });
  $('btn-open-dir').addEventListener('click', () => {
    invoke('open_game_dir', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel })
      .catch((err) => toast(`Dossier impossible à ouvrir : ${errMsg(err)}`));
  });

  // ===========================================================================
  // Diagnostic
  // ===========================================================================
  let lastReport = '';
  async function openDiagnostic() {
    $('diag-overlay').classList.remove('hidden');
    $('diag-status').textContent = '';
    await runDiagnostic();
  }
  async function runDiagnostic() {
    const report = $('diag-report');
    const summary = $('diag-summary');
    report.textContent = 'Analyse en cours (système, installation, connexion aux serveurs)…';
    summary.replaceChildren();
    ['diag-copy', 'diag-save', 'diag-send', 'diag-rerun'].forEach((id) => { $(id).disabled = true; });
    try {
      const ram = launchRam();
      lastReport = await invoke('collect_diagnostic', {
        req: {
          data_directory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel,
          site_url: siteUrl(),
          settings_summary: `RAM ${ram.min}-${ram.max} Go, profil ${settings.profile || 'auto'}${settings.ramMin ? ' (personnalisé)' : ''}, Java ${settings.javaPath ? 'personnalisé' : 'fourni'}, résolution ${settings.resolution}`,
        },
      });
      report.textContent = lastReport;
      const net = lastReport.split('--- Réseau ---')[1] || '';
      for (const line of net.split('\n').slice(1, 5)) {
        const m = /^(\S+) : (OK|ÉCHEC) \((\d+) ms\)(.*)$/.exec(line.trim());
        if (m) summary.append(el('div', { class: `probe ${m[2] === 'OK' ? 'ok' : 'ko'}` }, [el('strong', { text: `${m[2] === 'OK' ? '✔' : '✖'} ${m[1]}` }), el('div', { class: 'muted', text: `${m[3]} ms${m[4]}` })]));
      }
    } catch (err) {
      report.textContent = `Diagnostic impossible : ${errMsg(err)}`;
    } finally {
      ['diag-copy', 'diag-save', 'diag-send', 'diag-rerun'].forEach((id) => { $(id).disabled = false; });
    }
  }
  $('btn-diagnostic').addEventListener('click', () => openDiagnostic());
  $('diag-close').addEventListener('click', () => $('diag-overlay').classList.add('hidden'));
  $('diag-rerun').addEventListener('click', () => runDiagnostic());
  $('diag-copy').addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(lastReport); $('diag-status').textContent = 'Rapport copié : collez-le sur Discord ou dans un ticket.'; } catch (_) { $('diag-status').textContent = 'Copie impossible.'; }
  });
  $('diag-save').addEventListener('click', async () => {
    try {
      const path = await invoke('save_diagnostic', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel, report: lastReport });
      $('diag-status').textContent = `Enregistré : ${path}`;
      invoke('open_game_dir', { dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus', instance: settings.instance, channel: settings.channel }).catch(() => {});
    } catch (err) { $('diag-status').textContent = `Enregistrement impossible : ${errMsg(err)}`; }
  });
  $('diag-send').addEventListener('click', async () => {
    $('diag-status').textContent = 'Envoi…';
    try {
      const r = await invoke('send_diagnostic', { instance: settings.instance, report: lastReport });
      $('diag-status').textContent = r.sent ? '✅ Rapport envoyé à l’équipe. Merci !' : 'Le panel ne reçoit pas encore les rapports : utilisez « Copier » et envoyez-le sur Discord.';
    } catch (err) { $('diag-status').textContent = `Envoi impossible : ${errMsg(err)}. Utilisez « Copier ».`; }
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') ['diag-overlay', 'crash-overlay', 'whatsnew-overlay'].forEach((id) => $(id).classList.add('hidden'));
  });

  // ===========================================================================
  // Mise à jour du launcher
  // ===========================================================================
  async function installUpdate(version) {
    $('update-overlay').classList.remove('hidden');
    $('update-text').textContent = `Installation de la version ${version}… Le launcher va redémarrer.`;
    try { await invoke('install_launcher_update'); } catch (err) { $('update-overlay').classList.add('hidden'); toast(`Mise à jour impossible : ${errMsg(err)}`); }
  }
  async function checkUpdate(auto) {
    try {
      const info = await invoke('check_launcher_update');
      if (info.available) {
        if (!auto) $('update-status').textContent = `Version ${info.version} disponible.`;
        store.set('nexus_rust_whatsnew', { version: info.version, notes: info.notes || '' });
        return installUpdate(info.version);
      }
      if (!auto) $('update-status').textContent = `Vous avez la dernière version (${info.current}).`;
    } catch (err) {
      uiLog('warn', `Mise à jour non vérifiée : ${errMsg(err)}`);
      if (!auto) $('update-status').textContent = 'Vérification impossible pour le moment (serveur de mises à jour injoignable ou aucune version publiée).';
    }
    return null;
  }
  $('btn-check-update').addEventListener('click', () => checkUpdate(false));

  // ===========================================================================
  // Statistiques anonymes (seulement avec l'accord du joueur) et notifications du système
  // ===========================================================================
  function track(event, extra) {
    if (settings.telemetry !== true) return;
    invoke('send_telemetry', { event, instance: settings.instance, extra: extra || null }).catch(() => {});
  }
  function askConsent() {
    if (settings.telemetry !== null && settings.telemetry !== undefined) return;
    $('consent').classList.remove('hidden');
  }
  function setConsent(v) {
    settings.telemetry = v;
    saveSettings();
    $('consent').classList.add('hidden');
    if ($('telemetry')) $('telemetry').checked = v;
    if (v) track('launch');
  }
  $('consent-yes').addEventListener('click', () => setConsent(true));
  $('consent-no').addEventListener('click', () => setConsent(false));
  $('telemetry').addEventListener('change', () => setConsent($('telemetry').checked));
  $('desktop-notifs').addEventListener('change', () => { settings.desktopNotifs = $('desktop-notifs').checked; saveSettings(); });

  // Notification du système, seulement si la fenêtre n'est pas sous les yeux du joueur.
  function notifyDesktop(title, body) {
    if (settings.desktopNotifs === false || (document.hasFocus() && !document.hidden)) return;
    invoke('desktop_notify', { title, body }).catch(() => {});
  }
  // Annonces de maintenance et d'événement : une seule notification par annonce.
  function notifyNewAnnouncements(list) {
    const seen = store.get('nexus_rust_notified', []);
    const fresh = (Array.isArray(list) ? list : []).filter((n) => n && n.id != null && ['maintenance', 'event'].includes(n.type) && !seen.includes(n.id));
    if (!fresh.length) return;
    if (seen.length) fresh.forEach((n) => notifyDesktop(n.type === 'maintenance' ? 'Maintenance annoncée' : 'Événement Geoventure', stripTags(n.message)));
    store.set('nexus_rust_notified', seen.concat(fresh.map((n) => n.id)).slice(-200));
  }

  // ===========================================================================
  // Merveille (concours de construction, /utils/wonder)
  // ===========================================================================
  async function loadWonder() {
    const card = $('wonder-card');
    let data = null;
    try { data = await invoke('panel_get', { endpoint: 'utils/wonder', instance: settings.instance, channel: settings.channel }); } catch (_) {}
    const w = data && (data.current || (Array.isArray(data.past) && data.past[0]));
    if (!w || !w.name) { card.classList.add('hidden'); return; }
    const phase = { upcoming: 'Bientôt', open: 'Constructions en cours', closed: 'Jury en délibération', published: 'Résultats publiés' }[w.phase] || '';
    const now = Date.now();
    let when = '';
    if (w.phase === 'upcoming' && w.opensAt > now) when = ` · ouvre dans ${formatDuration(w.opensAt - now)}`;
    else if (w.phase === 'open' && w.closesAt > now) when = ` · fin dans ${formatDuration(w.closesAt - now)}`;
    $('wonder-name').textContent = String(w.name);
    $('wonder-info').textContent = [w.theme ? `Thème : ${w.theme}` : '', phase + when].filter(Boolean).join(' · ');
    const ranking = Array.isArray(w.ranking) ? w.ranking.slice(0, 3) : [];
    $('wonder-rank').replaceChildren(...ranking.map((r) => el('li', { text: `${r.team || '?'}${r.total != null ? ` — ${fmtNum(r.total)} pts` : ''}` })));
    card.classList.remove('hidden');
  }

  // ===========================================================================
  // Discord Rich Presence (réglages rpc_* du panel)
  // ===========================================================================
  state.presenceStart = Date.now();
  function updatePresence(mode) {
    const c = state.config;
    if (settings.discordRpc === false || !c || !c.rpc_activation || !c.rpc_id) {
      invoke('discord_presence', { presence: null }).catch(() => {});
      return;
    }
    const srv = currentServer() || {};
    const name = srv.name || 'Geoventure';
    const buttons = [[c.rpc_button1, c.rpc_button1_url], [c.rpc_button2, c.rpc_button2_url]]
      .filter(([l, u]) => l && safeHttps(u)).map(([label, url]) => ({ label: String(label), url: safeHttps(url) }));
    const presence = {
      client_id: String(c.rpc_id),
      details: mode === 'game' ? `En jeu sur ${name}` : 'Dans le launcher Geoventure',
      state: mode === 'game' ? (settings.channel === 'test' ? 'Canal de test' : (srv.description || 'Survie moddée')) : String(c.rpc_state || 'Prépare son aventure'),
      large_image: String(c.rpc_large_image || ''), large_text: String(c.rpc_large_text || 'Geoventure'),
      small_image: String(c.rpc_small_image || ''), small_text: mode === 'game' ? 'En jeu' : String(c.rpc_small_text || 'Geoventure'),
      buttons, start_ms: state.presenceStart,
    };
    invoke('discord_presence', { presence }).catch(() => {});
  }
  // Zone de notification et démarrage avec l'ordinateur
  async function renderTraySettings() {
    const t = await invoke('tray_settings').catch(() => null);
    $('row-close-tray').classList.toggle('hidden', !(t && t.tray));
    $('close-to-tray').checked = !!settings.closeToTray;
    $('autostart').checked = !!(t && t.autostart);
  }
  $('language').addEventListener('change', () => {
    settings.language = $('language').value;
    saveSettings();
    window.location.reload();
  });
  $('close-to-tray').addEventListener('change', () => {
    settings.closeToTray = $('close-to-tray').checked;
    saveSettings();
    invoke('set_tray_settings', { closeToTray: settings.closeToTray, autostart: null }).catch(() => {});
  });
  $('autostart').addEventListener('change', async () => {
    try {
      await invoke('set_tray_settings', { closeToTray: null, autostart: $('autostart').checked });
      toast($('autostart').checked ? 'Le launcher s’ouvrira au démarrage de l’ordinateur.' : 'Le launcher ne s’ouvrira plus au démarrage.');
    } catch (err) { $('autostart').checked = !$('autostart').checked; toast(errMsg(err)); }
  });

  $('discord-rpc').addEventListener('change', () => {
    settings.discordRpc = $('discord-rpc').checked;
    saveSettings();
    updatePresence(state.gameRunning ? 'game' : 'launcher');
  });

  // ===========================================================================
  // Affluence (48 h) et vidéo de la communauté
  // ===========================================================================
  async function loadAffluence() {
    const card = $('affluence-card');
    try {
      const data = await invoke('panel_get', { endpoint: 'utils/servers-history', instance: settings.instance, channel: settings.channel });
      const points = Array.isArray(data && data.points) ? data.points.slice(-48) : [];
      if (!points.length) { card.classList.add('hidden'); return; }
      const max = Math.max(1, ...points.map((p) => Number(p.players) || 0));
      $('affluence-bars').replaceChildren(...points.map((p) => {
        const n = Math.max(0, Number(p.players) || 0);
        const at = new Date(Number(p.t));
        return el('div', { class: 'affluence-bar', style: { height: `${Math.max(6, Math.round((n / max) * 100))}%` },
          title: `${Number.isNaN(at.getTime()) ? '' : `${at.getHours()} h — `}${n} joueur${n > 1 ? 's' : ''}` });
      }));
      const peaks = (Array.isArray(data.peakHours) ? data.peakHours : []).map(Number).filter((h) => h >= 0 && h < 24);
      $('affluence-peak').textContent = peaks.length ? `Heures de pointe : ${peaks.map((h) => `${h} h`).join(', ')}` : '';
      card.classList.remove('hidden');
    } catch (_) { card.classList.add('hidden'); }
  }

  function renderVideo() {
    const c = state.config || {};
    const id = String(c.video_url || '');
    const card = $('video-card');
    if (!c.video_activate || !/^[A-Za-z0-9_-]{11}$/.test(id)) { card.classList.add('hidden'); return; }
    card.classList.remove('hidden');
    const thumb = $('video-thumb');
    thumb.replaceChildren(el('img', { src: `https://img.youtube.com/vi/${id}/hqdefault.jpg`, alt: '' }), el('span', { class: 'video-play', 'aria-hidden': 'true', text: '▶' }));
    thumb.onclick = () => thumb.replaceChildren(el('iframe', {
      src: `https://www.youtube-nocookie.com/embed/${id}?autoplay=1`, title: 'Vidéo de la communauté',
      allow: 'autoplay; encrypted-media; picture-in-picture', allowfullscreen: '',
    }));
    $('video-open').onclick = () => openUrl(`https://www.youtube.com/watch?v=${id}`);
  }

  // ===========================================================================
  // Mods de la communauté
  // ===========================================================================
  const community = { mods: [], installed: new Set(), category: 'all', term: '' };
  function modFileName(mod) {
    let fromUrl = '';
    try { fromUrl = decodeURIComponent(new URL(String(mod.url)).pathname.split('/').pop() || ''); } catch (_) {}
    const n = String(mod.filename || fromUrl || '').trim();
    return /^[^/\\\0]+\.jar$/i.test(n) && !n.includes('..') ? n : '';
  }
  async function loadCommunity() {
    const box = $('community-mods');
    box.replaceChildren(el('p', { class: 'muted small', text: 'Chargement…' }));
    let mods = null;
    for (const endpoint of ['utils/community-mods', 'api/centralcorp/community-mods']) {
      try { mods = await invoke('panel_get', { endpoint, instance: settings.instance, channel: settings.channel }); break; } catch (_) {}
    }
    if (!Array.isArray(mods)) { box.replaceChildren(el('p', { class: 'muted small', text: 'Liste des mods communauté injoignable pour le moment.' })); return; }
    community.mods = mods;
    community.installed = new Set(await invoke('community_mods_installed', gameArgs()).catch(() => []));
    const cats = [...new Set(mods.map((m) => String(m.category || '').trim()).filter(Boolean))].sort((a, b) => a.localeCompare(b));
    $('community-filters').replaceChildren(...['all', ...cats].map((c) => {
      const b = el('button', { class: `chip-btn${c === community.category ? ' active' : ''}`, text: c === 'all' ? 'Tous' : c });
      b.addEventListener('click', () => { community.category = c; loadCommunityRender(); document.querySelectorAll('#community-filters .chip-btn').forEach((x) => x.classList.toggle('active', x === b)); });
      return b;
    }));
    loadCommunityRender();
  }
  function loadCommunityRender() {
    const term = community.term.toLowerCase();
    const list = community.mods.filter((m) => (community.category === 'all' || String(m.category || '').trim() === community.category)
      && (!term || `${m.name || ''} ${m.description || ''} ${m.author || ''}`.toLowerCase().includes(term)));
    if (!list.length) { $('community-mods').replaceChildren(el('p', { class: 'muted small', text: community.mods.length ? 'Aucun mod ne correspond.' : 'Aucun mod communauté pour le moment.' })); return; }
    $('community-mods').replaceChildren(...list.map((m) => {
      const file = modFileName(m);
      const url = safeHttps(m.url);
      const installed = !!file && community.installed.has(file);
      const btn = el('button', { class: `btn btn-small${installed ? '' : ' btn-primary'}`, text: installed ? 'Retirer' : 'Installer' });
      btn.disabled = !file || !url;
      btn.addEventListener('click', async () => {
        btn.disabled = true;
        btn.textContent = '…';
        try {
          if (community.installed.has(file)) {
            await invoke('uninstall_community_mod', { ...gameArgs(), fileName: file });
            community.installed.delete(file);
            toast(`${m.name} retiré.`);
          } else {
            await invoke('install_community_mod', { ...gameArgs(), url, fileName: file, hash: String(m.sha256 || m.sha1 || m.hash || '') || null });
            community.installed.add(file);
            toast(`${m.name} installé : il sera chargé au prochain lancement.`);
          }
        } catch (err) { toast(`${m.name} : ${errMsg(err)}`); }
        loadCommunityRender();
      });
      const meta = [m.author, m.version ? `v${m.version}` : '', m.category].filter(Boolean).join(' · ');
      const icon = safeHttps(m.icon);
      const page = safeHttps(m.page || m.link || m.website);
      return el('div', { class: `cmod${installed ? ' installed' : ''}` }, [
        el('img', { src: icon || 'assets/img/icon.png', alt: '', loading: 'lazy', onerror: (e) => { e.target.onerror = null; e.target.src = 'assets/img/icon.png'; } }),
        el('div', { class: 'cmod-body' }, [
          el('strong', { text: String(m.name || file || 'Mod') }),
          meta ? el('span', { class: 'muted small', text: meta }) : '',
          el('p', { text: String(m.description || '') }),
          el('div', { class: 'cmod-actions' }, [btn, page ? (() => { const b = el('button', { class: 'btn btn-small', text: 'Page du mod' }); b.addEventListener('click', () => openUrl(page)); return b; })() : '']),
        ]),
      ]);
    }));
  }
  $('community-search').addEventListener('input', () => { community.term = $('community-search').value.trim(); loadCommunityRender(); });

  // ===========================================================================
  // Contenu : captures d'écran, packs de ressources, shaders
  // ===========================================================================
  const assetUrl = (p) => (T.core && T.core.convertFileSrc ? T.core.convertFileSrc(p) : '');
  function confirmButton(label, onConfirm) {
    const b = el('button', { class: 'btn btn-small', text: label });
    let armed = false;
    b.addEventListener('click', async () => {
      if (!armed) { armed = true; b.textContent = 'Confirmer ?'; b.classList.add('btn-danger'); setTimeout(() => { armed = false; b.textContent = label; b.classList.remove('btn-danger'); }, 4000); return; }
      await onConfirm();
    });
    return b;
  }
  const sizeText = (n) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} Mo` : `${Math.max(1, Math.round(n / 1024))} Ko`);
  async function renderContent() {
    const shots = await invoke('list_content', { ...gameArgs(), kind: 'screenshots' }).catch(() => null);
    const sEntries = shots ? shots.entries.slice(0, 60) : [];
    $('shots').replaceChildren(...(sEntries.length ? sEntries.map((e) => {
      const b = el('button', { class: 'shot', title: e.name }, [el('img', { src: assetUrl(e.path), alt: '', loading: 'lazy' }), el('span', { text: new Date(e.modified_ms).toLocaleString(LOCALE, { dateStyle: 'short', timeStyle: 'short' }) })]);
      b.addEventListener('click', () => invoke('open_content', { ...gameArgs(), kind: 'screenshots', name: e.name }).catch((err) => toast(errMsg(err))));
      return b;
    }) : [el('p', { class: 'muted small', text: 'Aucune capture pour le moment.' })]));

    const rp = await invoke('list_content', { ...gameArgs(), kind: 'resourcepacks' }).catch(() => null);
    renderPacks('resourcepacks', rp ? rp.entries : [], null);
    const sp = await invoke('list_content', { ...gameArgs(), kind: 'shaderpacks' }).catch(() => null);
    const loader = !!(sp && sp.shader_loader);
    $('shaders-hint').textContent = loader
      ? 'Choisissez un pack de shaders. Les shaders demandent une bonne carte graphique ; en cas de lenteur, choisissez « Aucun ».'
      : 'Les shaders demandent le mod Oculus : installez « Oculus » dans Mods › Mods de la communauté, puis revenez ici.';
    renderPacks('shaderpacks', sp ? sp.entries : [], loader ? { enabled: sp.shaders_enabled, pack: sp.shader_pack } : null);
  }
  function renderPacks(kind, entries, shader) {
    const box = $(`packs-${kind}`);
    const rows = entries.map((e) => {
      const active = shader && shader.enabled && shader.pack === e.name;
      const actions = [];
      if (shader) {
        const use = el('button', { class: `btn btn-small${active ? ' btn-primary' : ''}`, text: active ? 'Actif' : 'Activer' });
        use.disabled = active;
        use.addEventListener('click', () => invoke('set_shader_pack', { ...gameArgs(), pack: e.name }).then(renderContent).catch((err) => toast(errMsg(err))));
        actions.push(use);
      }
      actions.push(confirmButton('Supprimer', () => invoke('delete_content', { ...gameArgs(), kind, name: e.name }).then(renderContent).catch((err) => toast(errMsg(err)))));
      return el('div', { class: `pack${active ? ' active' : ''}` }, [el('span', { text: e.name, title: e.name }), el('small', { class: 'muted', text: e.is_dir ? 'dossier' : sizeText(e.size) }), ...actions]);
    });
    if (shader) {
      const none = el('button', { class: `btn btn-small${shader.enabled ? '' : ' btn-primary'}`, text: shader.enabled ? 'Désactiver les shaders' : 'Shaders désactivés' });
      none.disabled = !shader.enabled;
      none.addEventListener('click', () => invoke('set_shader_pack', { ...gameArgs(), pack: null }).then(renderContent).catch((err) => toast(errMsg(err))));
      rows.unshift(el('div', { class: 'pack' }, [el('span', { text: 'Aucun' }), none]));
    }
    box.replaceChildren(...(rows.length ? rows : [el('p', { class: 'muted small', text: 'Dossier vide.' })]));
  }
  document.querySelectorAll('[data-open-kind]').forEach((b) => b.addEventListener('click', () => invoke('open_content', { ...gameArgs(), kind: b.dataset.openKind, name: null }).catch((err) => toast(errMsg(err)))));

  // ===========================================================================
  // Quoi de neuf (notes de la version installée par la mise à jour automatique)
  // ===========================================================================
  async function showWhatsNew() {
    const pending = store.get('nexus_rust_whatsnew', null);
    if (!pending || !T.app) return;
    let current = '';
    try { current = await T.app.getVersion(); } catch (_) { return; }
    if (pending.version !== current) return;
    store.set('nexus_rust_whatsnew', null);
    $('whatsnew-version').textContent = `Version ${current}`;
    $('whatsnew-notes').textContent = String(pending.notes || 'Corrections et améliorations.').replace(/[#*`]/g, '').trim();
    $('whatsnew-overlay').classList.remove('hidden');
  }
  $('whatsnew-close').addEventListener('click', () => $('whatsnew-overlay').classList.add('hidden'));

  // ===========================================================================
  // Démarrage
  // ===========================================================================
  async function listenAll() {
    if (!T.event) return;
    const on = (name, fn) => T.event.listen(name, fn).catch((err) => uiLog('error', `Écoute ${name} impossible : ${errMsg(err)}`));
    await Promise.all([
      on('launcher://log', (e) => appendLogLine(e.payload)),
      on('launcher://install-progress', (e) => onInstallProgress(e.payload)),
      on('launcher://launch-result', (e) => onLaunchResult(e.payload)),
      on('launcher://game-exit', (e) => onGameExit(e.payload)),
      on('launcher://game-crash', (e) => onGameCrash(e.payload)),
      on('launcher://tray-play', () => play(settings.joinServer)),
      on('launcher://game-slow', () => toast('🐢 Le jeu met longtemps à s’ouvrir. Patientez encore un peu ; s’il n’apparaît pas, lancez un diagnostic.')),
      on('launcher://update-progress', (e) => { const p = e.payload || {}; $('update-fill').style.width = `${p.total ? Math.round((p.downloaded / p.total) * 100) : 0}%`; }),
      on('launcher://update-error', (e) => { $('update-overlay').classList.add('hidden'); toast(`Mise à jour impossible : ${e.payload}`); }),
    ]);
  }

  async function bootstrap() {
    applyMotion();
    renderServers();
    renderChannel();
    await listenAll();
    invoke('set_tray_settings', { closeToTray: !!settings.closeToTray, autostart: null }).catch(() => {});
    try { if (T.app) $('launcher-version').textContent = `Geoventure Launcher ${await T.app.getVersion()}`; } catch (_) {}
    (await invoke('get_logs').catch(() => [])).forEach(appendLogLine);
    try { const gs = await invoke('game_status'); state.gameRunning = !!(gs && gs.running); } catch (_) {}

    $('preload-text').textContent = 'Connexion au panel…';
    await loadConfig();
    $('preload-text').textContent = 'Vérification de la session…';
    const restored = await invoke('azuriom_restore_session', { fallbackSiteUrl: siteUrl() }).catch(() => null);
    await refreshAccounts();
    renderAccount(restored && restored.account ? restored.account : null);
    await loadSystem();
    $('preload').classList.add('done');

    loadStatuses(); loadNotifications(); loadGameEvents(); loadAchievements(); loadNews(); loadLauncherContent();
    loadAffluence(); renderVideo(); loadWonder(); updatePresence('launcher'); showWhatsNew();
    if (settings.telemetry === true) track('launch'); else setTimeout(askConsent, 4000);
    checkUpdate(true);
    setInterval(renderGameEvents, 1000);
    setInterval(loadStatuses, 60000);
    setInterval(loadNotifications, 5 * 60000);
    setInterval(loadGameEvents, 5 * 60000);
    setInterval(loadNews, 15 * 60000);
  }

  window.addEventListener('error', (e) => uiLog('error', `Erreur de l’interface : ${e.message}`));
  window.addEventListener('unhandledrejection', (e) => uiLog('error', `Erreur de l’interface : ${errMsg(e.reason)}`));
  bootstrap().catch((err) => {
    $('preload').classList.add('done');
    uiLog('error', `Démarrage de l’interface : ${errMsg(err)}`);
    toast(`Erreur au démarrage : ${errMsg(err)}`);
  });
})();
