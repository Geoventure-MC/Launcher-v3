(() => {
  'use strict';

  // ---------------------------------------------------------------------------
  // Pont Tauri
  // ---------------------------------------------------------------------------
  const tauriCore = window.__TAURI__ && window.__TAURI__.core;
  const tauriEvent = window.__TAURI__ && window.__TAURI__.event;
  const invoke = tauriCore
    ? (cmd, args) => tauriCore.invoke(cmd, args)
    : async (cmd) => { throw new Error(`Pont Tauri indisponible pour ${cmd}`); };

  const $ = (id) => document.getElementById(id);
  const errMsg = (err) => (err && (err.message || err.code)) || String(err);

  // Serveurs connus (même liste que le launcher Electron), complétés par le panel.
  const DEFAULT_SERVERS = [
    { id: 'geoventure', name: 'Geoventure', description: 'Aventure & Exploration', theme_color: '#4ade80', is_default: true },
    { id: 'elandor', name: 'Elandor', description: 'RPG & Fantaisie', theme_color: '#a78bfa' },
    { id: 'pokeland', name: 'Pokeland', description: 'Pokémon & Combat', theme_color: '#fb923c' },
  ];

  // ---------------------------------------------------------------------------
  // Stockage local (préférences du joueur, compteurs des succès)
  // ---------------------------------------------------------------------------
  const store = {
    get(key, fallback) {
      try {
        const raw = localStorage.getItem(key);
        return raw == null ? fallback : JSON.parse(raw);
      } catch (_) { return fallback; }
    },
    set(key, value) {
      try { localStorage.setItem(key, JSON.stringify(value)); } catch (_) {}
    },
  };

  const DEFAULT_SETTINGS = {
    instance: 'geoventure',
    channel: 'stable',
    profile: null,
    ramMin: null,
    ramMax: null,
    jvmArgs: null,
    javaPath: '',
    resolution: 'auto',
    afterLaunch: 'keep',
    joinServer: true,
    optionalMods: {},
  };
  const settings = Object.assign({}, DEFAULT_SETTINGS, store.get('nexus_rust_settings', {}));
  const saveSettings = () => store.set('nexus_rust_settings', settings);

  const stats = Object.assign(
    { launch_count: 0, playtime_minutes: 0, instances: [], first_launch: null, unlocked: [] },
    store.get('nexus_rust_stats', {}),
  );
  const saveStats = () => store.set('nexus_rust_stats', stats);

  const state = {
    config: null,
    offline: false,
    account: null,
    system: null,
    launching: false,
    gameRunning: false,
    gameStartedAt: null,
    servers: DEFAULT_SERVERS.slice(),
    statuses: [],
    achievements: [],
    page: 'home',
  };

  // ---------------------------------------------------------------------------
  // Outils d'affichage (texte uniquement : rien du panel n'est injecté en HTML)
  // ---------------------------------------------------------------------------
  function el(tag, props, children) {
    const node = document.createElement(tag);
    if (props) {
      for (const [k, v] of Object.entries(props)) {
        if (v == null) continue;
        if (k === 'class') node.className = v;
        else if (k === 'text') node.textContent = v;
        else if (k === 'dataset') Object.assign(node.dataset, v);
        else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
        else node.setAttribute(k, v);
      }
    }
    for (const c of [].concat(children || [])) {
      if (c == null) continue;
      node.append(c instanceof Node ? c : document.createTextNode(String(c)));
    }
    return node;
  }

  function safeColor(c) {
    return typeof c === 'string' && /^#[0-9a-fA-F]{3,8}$/.test(c) ? c : null;
  }

  let toastTimer = null;
  function toast(text) {
    const t = $('toast');
    t.textContent = text;
    t.classList.remove('hidden');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => t.classList.add('hidden'), 5000);
  }

  function nowStamp() {
    return new Date().toTimeString().slice(0, 8);
  }

  function appendLogLine(entry) {
    const out = $('log-output');
    const line = `[${entry.timestamp}] [${String(entry.level).toUpperCase()}] [${entry.source}] ${entry.message}\n`;
    const stick = out.scrollTop + out.clientHeight >= out.scrollHeight - 20;
    out.append(document.createTextNode(line));
    // Garde la console légère : 3 000 lignes maximum à l'écran.
    while (out.childNodes.length > 3000) out.removeChild(out.firstChild);
    if (stick) out.scrollTop = out.scrollHeight;
  }

  function uiLog(level, message) {
    appendLogLine({ timestamp: nowStamp(), level, source: 'ui', message });
  }

  function openUrl(url) {
    invoke('open_url', { url }).catch((e) => toast(`Lien impossible à ouvrir : ${errMsg(e)}`));
  }

  // ---------------------------------------------------------------------------
  // Navigation
  // ---------------------------------------------------------------------------
  function showPage(page) {
    state.page = page;
    document.querySelectorAll('.nav-btn').forEach((b) => b.classList.toggle('active', b.dataset.page === page));
    document.querySelectorAll('.page').forEach((p) => p.classList.toggle('hidden', p.id !== `page-${page}`));
    if (page === 'profile') loadProfile();
    if (page === 'settings') renderSettings();
    if (page === 'console') $('log-output').scrollTop = $('log-output').scrollHeight;
  }
  document.querySelectorAll('.nav-btn').forEach((b) => b.addEventListener('click', () => showPage(b.dataset.page)));

  // ---------------------------------------------------------------------------
  // Serveurs, canaux, grades
  // ---------------------------------------------------------------------------
  const currentServer = () => state.servers.find((s) => s.id === settings.instance) || state.servers[0];
  const roleName = () => (state.account && state.account.role && state.account.role.name) || '';

  function roleIn(list) {
    if (!Array.isArray(list) || list.length === 0) return true;
    const r = roleName().toLowerCase();
    return !!r && list.some((x) => String(x).toLowerCase() === r);
  }

  function channelAccess(channel) {
    const srv = currentServer() || {};
    const cfg = (state.config) || {};
    const mode = srv.channel || 'both';
    if (channel === 'test') {
      if (mode === 'stable') return { ok: false, why: 'Pas de canal Test pour ce serveur.' };
      const roles = srv.test_roles || cfg.test_roles || [];
      if (!roleIn(roles)) return { ok: false, why: `Réservé aux grades : ${roles.join(', ')}` };
      return { ok: true };
    }
    if (mode === 'test') return { ok: false, why: 'Ce serveur n’a que le canal Test.' };
    const roles = srv.allowed_roles || cfg.allowed_roles || [];
    if (!roleIn(roles)) return { ok: false, why: `Réservé aux grades : ${roles.join(', ')}` };
    return { ok: true };
  }

  function renderServers() {
    const box = $('server-pills');
    box.replaceChildren();
    for (const srv of state.servers) {
      const status = state.statuses.find((s) => s.id === srv.id);
      const btn = el('button', {
        class: `pill${srv.id === settings.instance ? ' active' : ''}`,
        role: 'tab',
        'aria-selected': String(srv.id === settings.instance),
        title: srv.description || null,
        onclick: () => selectInstance(srv.id),
      }, [
        el('span', { class: 'swatch' }),
        srv.name || srv.id,
        status && status.online ? el('span', { class: 'muted small', text: ` ${status.players ?? 0}` }) : null,
      ]);
      const color = safeColor(srv.theme_color || srv.color);
      btn.querySelector('.swatch').style.background = color || 'var(--muted)';
      box.append(btn);
    }
    const srv = currentServer();
    $('server-name').textContent = (srv && srv.name) || 'Geoventure';
    const accent = safeColor(srv && (srv.theme_color || srv.color));
    document.documentElement.style.setProperty('--accent', accent || '#4ade80');
  }

  function renderChannel() {
    const testAccess = channelAccess('test');
    if (settings.channel === 'test' && !testAccess.ok) {
      settings.channel = 'stable';
      saveSettings();
    }
    document.querySelectorAll('#channel-switch .seg').forEach((b) => {
      const active = b.dataset.channel === settings.channel;
      b.classList.toggle('active', active);
      b.setAttribute('aria-checked', String(active));
      if (b.dataset.channel === 'test') {
        b.disabled = !testAccess.ok;
        b.textContent = testAccess.ok ? '🧪 Test' : '🔒 Test';
        b.title = testAccess.ok ? 'Version de test du modpack (dossier séparé)' : testAccess.why;
      }
    });
    $('channel-hint').textContent = settings.channel === 'test'
      ? 'Canal Test : modpack de test, installé dans un dossier séparé.'
      : '';
  }

  document.querySelectorAll('#channel-switch .seg').forEach((b) => b.addEventListener('click', () => {
    if (state.launching || state.gameRunning || b.disabled) return;
    settings.channel = b.dataset.channel;
    saveSettings();
    loadConfig();
  }));

  function selectInstance(id) {
    if (state.launching || state.gameRunning || id === settings.instance) return;
    settings.instance = id;
    saveSettings();
    renderServers();
    renderStatus();
    loadConfig();
    loadNotifications();
  }

  // ---------------------------------------------------------------------------
  // Configuration du panel
  // ---------------------------------------------------------------------------
  function setBadge(text, cls) {
    const b = $('status-badge');
    b.textContent = text;
    b.className = `badge ${cls || ''}`;
  }

  async function loadConfig() {
    setBadge('Chargement…');
    renderChannel();
    try {
      const res = await invoke('fetch_panel_config', { instance: settings.instance, channel: settings.channel });
      state.config = res.config;
      state.offline = !!res.offline;
      mergeServers(res.config.servers);
      const loader = res.config.loader && res.config.loader.enable && res.config.loader.build
        ? `${res.config.loader.type} ${res.config.loader.build}`
        : 'vanilla';
      $('info-version').textContent = `Minecraft ${res.config.game_version || '1.20.1'} · ${loader}`;
      $('splash').textContent = res.config.splash || '';
      if (res.offline) setBadge('Hors ligne (dernière config connue)', 'badge-offline');
      else if (res.config.maintenance) setBadge('Maintenance', 'badge-offline');
      else setBadge('Panel en ligne', 'badge-online');
      renderAlert();
    } catch (err) {
      state.config = null;
      setBadge('Panel injoignable', 'badge-error');
      uiLog('error', `Panel injoignable : ${errMsg(err)}`);
    }
    renderServers();
    renderChannel();
    renderMaintenance();
    updatePlayButton();
  }

  function mergeServers(panelServers) {
    if (!Array.isArray(panelServers) || panelServers.length === 0) return;
    const merged = DEFAULT_SERVERS.map((d) => Object.assign({}, d, panelServers.find((p) => p.id === d.id) || {}));
    for (const p of panelServers) {
      if (p && p.id && !merged.some((m) => m.id === p.id)) merged.push(p);
    }
    state.servers = merged;
  }

  function renderAlert() {
    const cfg = state.config || {};
    const box = $('alert-banner');
    if (cfg.alert_activate && cfg.alert_msg) {
      box.textContent = String(cfg.alert_msg).replace(/<[^>]*>/g, '');
      box.classList.remove('hidden');
    } else {
      box.classList.add('hidden');
    }
  }

  function renderMaintenance() {
    const cfg = state.config || {};
    const box = $('maintenance-box');
    if (cfg.maintenance) {
      box.textContent = `🛠️ ${String(cfg.maintenance_message || 'Maintenance en cours.').replace(/<[^>]*>/g, '')}`;
      box.classList.remove('hidden');
    } else {
      box.classList.add('hidden');
    }
  }

  // Raisons qui empêchent de jouer (affichées sous le bouton).
  function playBlockReason() {
    const cfg = state.config;
    if (!state.account) return 'Connectez-vous pour jouer.';
    if (!cfg) return 'Configuration du serveur indisponible : réessayez dans un instant.';
    if (cfg.maintenance) return 'Serveur en maintenance.';
    if (cfg.email_verified && !state.account.email_verified) return 'Vérifiez votre adresse e-mail sur le site pour jouer.';
    if (cfg.whitelist_activate) {
      const names = (cfg.whitelist || []).map((n) => String(n).toLowerCase());
      const okName = names.includes(String(state.account.username).toLowerCase());
      const okRole = roleIn(cfg.whitelist_roles || []) && (cfg.whitelist_roles || []).length > 0;
      if (!okName && !okRole) return 'Accès réservé (liste blanche).';
    }
    const access = channelAccess(settings.channel);
    if (!access.ok) return access.why;
    if (state.system && state.system.plan && state.system.plan.insufficient) {
      return `Mémoire insuffisante : il faut au moins 3 Go utilisables (${state.system.plan.cap_gb} Go disponibles).`;
    }
    return '';
  }

  function updatePlayButton() {
    const busy = state.launching || state.gameRunning;
    const reason = busy ? '' : playBlockReason();
    $('btn-play').disabled = busy || !!reason;
    $('btn-play').textContent = state.launching ? 'Lancement…' : (state.gameRunning ? 'En jeu' : 'Jouer');
    $('play-block-reason').textContent = reason;
  }

  // ---------------------------------------------------------------------------
  // Statut des serveurs, annonces, événements
  // ---------------------------------------------------------------------------
  async function loadStatuses() {
    try {
      const list = await invoke('panel_get', { endpoint: 'utils/servers-status', instance: null, channel: settings.channel });
      state.statuses = Array.isArray(list) ? list : [];
    } catch (_) {
      state.statuses = [];
    }
    renderServers();
    renderStatus();
  }

  function renderStatus() {
    const box = $('server-status');
    const text = $('server-status-text');
    const st = state.statuses.find((s) => s.id === settings.instance);
    box.classList.remove('online', 'offline');
    if (!st) {
      text.textContent = 'Statut du serveur inconnu';
      return;
    }
    if (st.online) {
      box.classList.add('online');
      const ping = st.latency != null ? ` · ${st.latency} ms` : '';
      text.textContent = `En ligne · ${st.players ?? 0}/${st.max_players ?? '?'} joueurs${ping}`;
    } else {
      box.classList.add('offline');
      text.textContent = 'Serveur hors ligne';
    }
  }

  async function loadNotifications() {
    const box = $('notifications');
    let list = [];
    try {
      list = await invoke('panel_get', { endpoint: 'utils/notifications', instance: settings.instance, channel: settings.channel });
    } catch (_) {}
    box.replaceChildren();
    const dismissed = store.get('nexus_rust_dismissed', []);
    const now = Date.now();
    for (const n of Array.isArray(list) ? list : []) {
      if (!n || dismissed.includes(n.id)) continue;
      if (n.expiresAt && Number(n.expiresAt) < now) continue;
      const type = ['info', 'warning', 'maintenance', 'event'].includes(n.type) ? n.type : 'info';
      const icon = { info: 'ℹ️', warning: '⚠️', maintenance: '🛠️', event: '🎉' }[type];
      const item = el('div', { class: `notif ${type}` }, [
        el('span', { text: icon }),
        el('span', { class: 'msg', text: String(n.message || '').replace(/<[^>]*>/g, '') }),
      ]);
      if (n.url && /^https?:\/\//i.test(n.url)) {
        item.append(el('button', { class: 'btn-secondary', text: 'En savoir plus', onclick: () => openUrl(n.url) }));
      }
      item.append(el('button', {
        class: 'btn-ghost', text: '✕', title: 'Masquer', 'aria-label': 'Masquer l’annonce',
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
    } catch (_) {
      gameEvents = [];
    }
    renderGameEvents();
  }

  function formatDuration(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    const d = Math.floor(s / 86400);
    const h = Math.floor((s % 86400) / 3600);
    const m = Math.floor((s % 3600) / 60);
    if (d > 0) return `${d} j ${h} h`;
    if (h > 0) return `${h} h ${m} min`;
    return `${m} min ${s % 60} s`;
  }

  function renderGameEvents() {
    const box = $('game-events');
    const now = Date.now();
    const items = gameEvents
      .filter((e) => e && e.enabled !== false)
      .map((e) => {
        let phase = e.phase;
        if (phase === 'upcoming' && e.opensAt && e.opensAt <= now) phase = 'open';
        if (e.endsAt && e.endsAt <= now) phase = 'ended';
        return Object.assign({}, e, { phase });
      })
      .filter((e) => e.phase === 'open' || e.phase === 'upcoming')
      .sort((a, b) => (a.phase === b.phase ? (a.opensAt || 0) - (b.opensAt || 0) : a.phase === 'open' ? -1 : 1))
      .slice(0, 2);
    box.replaceChildren();
    if (items.length === 0) {
      box.append(el('p', { class: 'muted', text: 'Aucun événement pour le moment.' }));
      return;
    }
    for (const e of items) {
      const when = e.phase === 'open'
        ? (e.endsAt ? `En cours · se termine dans ${formatDuration(e.endsAt - now)}` : 'En cours')
        : `Ouvre dans ${formatDuration((e.opensAt || now) - now)}`;
      box.append(el('div', { class: 'event', dataset: { slug: String(e.slug || '') } }, [
        el('strong', { text: e.name || e.slug }),
        el('span', { class: 'muted small', text: when }),
      ]));
    }
  }

  // ---------------------------------------------------------------------------
  // Compte Azuriom
  // ---------------------------------------------------------------------------
  function siteUrl() {
    const cfg = state.config || {};
    const srv = currentServer() || {};
    return srv.azauth || cfg.azauth || 'https://geoventure.fr/';
  }

  function renderAccount(account) {
    state.account = account;
    $('login-screen').classList.toggle('hidden', !!account);
    $('account-mini').classList.toggle('hidden', !account);
    $('btn-logout').classList.toggle('hidden', !account);
    if (account) {
      $('az-username').textContent = account.username;
      $('az-role').textContent = account.role ? account.role.name : 'Joueur';
      if (account.skin_url && /^https:\/\//.test(account.skin_url)) $('az-avatar').src = account.skin_url;
      $('az-status').textContent = '';
    } else {
      $('az-2fa-wrap').classList.add('hidden');
    }
    renderChannel();
    updatePlayButton();
  }

  $('az-login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const status = $('az-status');
    status.textContent = 'Connexion…';
    try {
      const res = await invoke('azuriom_login', {
        siteUrl: siteUrl(),
        email: $('az-email').value.trim(),
        password: $('az-password').value,
        code2fa: $('az-2fa').value.trim() || null,
      });
      if (res.outcome === 'requires2fa') {
        $('az-2fa-wrap').classList.remove('hidden');
        status.textContent = 'Entrez le code de votre application de double authentification.';
        $('az-2fa').focus();
      } else if (res.outcome === 'authenticated') {
        $('az-password').value = '';
        $('az-2fa').value = '';
        renderAccount(res.account);
        loadConfig();
      } else {
        status.textContent = res.message || 'Identifiants refusés.';
      }
    } catch (err) {
      status.textContent = `Connexion impossible : ${errMsg(err)}`;
    }
  });

  $('btn-logout').addEventListener('click', async () => {
    await invoke('azuriom_logout').catch(() => {});
    renderAccount(null);
  });

  // ---------------------------------------------------------------------------
  // Lancement
  // ---------------------------------------------------------------------------
  const PHASES = {
    minecraft: 'Minecraft',
    assets: 'Ressources du jeu',
    java: 'Java 17',
    forge: 'Forge',
    'forge-patch': 'Patch de Forge',
    modpack: 'Modpack',
    launch: 'Lancement',
  };

  function onInstallProgress(p) {
    const pct = p.total > 0 ? Math.round((p.completed / p.total) * 100) : 0;
    $('progress-fill').style.width = `${p.total > 0 ? pct : 0}%`;
    const name = PHASES[p.phase] || p.label;
    const count = p.total > 0 ? ` · ${p.completed}/${p.total}` : '';
    const label = p.phase === 'forge-patch' ? p.label : name;
    $('progress-text').textContent = `${label}${count}${p.current_file && p.phase !== 'forge-patch' ? ` · ${p.current_file.split('/').pop()}` : ''}`;
  }

  function launchRam() {
    const plan = (state.system && state.system.plan) || {};
    const cfg = state.config || {};
    const max = settings.ramMax || plan.ram_max_gb || cfg.ram_max || 4;
    const min = Math.min(settings.ramMin || plan.ram_min_gb || cfg.ram_min || 2, max);
    return { min, max };
  }

  function launchJvmArgs() {
    if (Array.isArray(settings.jvmArgs)) return settings.jvmArgs;
    const plan = (state.system && state.system.plan) || {};
    return plan.jvm_args || [];
  }

  async function applyOptionalModDefaults() {
    let data;
    try {
      data = await invoke('panel_get', { endpoint: 'utils/mods', instance: settings.instance, channel: settings.channel });
    } catch (_) { return; }
    const list = (data && data.optionalMods) || [];
    const meta = (data && data.mods) || {};
    const key = `${settings.instance}:${settings.channel}`;
    const saved = settings.optionalMods[key] || {};
    for (const file of list) {
      if (Object.prototype.hasOwnProperty.call(saved, file)) continue;
      const recommended = !!(meta[file] && meta[file].recommanded);
      if (!recommended) {
        await invoke('set_optional_mod', {
          dataDirectory: state.config.dataDirectory, instance: settings.instance, channel: settings.channel, file, enabled: false,
        }).catch(() => {});
      }
      saved[file] = recommended;
    }
    settings.optionalMods[key] = saved;
    saveSettings();
  }

  $('btn-play').addEventListener('click', async () => {
    if (state.launching || playBlockReason()) return;
    const cfg = state.config;
    const srv = currentServer() || {};
    state.launching = true;
    updatePlayButton();
    $('progress-fill').style.width = '0%';
    $('progress-text').textContent = 'Préparation…';
    try {
      await applyOptionalModDefaults();
      const ram = launchRam();
      const isTest = settings.channel === 'test';
      const host = isTest ? (srv.test_ip || srv.ip) : (srv.ip || (cfg.status && cfg.status.ip));
      const portRaw = isTest ? (srv.test_port || srv.port) : (srv.port || (cfg.status && cfg.status.port));
      const port = Number.parseInt(portRaw, 10);
      let width = null;
      let height = null;
      if (settings.resolution && settings.resolution !== 'auto') {
        [width, height] = settings.resolution.split('x').map((n) => Number.parseInt(n, 10));
      }
      const extraIgnored = Array.isArray(cfg.ignored) ? cfg.ignored : Object.values(cfg.ignored || {});
      // La commande répond tout de suite ; le résultat arrive par « launcher://launch-result ».
      const result = new Promise((resolve, reject) => { pendingLaunch = { resolve, reject }; });
      await invoke('launch_instance', {
        req: {
          data_directory: cfg.dataDirectory || 'Nexus',
          instance: settings.instance,
          channel: settings.channel,
          mc_version: cfg.game_version || '1.20.1',
          loader_build: (cfg.loader && cfg.loader.build) || '1.20.1-47.4.28',
          loader_type: (cfg.loader && cfg.loader.type) || 'forge',
          loader_enable: cfg.loader ? cfg.loader.enable !== false : true,
          custom_java_path: settings.javaPath || null,
          username: state.account.username,
          uuid: state.account.uuid,
          access_token: state.account.access_token,
          ram_min_gb: ram.min,
          ram_max_gb: ram.max,
          extra_jvm_args: launchJvmArgs(),
          ignored: extraIgnored.map(String),
          server_host: settings.joinServer && host ? String(host) : null,
          server_port: settings.joinServer && Number.isFinite(port) ? port : null,
          width,
          height,
          after_launch: settings.afterLaunch,
        },
      });
      const pid = await result;
      $('progress-fill').style.width = '100%';
      $('progress-text').textContent = `Minecraft est lancé (PID ${pid}). Bon jeu !`;
      state.gameRunning = true;
      recordLaunch();
    } catch (err) {
      $('progress-text').textContent = `Échec : ${errMsg(err)}`;
      toast(`Le lancement a échoué : ${errMsg(err)}`);
    } finally {
      pendingLaunch = null;
      state.launching = false;
      updatePlayButton();
    }
  });

  let pendingLaunch = null;
  function onLaunchResult(r) {
    if (!pendingLaunch) return;
    if (r && r.ok) pendingLaunch.resolve(r.pid);
    else pendingLaunch.reject((r && r.error) || new Error('Lancement impossible'));
  }

  // ---------------------------------------------------------------------------
  // Succès (compteurs locaux + catalogue du panel)
  // ---------------------------------------------------------------------------
  function recordLaunch() {
    stats.launch_count += 1;
    if (!stats.first_launch) stats.first_launch = Date.now();
    const inst = `${settings.instance}`;
    if (!stats.instances.includes(inst)) stats.instances.push(inst);
    state.gameStartedAt = Date.now();
    saveStats();
    checkAchievements(true);
  }

  function onGameExit() {
    state.gameRunning = false;
    updatePlayButton();
    if (state.gameStartedAt) {
      stats.playtime_minutes += Math.round((Date.now() - state.gameStartedAt) / 60000);
      state.gameStartedAt = null;
      saveStats();
      checkAchievements(true);
    }
    $('progress-text').textContent = 'Partie terminée.';
  }

  function achievementProgress(a) {
    const target = Number(a.condition_value) || 1;
    let value = 0;
    switch (a.condition_type) {
      case 'first_launch': value = stats.launch_count > 0 ? 1 : 0; break;
      case 'launch_count': value = stats.launch_count; break;
      case 'playtime_hours': value = Math.floor(stats.playtime_minutes / 60); break;
      case 'instances_tried': value = stats.instances.length; break;
      default: return { value: 0, target, done: stats.unlocked.includes(a.code) };
    }
    const tgt = a.condition_type === 'first_launch' ? 1 : target;
    return { value: Math.min(value, tgt), target: tgt, done: value >= tgt };
  }

  function checkAchievements(notify) {
    for (const a of state.achievements) {
      const p = achievementProgress(a);
      if (p.done && !stats.unlocked.includes(a.code)) {
        stats.unlocked.push(a.code);
        if (notify) toast(`🏆 Succès débloqué : ${a.name}`);
      }
    }
    saveStats();
  }

  const BI_ICONS = {
    'bi-star-fill': '⭐', 'bi-star': '⭐', 'bi-arrow-repeat': '🔁', 'bi-hourglass-split': '⏳', 'bi-hourglass': '⏳',
    'bi-trophy': '🏆', 'bi-trophy-fill': '🏆', 'bi-map': '🗺️', 'bi-controller': '🎮', 'bi-rocket': '🚀',
    'bi-rocket-takeoff': '🚀', 'bi-fire': '🔥', 'bi-gem': '💎', 'bi-heart': '❤️', 'bi-heart-fill': '❤️',
    'bi-clock': '🕒', 'bi-clock-history': '🕒', 'bi-people': '👥', 'bi-people-fill': '👥', 'bi-flag': '🚩',
    'bi-compass': '🧭', 'bi-calendar': '📅', 'bi-calendar-check': '📅', 'bi-lightning': '⚡', 'bi-lightning-fill': '⚡',
    'bi-shield': '🛡️', 'bi-shield-fill': '🛡️', 'bi-award': '🏅', 'bi-award-fill': '🏅', 'bi-globe': '🌍',
    'bi-moon': '🌙', 'bi-sun': '☀️', 'bi-play-fill': '▶️', 'bi-box': '📦', 'bi-diagram-3': '🧩',
  };
  function iconFor(a) {
    const raw = typeof a.icon === 'string' ? a.icon.trim() : '';
    if (raw.startsWith('bi-')) return BI_ICONS[raw] || '🏆';
    if (raw && [...raw].length <= 3) return raw;
    return '🏆';
  }

  async function loadAchievements() {
    try {
      const list = await invoke('panel_get', { endpoint: 'utils/achievements', instance: null, channel: 'stable' });
      state.achievements = Array.isArray(list) ? list : [];
    } catch (_) {
      state.achievements = [];
    }
    checkAchievements(false);
  }

  function renderAchievements() {
    const box = $('achievements');
    box.replaceChildren();
    if (state.achievements.length === 0) {
      box.append(el('p', { class: 'muted', text: 'Aucun succès disponible.' }));
      $('ach-score').textContent = '';
      return;
    }
    let points = 0;
    let total = 0;
    for (const a of state.achievements) {
      const p = achievementProgress(a);
      const unlocked = p.done || stats.unlocked.includes(a.code);
      total += Number(a.points) || 0;
      if (unlocked) points += Number(a.points) || 0;
      const hidden = a.secret && !unlocked;
      const bar = el('div', { class: 'bar' }, el('div'));
      bar.firstChild.style.width = `${Math.round((p.value / (p.target || 1)) * 100)}%`;
      box.append(el('div', { class: `ach${unlocked ? ' unlocked' : ''}` }, [
        el('div', { class: 'icon', text: hidden ? '❔' : iconFor(a) }),
        el('div', null, [
          el('strong', { text: hidden ? 'Succès secret' : a.name }),
          el('div', { class: 'desc', text: hidden ? 'À découvrir…' : (a.description || '') }),
          el('div', { class: 'meta', text: `${a.points || 0} pts${a.category ? ` · ${a.category}` : ''}${!unlocked && a.condition_type !== 'manual' && !hidden ? ` · ${p.value}/${p.target}` : ''}` }),
          unlocked || hidden || a.condition_type === 'manual' ? null : bar,
        ]),
      ]));
    }
    $('ach-score').textContent = `${points} / ${total} pts`;
  }

  // ---------------------------------------------------------------------------
  // Profil : classement, saison, factions
  // ---------------------------------------------------------------------------
  let leaderboardTimer = null;
  async function loadProfile() {
    await loadAchievements();
    renderAchievements();
    loadLeaderboard();
    loadSeason();
    loadFactions();
    clearInterval(leaderboardTimer);
    leaderboardTimer = setInterval(() => {
      if (state.page === 'profile') loadLeaderboard();
      else clearInterval(leaderboardTimer);
    }, 30000);
  }

  function formatNumber(n) {
    return Number.isFinite(Number(n)) ? Number(n).toLocaleString('fr-FR') : String(n ?? '');
  }

  async function loadLeaderboard() {
    const box = $('leaderboard');
    let list = [];
    try {
      list = await invoke('panel_get', { endpoint: 'utils/leaderboards', instance: settings.instance, channel: 'stable' });
    } catch (_) {}
    box.replaceChildren();
    if (!Array.isArray(list) || list.length === 0) {
      box.append(el('li', { class: 'muted', text: 'Pas encore de classement.' }));
      return;
    }
    const me = state.account ? String(state.account.username).toLowerCase() : '';
    for (const row of list.slice(0, 20)) {
      const value = row.coins ?? row.points ?? row.score ?? row.value ?? '';
      box.append(el('li', { class: String(row.name || '').toLowerCase() === me ? 'me' : null }, [
        el('span', { class: 'rank', text: `#${row.rank ?? ''}` }),
        el('span', { class: 'name', text: row.name || '?' }),
        el('span', { class: 'muted', text: formatNumber(value) }),
      ]));
    }
  }

  async function loadSeason() {
    const box = $('season');
    let data = null;
    try {
      data = await invoke('panel_get', { endpoint: 'utils/seasons', instance: settings.instance, channel: 'stable' });
    } catch (_) {}
    box.replaceChildren();
    const cur = data && data.current;
    if (!cur) {
      box.append(el('p', { class: 'muted', text: 'Pas de saison en cours.' }));
      return;
    }
    const ends = cur.endsAt ? ` · se termine dans ${formatDuration(cur.endsAt - Date.now())}` : '';
    box.append(el('p', null, [el('strong', { text: cur.name || 'Saison' }), el('span', { class: 'muted small', text: ends })]));
    const ol = el('ol', { class: 'leaderboard' });
    (cur.standings || []).slice(0, 10).forEach((s, i) => ol.append(el('li', null, [
      el('span', { class: 'rank', text: `#${i + 1}` }),
      el('span', { class: 'name', text: s.name || '?' }),
      el('span', { class: 'muted', text: `${formatNumber(s.points)} pts` }),
    ])));
    box.append(ol);
  }

  async function loadFactions() {
    const body = $('factions');
    let list = [];
    try {
      list = await invoke('panel_get', { endpoint: 'utils/factions', instance: settings.instance, channel: 'stable' });
    } catch (_) {}
    body.replaceChildren();
    if (!Array.isArray(list) || list.length === 0) {
      body.append(el('tr', null, el('td', { colspan: '3', class: 'muted', text: 'Aucune faction.' })));
      return;
    }
    for (const f of list.slice(0, 15)) {
      body.append(el('tr', null, [
        el('td', { text: f.tag && f.tag !== f.name ? `${f.name} [${f.tag}]` : f.name }),
        el('td', { text: formatNumber(f.members) }),
        el('td', { text: formatNumber(f.power) }),
      ]));
    }
  }

  // ---------------------------------------------------------------------------
  // Réglages
  // ---------------------------------------------------------------------------
  async function loadSystem(profile) {
    try {
      state.system = await invoke('get_system_overview', { profile: profile || settings.profile || null });
    } catch (_) {
      state.system = null;
    }
    const ram = launchRam();
    $('info-ram').textContent = `${ram.min}–${ram.max} Go`;
    updatePlayButton();
  }

  function renderRam() {
    const sys = state.system || { plan: {} };
    const cap = Math.max(1, sys.plan.cap_gb || 8);
    const ram = launchRam();
    for (const id of ['ram-min', 'ram-max']) $(id).max = String(cap);
    $('ram-min').value = String(Math.min(ram.min, cap));
    $('ram-max').value = String(Math.min(ram.max, cap));
    $('ram-min-val').textContent = `${$('ram-min').value} Go`;
    $('ram-max-val').textContent = `${$('ram-max').value} Go`;
    $('info-ram').textContent = `${ram.min}–${ram.max} Go`;
  }

  async function renderSettings() {
    await loadSystem();
    const sys = state.system;
    if (sys) {
      $('hw-summary').textContent = `${sys.total_ram_gb} Go de mémoire, ${sys.cpu_cores} cœurs · profil conseillé : ${({ eco: 'Économe', balanced: 'Équilibré', performance: 'Performant' })[sys.recommended_profile]}. Le jeu n’utilisera jamais plus de ${sys.plan.cap_gb} Go (60 %).`;
    }
    const active = settings.profile || (sys && sys.recommended_profile);
    document.querySelectorAll('#profile-switch .seg').forEach((b) => {
      b.classList.toggle('active', b.dataset.profile === active);
      b.setAttribute('aria-checked', String(b.dataset.profile === active));
    });
    $('profile-hint').textContent = sys && sys.plan.insufficient
      ? '⚠️ Pas assez de mémoire pour le modpack (3 Go minimum utilisables).'
      : (settings.ramMin || settings.jvmArgs ? 'Réglages personnalisés.' : 'Réglages du profil appliqués.');
    renderRam();
    $('jvm-args').value = launchJvmArgs().join('\n');
    $('resolution').value = settings.resolution;
    $('after-launch').value = settings.afterLaunch;

    // Java détectés
    const sel = $('java-select');
    const cfg = state.config || {};
    try {
      const jdks = await invoke('detect_jdks', { dataDirectory: cfg.dataDirectory || 'Nexus' });
      sel.replaceChildren(el('option', { value: '', text: 'Automatique (Java 17 fourni)' }));
      for (const j of jdks) sel.append(el('option', { value: j.path, text: `${j.label} — ${j.path}` }));
    } catch (_) {}
    if (settings.javaPath && ![...sel.options].some((o) => o.value === settings.javaPath)) {
      sel.append(el('option', { value: settings.javaPath, text: settings.javaPath }));
    }
    sel.value = settings.javaPath || '';
    renderOptionalMods();
  }

  document.querySelectorAll('#profile-switch .seg').forEach((b) => b.addEventListener('click', async () => {
    settings.profile = b.dataset.profile;
    settings.ramMin = null;
    settings.ramMax = null;
    settings.jvmArgs = null;
    saveSettings();
    await renderSettings();
    toast('Profil de performance appliqué.');
  }));

  $('ram-min').addEventListener('input', () => {
    if (Number($('ram-min').value) > Number($('ram-max').value)) $('ram-max').value = $('ram-min').value;
    settings.ramMin = Number($('ram-min').value);
    settings.ramMax = Number($('ram-max').value);
    saveSettings();
    renderRam();
  });
  $('ram-max').addEventListener('input', () => {
    if (Number($('ram-max').value) < Number($('ram-min').value)) $('ram-min').value = $('ram-max').value;
    settings.ramMin = Number($('ram-min').value);
    settings.ramMax = Number($('ram-max').value);
    saveSettings();
    renderRam();
  });
  $('jvm-args').addEventListener('change', () => {
    const args = $('jvm-args').value.split('\n').map((s) => s.trim()).filter((s) => s.startsWith('-') && !/^-Xm[sx]/i.test(s));
    settings.jvmArgs = args;
    saveSettings();
  });
  $('java-select').addEventListener('change', () => { settings.javaPath = $('java-select').value; saveSettings(); });
  $('resolution').addEventListener('change', () => { settings.resolution = $('resolution').value; saveSettings(); });
  $('after-launch').addEventListener('change', () => { settings.afterLaunch = $('after-launch').value; saveSettings(); });

  async function renderOptionalMods() {
    const box = $('optional-mods');
    const cfg = state.config;
    let data = null;
    try {
      data = await invoke('panel_get', { endpoint: 'utils/mods', instance: settings.instance, channel: settings.channel });
    } catch (_) {}
    box.replaceChildren();
    const list = (data && data.optionalMods) || [];
    if (!cfg || list.length === 0) {
      box.append(el('p', { class: 'muted small', text: 'Aucun mod optionnel pour ce serveur.' }));
      return;
    }
    const meta = data.mods || {};
    const key = `${settings.instance}:${settings.channel}`;
    const saved = settings.optionalMods[key] || {};
    let onDisk = {};
    try {
      onDisk = await invoke('optional_mods_state', {
        dataDirectory: cfg.dataDirectory, instance: settings.instance, channel: settings.channel, files: list,
      });
    } catch (_) {}
    for (const file of list) {
      const info = meta[file] || {};
      const enabled = Object.prototype.hasOwnProperty.call(saved, file) ? saved[file] : (onDisk[file] ?? !!info.recommanded);
      const input = el('input', { type: 'checkbox' });
      input.checked = !!enabled;
      input.addEventListener('change', async () => {
        try {
          await invoke('set_optional_mod', {
            dataDirectory: cfg.dataDirectory, instance: settings.instance, channel: settings.channel, file, enabled: input.checked,
          });
          saved[file] = input.checked;
          settings.optionalMods[key] = saved;
          saveSettings();
        } catch (err) {
          input.checked = !input.checked;
          toast(`Impossible de changer ce mod : ${errMsg(err)}`);
        }
      });
      box.append(el('label', { class: 'mod' }, [
        input,
        el('div', null, [
          el('strong', { text: info.name || file }),
          info.description ? el('div', { class: 'desc', text: info.description }) : null,
          info.recommanded ? el('div', { class: 'desc', text: '⭐ Recommandé' }) : null,
        ]),
      ]));
    }
  }

  $('btn-repair').addEventListener('click', async () => {
    const cfg = state.config;
    if (!cfg) return toast('Configuration indisponible.');
    if (!window.confirm('Vérifier tous les fichiers du modpack et supprimer ceux qui sont abîmés ? Ils seront re-téléchargés au prochain lancement. Vos réglages sont conservés.')) return;
    $('repair-status').textContent = 'Analyse en cours…';
    $('btn-repair').disabled = true;
    try {
      const r = await invoke('repair_instance', {
        dataDirectory: cfg.dataDirectory, instance: settings.instance, channel: settings.channel,
        ignored: (cfg.ignored || []).map(String),
      });
      $('repair-status').textContent = `${r.total} fichiers vérifiés · ${r.corrupted.length} abîmé(s) supprimé(s) · ${r.missing.length} manquant(s). Cliquez sur Jouer pour terminer.`;
    } catch (err) {
      $('repair-status').textContent = `Réparation impossible : ${errMsg(err)}`;
    } finally {
      $('btn-repair').disabled = false;
    }
  });

  $('btn-open-dir').addEventListener('click', () => {
    const cfg = state.config || {};
    invoke('open_game_dir', { dataDirectory: cfg.dataDirectory || 'Nexus', instance: settings.instance, channel: settings.channel })
      .catch((err) => toast(`Dossier impossible à ouvrir : ${errMsg(err)}`));
  });

  // ---------------------------------------------------------------------------
  // Mise à jour du launcher
  // ---------------------------------------------------------------------------
  async function installUpdate(version) {
    $('update-overlay').classList.remove('hidden');
    $('update-text').textContent = `Installation de la version ${version}… Le launcher va redémarrer.`;
    try {
      await invoke('install_launcher_update');
    } catch (err) {
      $('update-overlay').classList.add('hidden');
      toast(`Mise à jour impossible : ${errMsg(err)}`);
    }
  }

  async function checkUpdate(auto) {
    try {
      const info = await invoke('check_launcher_update');
      $('launcher-version').textContent = `Launcher ${info.current}`;
      if (info.available) {
        if (auto) return installUpdate(info.version);
        $('update-status').textContent = `Version ${info.version} disponible.`;
        return installUpdate(info.version);
      }
      if (!auto) $('update-status').textContent = `Vous avez la dernière version (${info.current}).`;
    } catch (err) {
      uiLog('warn', `Mise à jour non vérifiée : ${errMsg(err)}`);
      if (!auto) $('update-status').textContent = 'Vérification impossible pour le moment (serveur de mises à jour injoignable ou aucune version publiée). Réessayez plus tard.';
    }
    return null;
  }
  $('btn-check-update').addEventListener('click', () => checkUpdate(false));

  // ---------------------------------------------------------------------------
  // Console
  // ---------------------------------------------------------------------------
  window.addEventListener('keydown', (e) => {
    if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.key.toUpperCase() === 'I')) {
      e.preventDefault();
      invoke('toggle_devtools').catch(() => {});
    }
  });
  $('btn-devtools').addEventListener('click', () => invoke('toggle_devtools').catch(() => {}));
  $('btn-clear-logs').addEventListener('click', async () => {
    $('log-output').replaceChildren();
    await invoke('clear_logs').catch(() => {});
  });
  $('btn-copy-logs').addEventListener('click', async () => {
    const text = await invoke('export_logs_text').catch(() => $('log-output').textContent);
    try {
      await navigator.clipboard.writeText(text);
      toast('Journal copié.');
    } catch (_) {
      toast('Copie impossible.');
    }
  });

  // ---------------------------------------------------------------------------
  // Démarrage
  // ---------------------------------------------------------------------------
  async function listenAll() {
    if (!tauriEvent) return;
    const on = (name, fn) => tauriEvent.listen(name, fn).catch((err) => uiLog('error', `Écoute ${name} impossible : ${errMsg(err)}`));
    await Promise.all([
      on('launcher://log', (e) => appendLogLine(e.payload)),
      on('launcher://install-progress', (e) => onInstallProgress(e.payload)),
      on('launcher://game-exit', () => onGameExit()),
      on('launcher://launch-result', (e) => onLaunchResult(e.payload)),
      on('launcher://update-error', (e) => {
        $('update-overlay').classList.add('hidden');
        toast(`Mise à jour impossible : ${e.payload}`);
      }),
      on('launcher://update-progress', (e) => {
        const p = e.payload || {};
        const pct = p.total ? Math.round((p.downloaded / p.total) * 100) : 0;
        $('update-fill').style.width = `${pct}%`;
      }),
    ]);
  }

  async function bootstrap() {
    await listenAll();
    try {
      const app = window.__TAURI__ && window.__TAURI__.app;
      if (app) $('launcher-version').textContent = `Launcher ${await app.getVersion()}`;
    } catch (_) {}
    (await invoke('get_logs').catch(() => [])).forEach(appendLogLine);

    try {
      const gs = await invoke('game_status');
      state.gameRunning = !!(gs && gs.running);
    } catch (_) {}
    renderServers();
    renderChannel();
    const restored = await invoke('azuriom_restore_session', { fallbackSiteUrl: 'https://geoventure.fr/' }).catch(() => null);
    renderAccount(restored && restored.account ? restored.account : null);

    await Promise.all([loadConfig(), loadSystem()]);
    loadStatuses();
    loadNotifications();
    loadGameEvents();
    loadAchievements();
    checkUpdate(true);

    setInterval(renderGameEvents, 1000);
    setInterval(loadStatuses, 60000);
    setInterval(loadNotifications, 5 * 60000);
    setInterval(loadGameEvents, 5 * 60000);
  }

  window.addEventListener('error', (e) => uiLog('error', `Erreur de l’interface : ${e.message}`));
  window.addEventListener('unhandledrejection', (e) => uiLog('error', `Erreur de l’interface : ${errMsg(e.reason)}`));
  bootstrap().catch((err) => {
    uiLog('error', `Démarrage de l’interface : ${errMsg(err)}`);
    toast(`Erreur au démarrage : ${errMsg(err)}`);
  });
})();
