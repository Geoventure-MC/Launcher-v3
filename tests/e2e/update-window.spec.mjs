import { test, expect, _electron as electron } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { startMock } from './mock-panel.mjs';
import { launchLauncher, panelActive, ROOT } from './helpers.mjs';

// Fenêtre de mise à jour (affichée au vrai démarrage, hors mode dev) : isolée, sans Node, et le démarrage
// doit toujours aboutir à la fenêtre principale. Les autres tests tournent en NODE_ENV=dev (fenêtre principale seule).
let mock;
test.beforeAll(async () => { mock = await startMock(); });
test.afterAll(async () => { await mock.close(); });

// Lance l'application comme un joueur : sans NODE_ENV=dev, donc fenêtre de mise à jour d'abord. Le profil
// (localStorage : adresse du faux panel) est préparé par un premier lancement en mode dev, même dossier.
async function launchAsPlayer() {
  mock.setMode('ok');
  const L = await launchLauncher(mock, { account: false, instance: 'geoventure' });
  await L.app.close().catch(() => {});
  const env = { ...process.env, HOME: L.home, APPDATA: '', NO_PROXY: '127.0.0.1,localhost', no_proxy: '127.0.0.1,localhost' };
  delete env.NODE_ENV; delete env.DEV_TOOL; delete env.HTTPS_PROXY; delete env.https_proxy; delete env.HTTP_PROXY; delete env.http_proxy;
  const app = await electron.launch({
    executablePath: path.join(ROOT, 'node_modules/electron/dist/electron'),
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', `--user-data-dir=${path.join(L.tmp, 'AppData', 'Launcher')}`, ROOT],
    cwd: L.tmp, env,
  });
  return { app, tmp: L.tmp, close: async () => { await app.close().catch(() => {}); fs.rmSync(L.tmp, { recursive: true, force: true }); } };
}

test.describe('fenêtre de mise à jour', () => {
  test('isolée (sans Node) et le démarrage aboutit à la fenêtre principale', async () => {
    const P = await launchAsPlayer();
    mock.clearHits();
    try {
      const win = await P.app.firstWindow();
      expect(win.url()).toContain('index.html');
      await win.waitForLoadState('domcontentloaded');
      const iso = await win.evaluate(() => ({
        require: typeof require, process: typeof process, ipc: typeof ipcRenderer,
        api: Object.keys(window.nexusUpdate || {}).sort(),
      }));
      expect(iso.require).toBe('undefined');
      expect(iso.process).toBe('undefined');
      expect(iso.ipc).toBe('undefined');
      expect(iso.api).not.toContain('ipcRenderer');
      expect(iso.api).toContain('openMainWindow');
      // Le texte traduit est affiché, puis la fenêtre principale prend le relais.
      await expect(win.locator('#splash-message')).not.toHaveText('', { timeout: 15000 });
      // Le processus principal a bien interrogé le panel pour la fenêtre isolée (chemin fixe, instance transmise).
      await expect.poll(() => mock.hits.some(h => h.path === '/utils/api' && h.query.includes('instance=geoventure'))).toBeTruthy();
      // Hors paquet, electron-updater ne cherche rien : on simule « aucune mise à jour » depuis le processus principal
      // (renvoyé tant que la page n'écoute pas encore ; le démarrage est idempotent).
      await expect.poll(async () => {
        await P.app.evaluate(({ BrowserWindow }) => { const w = BrowserWindow.getAllWindows().find(x => x.webContents.getURL().includes('index.html')); if (w) w.webContents.send('update-not-available'); }).catch(() => {});
        return P.app.windows().some(w => w.url().includes('launcher.html'));
      }, { timeout: 45000, intervals: [1000] }).toBeTruthy();
      const main = P.app.windows().find(w => w.url().includes('launcher.html'));
      await panelActive(main, 'login');
    } finally { await P.close(); }
  });
});
