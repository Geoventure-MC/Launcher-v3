/**
 * @author Luuxis
 * Licensed under CC BY-NC 4.0
 * https://creativecommons.org/licenses/by-nc/4.0/
 *
 * Edited by CentralCorp Team
 */
const { app, ipcMain, nativeTheme, dialog, shell, clipboard } = require('electron');
const { autoUpdater } = require('electron-updater')

const path = require('path');
const fs = require('fs');

const UpdateWindow = require("./assets/js/windows/updateWindow.js");
const MainWindow = require("./assets/js/windows/mainWindow.js");

let data
let dev = process.env.NODE_ENV === 'dev';

if (dev) {
    let appPath = path.resolve('./AppData/Launcher').replace(/\\/g, '/');
    if (!fs.existsSync(appPath)) fs.mkdirSync(appPath, { recursive: true });
    app.setPath('userData', appPath);
}

// Verrou d'instance unique AVANT le garde-fou de démarrage : un second lancement ne doit ni compter
// un « échec » (ce qui finissait par couper l'accélération GPU pour de bon) ni effacer le cache GPU
// de l'instance déjà ouverte. Il remet simplement la fenêtre existante au premier plan.
const gotLock = app.requestSingleInstanceLock();
if (!gotLock) {
    app.quit();
} else {
    // Évite la fenêtre noire (cache/pilote GPU) : voir utils/startup-guard.js. Avant app.whenReady().
    if (!dev) require('./assets/js/utils/startup-guard.js').install(app);
    app.on('second-instance', () => {
        const win = MainWindow.getWindow() || UpdateWindow.getWindow();
        if (!win || win.isDestroyed()) return;
        if (win.isMinimized()) win.restore();
        if (!win.isVisible()) win.show();
        win.focus();
    });
    require('./assets/js/utils/window-guard.js').install(app);
    app.whenReady().then(() => {
        if (dev) return MainWindow.createWindow()
        UpdateWindow.createWindow()
    });
}

// Fenêtre vivante ou null : un message IPC reçu pendant la fermeture ou l'ouverture d'une fenêtre ne doit
// pas lever une TypeError dans le processus principal (boîte d'erreur Electron).
const live = (w) => (w && !w.isDestroyed() ? w : null);
const main = () => live(MainWindow.getWindow());
const upd = () => live(UpdateWindow.getWindow());

// Canaux de la fenêtre de mise à jour isolée (sandbox) : init, config du panel, ouverture du navigateur.
require('./assets/js/utils/splash-ipc.js').install({
    ipcMain, shell,
    pkg: require('../package.json'),
    translationsDir: path.join(__dirname, 'assets', 'translations'),
    openExternalSafe: require('./assets/js/utils/window-guard.js').openExternalSafe,
});

ipcMain.on('update-window-close', () => UpdateWindow.destroyWindow())
ipcMain.on('update-window-dev-tools', () => { const w = upd(); if (w) w.webContents.openDevTools() })
// Barre de progression de l'icône pendant la mise à jour (envoyée par index.js : jusqu'ici sans écouteur).
ipcMain.on('update-window-progress-load', () => { const w = upd(); if (w) w.setProgressBar(2, { mode: 'indeterminate' }) })
ipcMain.on('update-window-progress', (event, options) => {
    const w = upd();
    const ratio = options && Number(options.progress) / Number(options.size);
    if (w && Number.isFinite(ratio)) w.setProgressBar(Math.min(Math.max(ratio, 0), 1));
})
ipcMain.on('main-window-open', () => MainWindow.createWindow())
ipcMain.on('main-window-dev-tools', () => { const w = main(); if (w) w.webContents.openDevTools() })
ipcMain.on('main-window-close', () => MainWindow.destroyWindow())
ipcMain.on('main-window-progress', (event, options) => {
    const w = main();
    const ratio = options && Number(options.DL) / Number(options.totDL);
    if (w && Number.isFinite(ratio)) w.setProgressBar(Math.min(Math.max(ratio, 0), 1));
})
ipcMain.on('main-window-progress-reset', () => { const w = main(); if (w) w.setProgressBar(0) })
ipcMain.on('main-window-minimize', () => { const w = main(); if (w) w.minimize() })

ipcMain.on('main-window-maximize', () => {
    const w = main();
    if (!w) return;
    if (w.isMaximized()) {
        w.unmaximize();
    } else {
        w.maximize();
    }
})

ipcMain.on('main-window-hide', () => { const w = main(); if (w) w.hide() })
ipcMain.on('main-window-show', () => { const w = main(); if (w) w.show() })
ipcMain.on('main-window-reload', () => { const w = main(); if (w) w.webContents.reload() })

// Presse-papiers écrit par le processus principal : sous Electron 45, `require('electron').clipboard` n'est plus
// fiable dans la fenêtre (le texte n'arrivait pas, le joueur gardait son ancien presse-papiers).
ipcMain.handle('clipboard-write', (_, text) => {
    clipboard.writeText(String(text ?? '').slice(0, 2_000_000));
    return true;
});

ipcMain.handle('is-dark-theme', (_, theme) => {
    if (theme === 'dark') return true
    if (theme === 'light') return false
    return nativeTheme.shouldUseDarkColors;
})

