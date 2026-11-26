// Faux pont Tauri pour tester l'interface dans un navigateur : chaque commande renvoie
// une réponse prévue (window.__MOCK__), les appels sont notés dans window.__CALLS__ et
// window.__emit(nom, données) simule un événement venu du launcher.
(() => {
  const listeners = {};
  window.__CALLS__ = [];
  const plan = (id) => ({ id, ram_min_gb: { eco: 2, balanced: 3, performance: 4 }[id], ram_max_gb: { eco: 4, balanced: 6, performance: 8 }[id], jvm_args: ['-XX:+UseG1GC'], render_distance: 12, cap_gb: 9, insufficient: false, warnings: [] });
  const summary = (key, username, extra) => ({ key, id: 1, username, skin_url: null, site_url: 'https://geoventure.fr/', role: null, active: false, needs_login: false, ...extra });
  const fullAccount = (s) => ({ id: s.id, username: s.username, uuid: `uuid-${s.username}`, access_token: `jeton-${s.username}`, email_verified: true, money: 10, banned: false, role: s.role, site_url: s.site_url, skin_url: s.skin_url });
  const accountState = () => (window.__ACC__ = window.__ACC__ || window.__MOCK__.accounts || (window.__MOCK__.loggedIn === false
    ? { accounts: [], active: null }
    : { accounts: [summary('geoventure.fr#1', 'Testeur', { active: true })], active: 'geoventure.fr#1' }));
  const accountsView = () => JSON.parse(JSON.stringify(accountState()));
  const defaults = {
    fetch_panel_config: () => ({ offline: false, config: {
      dataDirectory: 'Nexus', game_version: '1.20.1', loader: { enable: true, type: 'forge', build: '1.20.1-47.4.23' },
      maintenance: false, status: { ip: 'play.example', port: 25565 }, servers: [], ignored: [],
      rpc_activation: true, rpc_id: '1517422035033395230', rpc_state: 'Prépare son aventure',
      video_activate: true, video_url: 'dQw4w9WgXcQ', splash: 'Fonde ton pays !', splash_author: 'Équipe', money: true, role: true,
      role_data: { role1: { name: 'Joueur', background: 'https://launcher.example/storage/bg-joueur.jpg' } },
      servers: [{ id: 'geoventure', name: 'Geoventure', icon: 'https://launcher.example/storage/geo.png' }],
    } }),
    azuriom_restore_session: () => (window.__MOCK__.accounts ? { verified_online: true, account: (() => { const st = accountState(); const a = st.accounts.find((x) => x.key === st.active); return a ? fullAccount(a) : null; })() } : window.__MOCK__.loggedIn === false ? null : { verified_online: true, account: {
      id: 1, username: 'Testeur', uuid: '00000000-0000-0000-0000-000000000001', access_token: 'jeton-secret',
      email_verified: true, money: 1234, banned: false, role: { name: 'Joueur', color: '#4ade80' }, site_url: 'https://geoventure.fr/', skin_url: null,
    } }),
    // Comptes enregistrés : liste modifiable (window.__MOCK__.accounts = { accounts: [...], active }).
    azuriom_list_accounts: () => accountsView(),
    azuriom_switch_account: ({ key }) => {
      const st = accountState();
      const acc = st.accounts.find((a) => a.key === key);
      if (!acc) return new Error('Compte inconnu.');
      if (window.__MOCK__.refusedKeys && window.__MOCK__.refusedKeys.includes(key)) {
        acc.needs_login = true;
        if (st.active === key) st.active = null;
        return { account: null, verified_online: false, reconnect_required: true, accounts: accountsView() };
      }
      st.active = key;
      st.accounts.forEach((a) => { a.active = a.key === key; });
      acc.needs_login = false;
      return { account: fullAccount(acc), verified_online: true, reconnect_required: false, accounts: accountsView() };
    },
    azuriom_remove_account: ({ key }) => {
      const st = accountState();
      st.accounts = st.accounts.filter((a) => a.key !== key);
      if (st.active === key) st.active = null;
      return accountsView();
    },
    azuriom_logout: () => {
      const st = accountState();
      st.accounts = st.accounts.filter((a) => a.key !== st.active);
      st.active = null;
      return accountsView();
    },
    azuriom_login: ({ siteUrl, email }) => {
      const st = accountState();
      const key = `${siteUrl.replace(/^https?:\/\//, '').replace(/\/$/, '')}#${email}`;
      st.accounts.forEach((a) => { a.active = false; });
      const existing = st.accounts.find((a) => a.key === key);
      const acc = existing || summary(key, email, { site_url: siteUrl });
      acc.active = true; acc.needs_login = false;
      if (!existing) st.accounts.push(acc);
      st.active = key;
      return { outcome: 'authenticated', account: fullAccount(acc) };
    },
    get_system_overview: (a) => ({ total_ram_gb: 16, free_ram_gb: 12, cpu_cores: 8, cpu_name: 'CPU test', os: 'Test OS', recommended_profile: 'balanced', plan: plan(a && a.profile || 'balanced') }),
    game_status: () => ({ launching: false, running: false }),
    get_logs: () => [],
    fetch_news: () => [{ title: 'Article de test', link: 'https://geoventure.fr/news/1', description: 'Résumé', image: null, published: '2026-11-21T10:00:00Z', author: 'equipe' }],
    panel_get: (a) => ({
      'utils/servers-history': { points: [{ t: 1, players: 2 }, { t: 2, players: 5 }, { t: 3, players: 1 }], peakHours: [20, 21] },
      'utils/wonder': { current: { name: 'Merveille test', theme: 'Ponts', phase: 'open', closesAt: Date.now() + 3600e3, ranking: [] }, past: [] },
      'utils/community-mods': [
        { name: 'Oculus (Shaders)', description: 'Shaders', category: 'Graphismes', author: 'Iris', version: '1.8.0', url: 'https://cdn.example/oculus.jar', filename: 'oculus.jar' },
        { name: 'Tweakerge', description: 'Outils', category: 'Utilitaires', url: 'https://cdn.example/tweak.jar' },
      ],
    }[a.endpoint] ?? []),
    community_mods_installed: () => [],
    tray_settings: () => ({ tray: true, close_to_tray: false, autostart: false }),
    set_tray_settings: () => ({ tray: true, close_to_tray: false, autostart: false }),
    detect_gpus: () => ({ gpus: ['GPU test'], only_integrated: false }),
    check_launcher_update: () => ({ current: '2.0.9', available: false }),
    launch_instance: () => { setTimeout(() => window.__emit('launcher://launch-result', { ok: true, pid: 4242 }), 50); return null; },
    kill_game: () => true,
    list_content: () => ({ entries: [], shader_loader: false, shaders_enabled: false, shader_pack: null }),
    detect_jdks: () => [],
    get_panel_url: () => ({ current: 'https://launcher.geoventure.fr/', default: 'https://launcher.geoventure.fr/' }),
    optional_mods_state: () => ({}),
  };
  window.__MOCK__ = window.__MOCK__ || {};
  window.__emit = (name, payload) => (listeners[name] || []).forEach((fn) => fn({ event: name, payload }));
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        window.__CALLS__.push({ cmd, args });
        const h = window.__MOCK__[cmd] || defaults[cmd];
        if (!h) return null;
        const r = h(args);
        if (r instanceof Error) throw r;
        return r;
      },
      convertFileSrc: (p) => `asset://localhost/${encodeURIComponent(p)}`,
    },
    event: { listen: async (name, fn) => { (listeners[name] = listeners[name] || []).push(fn); return () => {}; } },
    app: { getVersion: async () => window.__MOCK__.version || '2.0.9' },
  };
})();
