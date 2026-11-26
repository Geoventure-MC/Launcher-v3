// Tests de l'interface du launcher Rust : vraie page (frontend/), faux pont Tauri.
// Lancer : node --test tests/ui/
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../../frontend');
const mock = fs.readFileSync(path.join(here, 'mock-tauri.js'), 'utf8');
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.jpg': 'image/jpeg', '.woff2': 'font/woff2', '.ttf': 'font/ttf' };

let server; let browser; let base;
before(async () => {
  server = http.createServer((req, res) => {
    const rel = decodeURIComponent(new URL(req.url, 'http://x').pathname).replace(/^\/+/, '') || 'index.html';
    const file = path.join(root, rel);
    if (!file.startsWith(root) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { 'Content-Type': TYPES[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}/index.html`;
  browser = await chromium.launch();
});
after(async () => { await browser?.close(); server?.close(); });

// Ouvre le launcher avec des réponses de commandes remplacées (code source de fonctions).
async function open({ mockSrc = '', locale = 'fr-FR', settings = null } = {}) {
  const context = await browser.newContext({ locale, viewport: { width: 1280, height: 760 } });
  await context.addInitScript(`window.__MOCK__ = {${mockSrc}};` + (settings ? `localStorage.setItem('nexus_rust_settings', ${JSON.stringify(JSON.stringify(settings))});` : ''));
  await context.addInitScript(mock);
  // Pas d'accès à Internet pendant les tests : images et polices externes bloquées.
  await context.route(/^https?:\/\/(?!127\.0\.0\.1)/, (r) => r.abort());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(base);
  await page.waitForSelector('#preload.done', { state: 'attached', timeout: 10000 });
  return { page, context, errors };
}
const calls = (page, cmd) => page.evaluate((c) => window.__CALLS__.filter((x) => x.cmd === c).map((x) => x.args), cmd);

test('sans session : écran de connexion, bouton Jouer bloqué', async () => {
  const { page, context, errors } = await open({ mockSrc: 'loggedIn: false' });
  assert.equal(await page.isVisible('#login-screen'), true);
  assert.equal(await page.isDisabled('#btn-play'), true);
  assert.deepEqual(errors, []);
  await context.close();
});

test('accueil : affluence, vidéo, présence Discord', async () => {
  const { page, context, errors } = await open();
  assert.equal(await page.isVisible('#login-screen'), false);
  await page.waitForSelector('#affluence-card:not(.hidden)');
  assert.equal(await page.locator('#affluence-bars .affluence-bar').count(), 3);
  assert.match(await page.textContent('#affluence-peak'), /20 h, 21 h/);
  assert.equal(await page.isVisible('#video-card'), true);
  const presence = (await calls(page, 'discord_presence')).at(-1).presence;
  assert.equal(presence.client_id, '1517422035033395230');
  assert.equal(presence.details, 'Dans le launcher Geoventure');
  assert.deepEqual(errors, []);
  await context.close();
});

test('jouer : arguments du lancement, puis « Fermer le jeu » en deux clics', async () => {
  const { page, context, errors } = await open();
  await page.click('#btn-play');
  await page.waitForFunction(() => document.getElementById('play-label').textContent === 'En jeu');
  const [launch] = await calls(page, 'launch_instance');
  assert.equal(launch.req.username, 'Testeur');
  assert.equal(launch.req.mc_version, '1.20.1');
  assert.equal(launch.req.loader_build, '1.20.1-47.4.23');
  assert.equal(launch.req.ram_max_gb, 6);
  assert.equal(launch.req.light_game_options, false);
  assert.equal((await calls(page, 'discord_presence')).at(-1).presence.details, 'En jeu sur Geoventure');
  // Premier clic : avertissement seulement.
  await page.click('#btn-join');
  assert.equal((await calls(page, 'kill_game')).length, 0);
  await page.click('#btn-join');
  assert.equal((await calls(page, 'kill_game')).length, 1);
  assert.deepEqual(errors, []);
  await context.close();
});

test('plantage : cause et conseil affichés', async () => {
  const { page, context, errors } = await open();
  await page.evaluate(() => window.__emit('launcher://game-crash', {
    exit_code: 1, crash_report: null, excerpt: 'java.lang.OutOfMemoryError: Java heap space',
    cause: { id: 'out_of_memory', title: 'Le jeu a manqué de mémoire', advice: 'Augmentez la RAM allouée.' },
  }));
  await page.waitForSelector('#crash-overlay:not(.hidden)');
  assert.match(await page.textContent('#crash-cause'), /manqué de mémoire/);
  assert.match(await page.textContent('#crash-excerpt'), /OutOfMemoryError/);
  assert.deepEqual(errors, []);
  await context.close();
});

test('mods communauté : filtre et installation', async () => {
  const { page, context, errors } = await open();
  await page.click('.nav-btn[data-tab="game"]');
  await page.click('.tab[data-tab="mods"]');
  await page.waitForSelector('#community-mods .cmod');
  assert.equal(await page.locator('#community-mods .cmod').count(), 2);
  await page.click('#community-filters .chip-btn:has-text("Graphismes")');
  assert.equal(await page.locator('#community-mods .cmod').count(), 1);
  await page.click('#community-mods .cmod button:has-text("Installer")');
  await page.waitForSelector('#community-mods .cmod.installed');
  const [inst] = await calls(page, 'install_community_mod');
  assert.equal(inst.fileName, 'oculus.jar');
  assert.equal(inst.url, 'https://cdn.example/oculus.jar');
  assert.deepEqual(errors, []);
  await context.close();
});

test('anglais : interface traduite', async () => {
  const { page, context, errors } = await open({ locale: 'en-GB' });
  assert.equal(await page.textContent('#play-label'), 'Play');
  assert.equal((await page.textContent('#btn-join')).trim(), 'Join the server');
  await page.click('.nav-btn[data-tab="game"]');
  assert.equal((await page.textContent('.tab[data-tab="perf"]')).trim(), '⚡ Performance');
  assert.deepEqual(errors, []);
  await context.close();
});

test('profil Économe : réglages légers demandés seulement avec accord', async () => {
  const { page, context, errors } = await open({ settings: { profile: 'eco', lightGame: true } });
  await page.click('#btn-play');
  await page.waitForFunction(() => window.__CALLS__.some((c) => c.cmd === 'launch_instance'));
  const [launch] = await calls(page, 'launch_instance');
  assert.equal(launch.req.light_game_options, true);
  assert.equal(launch.req.ram_max_gb, 4);
  assert.deepEqual(errors, []);
  await context.close();
});

test('statistiques : rien sans accord, envoi après « D’accord »', async () => {
  const { page, context, errors } = await open();
  await page.waitForSelector('#consent:not(.hidden)', { timeout: 8000 });
  assert.equal((await calls(page, 'send_telemetry')).length, 0);
  await page.click('#consent-yes');
  const [ev] = await calls(page, 'send_telemetry');
  assert.equal(ev.event, 'launch');
  await page.click('#btn-play');
  await page.waitForFunction(() => window.__CALLS__.some((c) => c.cmd === 'send_telemetry' && c.args.event === 'game_launch'));
  assert.deepEqual(errors, []);
  await context.close();
});

test('refus des statistiques : plus jamais demandé, rien envoyé', async () => {
  const { page, context, errors } = await open({ settings: { telemetry: false } });
  await page.click('#btn-play');
  await page.waitForFunction(() => document.getElementById('play-label').textContent === 'En jeu');
  await page.waitForTimeout(4500);
  assert.equal(await page.isVisible('#consent'), false);
  assert.equal((await calls(page, 'send_telemetry')).length, 0);
  assert.deepEqual(errors, []);
  await context.close();
});

test('merveille affichée sur l’accueil', async () => {
  const { page, context, errors } = await open();
  await page.waitForSelector('#wonder-card:not(.hidden)');
  assert.equal(await page.textContent('#wonder-name'), 'Merveille test');
  assert.match(await page.textContent('#wonder-info'), /Ponts · Constructions en cours · fin dans/);
  assert.deepEqual(errors, []);
  await context.close();
});

test('réglages du panel : logo, fond du grade, auteur de la citation, argent', async () => {
  const { page, context, errors } = await open();
  assert.equal(await page.getAttribute('#server-logo', 'src'), 'https://launcher.example/storage/geo.png');
  assert.equal(await page.textContent('#splash'), '« Fonde ton pays ! » — Équipe');
  assert.equal(await page.textContent('#stat-money'), (1234).toLocaleString('fr-FR'));
  assert.equal(await page.evaluate(() => document.getElementById('stat-money-wrap').classList.contains('hidden')), false);
  assert.deepEqual(errors, []);
  await context.close();
});

// ---- Plusieurs comptes ------------------------------------------------------------------
const ACCOUNTS = `accounts: { active: 'geoventure.fr#1', accounts: [
  { key: 'geoventure.fr#1', id: 1, username: 'Testeur', skin_url: null, site_url: 'https://geoventure.fr/', role: null, active: true, needs_login: false },
  { key: 'geoventure.fr#2', id: 2, username: 'Alice', skin_url: null, site_url: 'https://geoventure.fr/', role: null, active: false, needs_login: false },
  { key: 'geoventure.fr#3', id: 3, username: 'Vieux', skin_url: null, site_url: 'https://geoventure.fr/', role: null, active: false, needs_login: true },
] }`;
const openAccounts = async (page) => { await page.click('.nav-btn[data-tab="game"]'); await page.click('.tab[data-tab="advanced"]'); };
const accountRow = (page, name) => page.locator('#account-list .account', { hasText: name });

test('comptes : liste avec compte actif et reconnexion requise', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS });
  await openAccounts(page);
  assert.equal(await page.locator('#account-list .account').count(), 3);
  assert.match(await accountRow(page, 'Testeur').textContent(), /Actif/);
  assert.equal(await accountRow(page, 'Testeur').locator('button:has-text("Utiliser")').count(), 0);
  assert.match(await accountRow(page, 'Vieux').textContent(), /Reconnexion requise/);
  assert.equal(await accountRow(page, 'Alice').locator('button:has-text("Utiliser")').count(), 1);
  assert.equal(await page.locator('#login-screen').isVisible(), false);
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : « Utiliser » change de compte, le jeu part avec le nouveau pseudo', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS });
  await openAccounts(page);
  await accountRow(page, 'Alice').locator('button:has-text("Utiliser")').click();
  await page.waitForFunction(() => document.getElementById('profile-name').textContent === 'Alice');
  assert.deepEqual(await calls(page, 'azuriom_switch_account'), [{ key: 'geoventure.fr#2' }]);
  assert.match(await accountRow(page, 'Alice').textContent(), /Actif/);
  assert.doesNotMatch(await accountRow(page, 'Testeur').textContent(), /Actif/);
  await page.click('#btn-play');
  await page.waitForFunction(() => window.__CALLS__.some((c) => c.cmd === 'launch_instance'));
  const [launch] = await calls(page, 'launch_instance');
  assert.equal(launch.req.username, 'Alice');
  assert.equal(launch.req.uuid, 'uuid-Alice');
  assert.equal(launch.req.access_token, 'jeton-Alice');
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : pas de changement de compte pendant la partie', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS });
  await page.click('#btn-play');
  await page.waitForFunction(() => document.getElementById('play-label').textContent === 'En jeu');
  await openAccounts(page);
  assert.equal(await accountRow(page, 'Alice').locator('button:has-text("Utiliser")').isDisabled(), true);
  assert.equal(await page.isDisabled('#btn-add-account'), true);
  assert.equal((await calls(page, 'azuriom_switch_account')).length, 0);
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : session refusée, le compte est marqué et la reconnexion proposée', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS.replace('accounts:', "refusedKeys: ['geoventure.fr#2'], accounts:") });
  await openAccounts(page);
  await accountRow(page, 'Alice').locator('button:has-text("Utiliser")').click();
  await page.waitForSelector('#login-screen:not(.hidden)');
  assert.equal(await page.inputValue('#az-email'), 'Alice');
  assert.match(await page.textContent('#az-status'), /Reconnexion requise pour Alice/);
  // « Annuler » revient au compte actif, qui n'a pas changé.
  await page.click('#btn-login-cancel');
  assert.equal(await page.locator('#login-screen').isVisible(), false);
  assert.equal(await page.textContent('#profile-name'), 'Testeur');
  assert.match(await accountRow(page, 'Alice').textContent(), /Reconnexion requise/);
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : « Retirer » demande confirmation', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS });
  await openAccounts(page);
  await accountRow(page, 'Alice').locator('button:has-text("Retirer")').click();
  assert.equal((await calls(page, 'azuriom_remove_account')).length, 0);
  await accountRow(page, 'Alice').locator('button:has-text("Confirmer")').click();
  await page.waitForFunction(() => document.querySelectorAll('#account-list .account').length === 2);
  assert.deepEqual(await calls(page, 'azuriom_remove_account'), [{ key: 'geoventure.fr#2' }]);
  assert.equal(await page.textContent('#profile-name'), 'Testeur');
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : « Ajouter un compte » garde les autres, « Annuler » revient au compte actif', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS });
  await openAccounts(page);
  await page.click('#btn-add-account');
  await page.waitForSelector('#login-screen:not(.hidden)');
  assert.equal(await page.locator('#login-account-list .account').count(), 3);
  assert.equal(await page.inputValue('#az-email'), '');
  await page.click('#btn-login-cancel');
  assert.equal(await page.locator('#login-screen').isVisible(), false);
  assert.equal(await page.textContent('#profile-name'), 'Testeur');
  // Nouvelle connexion : le nouveau compte s'ajoute et devient actif.
  await page.click('#btn-add-account');
  await page.fill('#az-email', 'Bob');
  await page.fill('#az-password', 'secret');
  await page.click('#btn-login');
  await page.waitForFunction(() => document.getElementById('profile-name').textContent === 'Bob');
  assert.equal(await page.locator('#login-screen').isVisible(), false);
  assert.equal(await page.locator('#account-list .account').count(), 4);
  assert.match(await accountRow(page, 'Bob').textContent(), /Actif/);
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : déconnexion du compte actif seulement, les autres sont proposés d’un clic', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS });
  await openAccounts(page);
  await page.click('#btn-logout');
  await page.waitForSelector('#login-screen:not(.hidden)');
  // Aucun autre compte n'est activé d'office ; pas d'« Annuler » sans compte actif.
  assert.equal(await page.locator('#btn-login-cancel').isVisible(), false);
  assert.equal(await page.locator('#login-account-list .account').count(), 2);
  assert.equal(await page.locator('#login-account-list .account.active').count(), 0);
  assert.equal(await page.isDisabled('#btn-play'), true);
  await page.locator('#login-account-list .account', { hasText: 'Alice' }).click();
  await page.waitForSelector('#login-screen', { state: 'hidden' });
  assert.equal(await page.textContent('#profile-name'), 'Alice');
  assert.deepEqual(errors, []);
  await context.close();
});

test('comptes : au démarrage sans compte actif mais des comptes gardés, ils sont proposés', async () => {
  const { page, context, errors } = await open({ mockSrc: ACCOUNTS.replace("active: 'geoventure.fr#1'", 'active: null').replace('active: true', 'active: false') });
  assert.equal(await page.locator('#login-screen').isVisible(), true);
  assert.equal(await page.locator('#login-account-list .account').count(), 3);
  // Compte à reconnecter : le formulaire est prérempli avec son pseudo.
  await page.locator('#login-account-list .account', { hasText: 'Vieux' }).click();
  assert.equal(await page.inputValue('#az-email'), 'Vieux');
  assert.equal(await page.isDisabled('#btn-play'), true);
  assert.deepEqual(errors, []);
  await context.close();
});
