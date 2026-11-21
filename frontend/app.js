(() => {
  'use strict';

  const tauriCore = window.__TAURI__ && window.__TAURI__.core;
  const tauriEvent = window.__TAURI__ && window.__TAURI__.event;
  const invoke = tauriCore
    ? (cmd, args) => tauriCore.invoke(cmd, args)
    : async (cmd) => {
        throw new Error(`Pont Tauri indisponible pour ${cmd} (ouvrir via "npm run dev")`);
      };

  const state = {
    config: null,
    account: null,
    java: null,
    system: null,
    launching: false,
  };

  const els = {
    statusBadge: document.getElementById('status-badge'),
    instanceSelect: document.getElementById('instance-select'),
    channelSelect: document.getElementById('channel-select'),
    btnRefresh: document.getElementById('btn-refresh'),
    btnDevtools: document.getElementById('btn-devtools'),
    azAccountBox: document.getElementById('az-account-box'),
    azAvatar: document.getElementById('az-avatar'),
    azUsername: document.getElementById('az-username'),
    azRole: document.getElementById('az-role'),
    btnLogout: document.getElementById('btn-logout'),
    azForm: document.getElementById('az-login-form'),
    azEmail: document.getElementById('az-email'),
    azPassword: document.getElementById('az-password'),
    az2faWrap: document.getElementById('az-2fa-wrap'),
    az2fa: document.getElementById('az-2fa'),
    azStatus: document.getElementById('az-status'),
    infoVersion: document.getElementById('info-version'),
    infoJava: document.getElementById('info-java'),
    infoRam: document.getElementById('info-ram'),
    progressFill: document.getElementById('progress-fill'),
    progressText: document.getElementById('progress-text'),
    btnSync: document.getElementById('btn-sync'),
    btnPlay: document.getElementById('btn-play'),
    logOutput: document.getElementById('log-output'),
    btnCopyLogs: document.getElementById('btn-copy-logs'),
    btnClearLogs: document.getElementById('btn-clear-logs'),
  };

  function appendLogLine(entry) {
    const line = `[${entry.timestamp}] [${String(entry.level).toUpperCase()}] [${entry.source}] ${entry.message}\n`;
    els.logOutput.textContent += line;
    els.logOutput.scrollTop = els.logOutput.scrollHeight;
  }

  function setBadge(text, cls) {
    els.statusBadge.textContent = text;
    els.statusBadge.className = `badge ${cls}`;
  }

  function updatePlayButton() {
    const ready = !!(
      !state.launching &&
      state.config &&
      !state.config.maintenance &&
      state.account &&
      state.java
    );
    els.btnPlay.disabled = !ready;
  }

  function renderAccount(account) {
    state.account = account;
    if (account) {
      els.azAccountBox.classList.remove('hidden');
      els.azForm.classList.add('hidden');
      els.azUsername.textContent = account.username;
      els.azRole.textContent = account.role ? account.role.name : 'Membre Azuriom';
      if (account.skin_url) {
        els.azAvatar.src = account.skin_url;
      }
      els.azStatus.textContent = '';
    } else {
      els.azAccountBox.classList.add('hidden');
      els.azForm.classList.remove('hidden');
      els.az2faWrap.classList.add('hidden');
    }
    updatePlayButton();
  }

  // Raccourci F12 / Ctrl+Shift+I garanti vers les DevTools Tauri v2
  window.addEventListener('keydown', (e) => {
    if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.key.toUpperCase() === 'I')) {
      e.preventDefault();
      invoke('toggle_devtools').catch(() => {});
    }
  });
  els.btnDevtools.addEventListener('click', () => invoke('toggle_devtools').catch(() => {}));

  async function loadConfig() {
    setBadge('Chargement…', '');
    try {
      const res = await invoke('fetch_panel_config', {
        instance: els.instanceSelect.value,
        channel: els.channelSelect.value,
      });
      state.config = res.config;
      const loader = res.config.loader && res.config.loader.build
        ? `${res.config.loader.type} ${res.config.loader.build}`
        : 'vanilla';
      els.infoVersion.textContent = `${res.config.game_version || '1.20.1'} (${loader})`;
      if (res.offline) {
        setBadge('Mode hors-ligne (cache)', 'badge-offline');
      } else if (res.config.maintenance) {
        setBadge('Maintenance', 'badge-offline');
      } else {
        setBadge('En ligne', 'badge-online');
      }
      updatePlayButton();
    } catch (err) {
      setBadge('Hors-ligne', 'badge-error');
      appendLogLine({
        timestamp: new Date().toTimeString().slice(0, 8),
        level: 'error',
        source: 'ui',
        message: `Panel injoignable : ${err.message || err}`,
      });
    }
  }

  async function loadHardwareAndJava() {
    try {
      const sys = await invoke('get_system_overview', { profile: null });
      state.system = sys;
      const maxRam = sys.plan.ram_max_gb ? `${sys.plan.ram_max_gb} Go` : 'RAM insuffisante';
      els.infoRam.textContent = `${sys.total_ram_gb} Go (${sys.cpu_cores} cœurs) · Profil ${sys.recommended_profile} (${maxRam})`;
    } catch (_) {}

    try {
      const jdk = await invoke('resolve_preferred_java', {
        customPath: null,
        dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus',
      });
      state.java = jdk;
      els.infoJava.textContent = jdk.label;
    } catch (err) {
      state.java = null;
      els.infoJava.textContent = 'JDK 17 introuvable';
      appendLogLine({
        timestamp: new Date().toTimeString().slice(0, 8),
        level: 'warn',
        source: 'java',
        message: err.message || String(err),
      });
    }
    updatePlayButton();
  }

  async function restoreAzuriomSession() {
    const fallbackSite = (state.config && state.config.azauth) || 'https://geoventure.fr/';
    try {
      const res = await invoke('azuriom_restore_session', { fallbackSiteUrl: fallbackSite });
      if (res && res.account) {
        renderAccount(res.account);
      }
    } catch (_) {}
  }

  els.azForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    els.azStatus.textContent = 'Authentification Azuriom…';
    const siteUrl = (state.config && state.config.azauth) || 'https://geoventure.fr/';
    try {
      const res = await invoke('azuriom_login', {
        siteUrl,
        email: els.azEmail.value,
        password: els.azPassword.value,
        code2fa: els.az2fa.value || null,
      });
      if (res.outcome === 'requires2fa') {
        els.az2faWrap.classList.remove('hidden');
        els.azStatus.textContent = 'Entrez votre code 2FA Azuriom puis validez.';
        els.az2fa.focus();
      } else if (res.outcome === 'authenticated') {
        els.azPassword.value = '';
        els.az2fa.value = '';
        renderAccount(res.account);
      } else {
        els.azStatus.textContent = `Refusé : ${res.message || res.reason}`;
      }
    } catch (err) {
      els.azStatus.textContent = `Erreur : ${err.message || err}`;
    }
  });

  els.btnLogout.addEventListener('click', async () => {
    await invoke('azuriom_logout').catch(() => {});
    renderAccount(null);
  });

  async function runSync() {
    els.progressFill.style.width = '0%';
    els.progressText.textContent = 'Récupération du manifeste…';
    const manifest = await invoke('fetch_modpack_manifest', {
      instance: els.instanceSelect.value,
      channel: els.channelSelect.value,
    });
    const report = await invoke('sync_modpack_files', {
      manifest,
      dataDirectory: (state.config && state.config.dataDirectory) || 'Nexus',
      instance: els.instanceSelect.value,
      channel: els.channelSelect.value,
      ignored: (state.config && state.config.ignored) || [],
      concurrency: 12,
    });
    els.progressFill.style.width = '100%';
    els.progressText.textContent = `Sync : ${report.valid_count} OK, ${report.downloaded_count} téléchargés, ${report.failed.length} erreurs`;
    return report;
  }

  els.btnSync.addEventListener('click', async () => {
    els.btnSync.disabled = true;
    try {
      await runSync();
    } catch (err) {
      els.progressText.textContent = `Erreur sync : ${err.message || err}`;
    } finally {
      els.btnSync.disabled = false;
    }
  });

  els.btnPlay.addEventListener('click', async () => {
    if (!state.config || !state.account) return;
    state.launching = true;
    updatePlayButton();
    try {
      await runSync();
      els.progressText.textContent = 'Lancement de Minecraft Forge 1.20.1…';
      const plan = state.system && state.system.plan ? state.system.plan : {};
      const ramMin = plan.ram_min_gb || state.config.ram_min || 3;
      const ramMax = plan.ram_max_gb || state.config.ram_max || 6;
      const status = state.config.status || {};
      const portNum = status.port ? Number(status.port) : null;

      const pid = await invoke('launch_instance', {
        req: {
          data_directory: state.config.dataDirectory || 'Nexus',
          instance: els.instanceSelect.value,
          channel: els.channelSelect.value,
          mc_version: state.config.game_version || '1.20.1',
          loader_build: (state.config.loader && state.config.loader.build) || '1.20.1-47.4.23',
          custom_java_path: state.java ? state.java.path : null,
          username: state.account.username,
          uuid: state.account.uuid,
          access_token: state.account.access_token,
          ram_min_gb: ramMin,
          ram_max_gb: ramMax,
          extra_jvm_args: plan.jvm_args || [],
          server_host: status.ip || null,
          server_port: Number.isFinite(portNum) ? portNum : null,
          width: null,
          height: null,
        },
      });
      els.progressText.textContent = `Minecraft démarré (PID ${pid}) !`;
    } catch (err) {
      els.progressText.textContent = `Erreur lancement : ${err.message || err}`;
      appendLogLine({
        timestamp: new Date().toTimeString().slice(0, 8),
        level: 'error',
        source: 'launch',
        message: err.message || String(err),
      });
    } finally {
      state.launching = false;
      updatePlayButton();
    }
  });

  els.btnRefresh.addEventListener('click', () => loadConfig());
  els.instanceSelect.addEventListener('change', () => loadConfig());
  els.channelSelect.addEventListener('change', () => loadConfig());

  els.btnClearLogs.addEventListener('click', async () => {
    els.logOutput.textContent = '';
    await invoke('clear_logs').catch(() => {});
  });

  els.btnCopyLogs.addEventListener('click', async () => {
    const text = await invoke('export_logs_text').catch(() => els.logOutput.textContent);
    if (navigator.clipboard) await navigator.clipboard.writeText(text).catch(() => {});
  });

  async function bootstrap() {
    if (tauriEvent) {
      await tauriEvent.listen('launcher://log', (e) => appendLogLine(e.payload));
      await tauriEvent.listen('launcher://download-progress', (e) => {
        const p = e.payload;
        const pct = p.total > 0 ? Math.round((p.completed / p.total) * 100) : 0;
        els.progressFill.style.width = `${pct}%`;
        els.progressText.textContent = `${p.completed}/${p.total} — ${p.current_file}`;
      });
    }
    const existingLogs = await invoke('get_logs').catch(() => []);
    existingLogs.forEach(appendLogLine);
    await loadConfig();
    await Promise.all([loadHardwareAndJava(), restoreAzuriomSession()]);
  }

  bootstrap();
})();