// Save-dialog for the in-app game console "Export" button. Returns the chosen
// file path (or null if cancelled); the renderer writes the file itself.
ipcMain.handle('save-logs-dialog', async (_, defaultName) => {
    const win = MainWindow.getWindow();
    const res = await dialog.showSaveDialog(win, {
        defaultPath: (typeof defaultName === 'string' && /^[\w.-]{1,80}$/.test(defaultName)) ? defaultName : `minecraft-logs-${Date.now()}.txt`,
        filters: [{ name: 'Text', extensions: ['txt'] }],
    });
    return res && !res.canceled ? res.filePath : null;
})

// Sélecteur natif sécurisé de binaire Java (javaw.exe / java.exe / java).
// Valide dans le processus principal que la cible est un exécutable Java régulier.
ipcMain.handle('select-java-dialog', async () => {
    const win = MainWindow.getWindow();
    const isWin = process.platform === 'win32';
    const filters = isWin
        ? [{ name: 'Exécutable Java (javaw.exe / java.exe)', extensions: ['exe'] }]
        : [];
    const defaultDir = isWin
        ? [process.env.ProgramFiles, 'C:\\Program Files\\Java'].find(d => d && fs.existsSync(d))
        : ['/usr/lib/jvm', '/Library/Java/JavaVirtualMachines'].find(d => fs.existsSync(d));
    const res = await dialog.showOpenDialog(win, {
        title: 'Sélectionner un exécutable Java (JDK 17 recommandé)',
        defaultPath: defaultDir,
        properties: ['openFile'],
        filters,
    });
    if (!res || res.canceled || !Array.isArray(res.filePaths) || res.filePaths.length === 0) {
        return { canceled: true };
    }
    const rawPath = String(res.filePaths[0] || '').trim();
    const resolved = path.resolve(rawPath);
    const base = path.basename(resolved);
    if (!/^javaw?(\.exe)?$/i.test(base) || rawPath.startsWith('\\\\') || rawPath.startsWith('//')) {
        return { canceled: false, error: 'invalid_java_binary' };
    }
    try {
        if (!fs.statSync(resolved).isFile()) {
            return { canceled: false, error: 'invalid_java_binary' };
        }
    } catch {
        return { canceled: false, error: 'invalid_java_binary' };
    }
    return { canceled: false, path: resolved };
})

// GPU (informations de base) pour les profils de performance et le diagnostic. Jamais bloquant : délai de 3 s.
ipcMain.handle('get-gpu-info', async () => {
    try {
        const info = await Promise.race([
            app.getGPUInfo('basic'),
            new Promise((_, rej) => setTimeout(() => rej(new Error('timeout')), 3000)),
        ]);
        const devs = Array.isArray(info && info.gpuDevice) ? info.gpuDevice : [];
        const dev = devs.find(d => d.active) || devs[0] || {};
        const aux = (info && info.auxAttributes) || {};
        const hex = n => (typeof n === 'number' ? '0x' + n.toString(16).padStart(4, '0') : null);
        const name = aux.glRenderer || [dev.driverVendor, dev.vendorString, dev.deviceString].filter(Boolean).join(' ')
            || (dev.vendorId != null ? `vendor ${hex(dev.vendorId)} device ${hex(dev.deviceId)}` : '');
        return { name: name || null, count: devs.length };
    } catch (e) {
        return { name: null, count: 0, error: String(e && e.message || e) };
    }
})

app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') app.quit();
});

// Passage au nouveau launcher (Rust) : voir assets/js/windows/rustSwitch.js. L'ancien ne se ferme que
// si le nouveau a bien été ouvert ; sinon il démarre comme avant.
ipcMain.handle('rust-switch', async (event, opts) => {
    if (dev) return { ok: false, error: 'développement' };
    const o = opts && typeof opts === 'object' ? opts : {};
    const send = (s) => { try { event.sender.send('rust-switch-status', s); } catch { /* fenêtre fermée */ } };
    const res = await require('./assets/js/windows/rustSwitch.js').switchToRust({
        account: o.account && typeof o.account === 'object' ? o.account : null,
        siteUrl: typeof o.siteUrl === 'string' ? o.siteUrl : null,
        onStatus: send,
    });
    if (res.ok) setTimeout(() => app.quit(), 1500);
    return res;
});

autoUpdater.autoDownload = false;

ipcMain.handle('update-app', async () => {
    return await new Promise((resolve, reject) => {
        autoUpdater.checkForUpdates().then(res => {
            resolve(res);
        }).catch(error => {
            // Reject with a real Error so the message serializes as a readable
            // string across IPC (otherwise the renderer sees "[object Object]").
            reject(error instanceof Error
                ? error
                : new Error(String(error && error.message ? error.message : error)));
        })
    })
})

autoUpdater.on('update-available', () => {
    const updateWindow = UpdateWindow.getWindow();
    if (updateWindow) updateWindow.webContents.send('updateAvailable');
});

ipcMain.on('start-update', () => {
    autoUpdater.downloadUpdate();
})

autoUpdater.on('update-not-available', () => {
    const updateWindow = UpdateWindow.getWindow();
    if (updateWindow) updateWindow.webContents.send('update-not-available');
});

autoUpdater.on('update-downloaded', () => {
    autoUpdater.quitAndInstall();
});

autoUpdater.on('download-progress', (progress) => {
    const updateWindow = UpdateWindow.getWindow();
    if (updateWindow) updateWindow.webContents.send('download-progress', progress);
})
autoUpdater.on('error', (err) => {
    const updateWindow = UpdateWindow.getWindow();
    if (updateWindow) updateWindow.webContents.send('error', err);
});