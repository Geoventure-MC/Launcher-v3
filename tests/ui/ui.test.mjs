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
