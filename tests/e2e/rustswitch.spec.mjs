// Passage au launcher Rust (src/assets/js/windows/rustSwitch.js) : règles pures, sans Electron.
import { test, expect } from '@playwright/test';
import { createRequire } from 'node:module';
import crypto from 'node:crypto';
import path from 'node:path';
import { ROOT } from './helpers.mjs';

const require = createRequire(import.meta.url);
const rs = require(path.join(ROOT, 'src/assets/js/windows/rustSwitch.js'));
const { blake2b512 } = require(path.join(ROOT, 'src/assets/js/windows/blake2b.js'));

// Paire de clés minisign jetable, au format de Tauri (texte encodé en base64).
function keypair() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  const raw = Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url');
  const keyId = crypto.randomBytes(8);
  const pub = Buffer.from(`untrusted comment: test\n${Buffer.concat([Buffer.from('Ed'), keyId, raw]).toString('base64')}\n`).toString('base64');
  const sign = (data, prehash = true) => {
    const msg = prehash ? crypto.createHash('blake2b512').update(data).digest() : data;
    const sig = crypto.sign(null, msg, privateKey);
    const box = Buffer.concat([Buffer.from(prehash ? 'ED' : 'Ed'), keyId, sig]).toString('base64');
    return Buffer.from(`untrusted comment: sig\n${box}\ntrusted comment: t\nAAAA\n`).toString('base64');
  };
  return { pub, sign };
}

test('signature minisign : bonne acceptée, fichier ou clé modifiés refusés', () => {
  const k = keypair();
  const data = crypto.randomBytes(5000);
  expect(rs.verifyMinisign(data, k.sign(data), k.pub)).toBe(true);
  expect(rs.verifyMinisign(data, k.sign(data, false), k.pub)).toBe(true);
  const bad = Buffer.from(data); bad[10] ^= 1;
  expect(rs.verifyMinisign(bad, k.sign(data), k.pub)).toBe(false);
  expect(rs.verifyMinisign(data, k.sign(data), keypair().pub)).toBe(false);
});

test('BLAKE2b en JavaScript identique à celui de Node', () => {
  for (const n of [0, 1, 128, 129, 4097]) {
    const b = crypto.randomBytes(n);
    expect(blake2b512(b).equals(crypto.createHash('blake2b512').update(b).digest())).toBe(true);
  }
});

test('lien de téléchargement : seulement le dépôt officiel, bon format par système', () => {
  const base = 'https://github.com/Geoventure-MC/Launcher-v3/releases/download/rust-v2.0.9/';
  const latest = { version: '2.0.9', platforms: {
    'windows-x86_64': { url: `${base}Geoventure.Launcher_2.0.9_x64-setup.exe`, signature: 's' },
    'darwin-aarch64': { url: `${base}Geoventure.Launcher_universal.app.tar.gz`, signature: 's' },
    'linux-x86_64-appimage': { url: `${base}Geoventure.Launcher_2.0.9_amd64.AppImage`, signature: 's' },
  } };
  expect(rs.pickAsset(latest, 'win32', 'x64').url).toMatch(/setup\.exe$/);
  expect(rs.pickAsset(latest, 'darwin', 'arm64').url).toMatch(/\.app\.tar\.gz$/);
  expect(rs.pickAsset(latest, 'linux', 'x64').url).toMatch(/\.AppImage$/);
  latest.platforms['windows-x86_64'].url = 'https://evil.example/setup.exe';
  expect(rs.pickAsset(latest, 'win32', 'x64')).toBeNull();
  expect(rs.pickAsset(null, 'linux', 'x64')).toBeNull();
});

test('session recopiée au format du launcher Rust', () => {
  const s = rs.toRustSession({ access_token: 't', uuid: 'u', name: 'Bob', user_info: { role: { name: 'VIP' }, monnaie: 3 } }, 'https://geoventure.fr/');
  expect(s).toMatchObject({ id: 0, username: 'Bob', uuid: 'u', access_token: 't', money: 3, role: { name: 'VIP' }, site_url: 'https://geoventure.fr/' });
  expect(rs.toRustSession({ uuid: 'u', name: 'x' })).toBeNull();
  expect(rs.toRustSession({ access_token: 't', uuid: 'u', name: 'x' }, 'http://faux').site_url).toBe('https://geoventure.fr/');
});

test('la clé publique embarquée est lisible', () => {
  expect(rs.parsePublicKey(rs.PUBKEY_B64).key.length).toBe(32);
});
