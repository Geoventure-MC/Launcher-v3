/**
 * Preload de la fenêtre de mise à jour (sandbox + contextIsolation, sans Node dans la page).
 * La page ne reçoit QUE les fonctions ci-dessous : pas d'ipcRenderer, pas de require, pas de canal libre.
 * Liste blanche des canaux IPC : voir utils/splash-ipc.js et app.js.
 */
'use strict';
const { contextBridge, ipcRenderer } = require('electron');

// Écoute d'un évènement du processus principal : la page ne reçoit que la donnée mappée, jamais l'évènement IPC.
const listen = (channel, map) => (cb) => {
    if (typeof cb !== 'function') return;
    ipcRenderer.on(channel, (_event, payload) => cb(map(payload)));
};

contextBridge.exposeInMainWorld('nexusUpdate', {
    // Données statiques : plateforme, dépôt des releases, traductions
    init: (lang) => ipcRenderer.invoke('splash-init', String(lang || '')),
    // Configuration du panel (récupérée par le processus principal)
    fetchConfig: (opts) => {
        const o = opts && typeof opts === 'object' ? opts : {};
        return ipcRenderer.invoke('splash-config', {
            serverUrl: typeof o.serverUrl === 'string' ? o.serverUrl : null,
            instance: typeof o.instance === 'string' ? o.instance : null,
            channel: typeof o.channel === 'string' ? o.channel : null,
        });
    },
    // Mise à jour
    checkForUpdate: () => ipcRenderer.invoke('update-app').then(() => undefined),
    startUpdate: () => ipcRenderer.send('start-update'),
    onUpdateAvailable: listen('updateAvailable', () => undefined),
    onUpdateNotAvailable: listen('update-not-available', () => undefined),
    onUpdateError: listen('error', (err) => (err ? { message: String(err.message || err) } : null)),
    onDownloadProgress: listen('download-progress', (p) => ({
        transferred: Number(p && p.transferred),
        total: Number(p && p.total),
    })),
    // Fenêtres et barre des tâches
    progressLoad: () => ipcRenderer.send('update-window-progress-load'),
    progress: (progress, size) => ipcRenderer.send('update-window-progress', { progress: Number(progress), size: Number(size) }),
    openMainWindow: () => ipcRenderer.send('main-window-open'),
    closeWindow: () => ipcRenderer.send('update-window-close'),
    devTools: () => ipcRenderer.send('update-window-dev-tools'),
    openExternal: (url) => ipcRenderer.send('open-external', String(url)),
    // Passage au nouveau launcher (Rust)
    switchToRust: (opts) => {
        const o = opts && typeof opts === 'object' ? opts : {};
        return ipcRenderer.invoke('rust-switch', { account: o.account || null, siteUrl: typeof o.siteUrl === 'string' ? o.siteUrl : null });
    },
    onSwitchStatus: listen('rust-switch-status', (s) => ({
        text: String((s && s.text) || ''),
        got: Number(s && s.got) || 0,
        total: Number(s && s.total) || 0,
    })),
});
