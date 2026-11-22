/**
 * Passage au nouveau launcher Geoventure (Rust / Tauri).
 *
 * Dernière version de l'ancien launcher : au démarrage, il installe le nouveau launcher (une seule fois),
 * y recopie la connexion du joueur, l'ouvre, puis se ferme. Les raccourcis de l'ancien launcher continuent
 * donc d'ouvrir le nouveau. En cas d'échec (réseau, disque, signature), l'ancien launcher démarre comme
 * avant : personne ne reste bloqué.
 *
 * Sécurité : l'installateur téléchargé est vérifié avec la signature minisign publiée dans latest.json et
 * la clé publique du launcher Rust (la même que celle de ses mises à jour automatiques).
 */
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { spawn, execFile } = require('child_process');
const { blake2b512 } = require('./blake2b.js');

const LATEST_JSON = 'https://github.com/Geoventure-MC/Launcher-v3/releases/download/rust-latest/latest.json';
const ALLOWED_PREFIX = 'https://github.com/Geoventure-MC/Launcher-v3/releases/download/';
// Clé publique minisign du launcher Rust (src-tauri/tauri.conf.json, plugins.updater.pubkey).
const PUBKEY_B64 = 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEJGOEJEODJEMjNGMDNCNzEKUldSeE8vQWpMZGlMdnpYVU9NZWFIUStCWTliQk5PYXZGZldFWnVYSXNzR1h5ZUJSVDRjdkYwcDkK';
const PRODUCT = 'Geoventure Launcher';

// ---------------------------------------------------------------- règles pures (testées)

/** Plateforme de latest.json pour ce système. */
function platformKey(platform, arch) {
    if (platform === 'win32') return arch === 'arm64' ? 'windows-aarch64' : 'windows-x86_64';
    if (platform === 'darwin') return arch === 'arm64' ? 'darwin-aarch64' : 'darwin-x86_64';
    if (platform === 'linux') return arch === 'arm64' ? 'linux-aarch64-appimage' : 'linux-x86_64-appimage';
    return null;
}

/** Lien et signature à utiliser, ou null si latest.json ne convient pas (lien hors du dépôt, format). */
function pickAsset(latest, platform, arch) {
    const key = platformKey(platform, arch);
    const p = key && latest && latest.platforms && (latest.platforms[key] || latest.platforms[key.replace('-appimage', '')]);
    if (!p || typeof p.url !== 'string' || typeof p.signature !== 'string') return null;
    if (!p.url.startsWith(ALLOWED_PREFIX) || /[?#]/.test(p.url)) return null;
    const ok = platform === 'win32' ? /\.exe$/i.test(p.url)
        : platform === 'darwin' ? /\.app\.tar\.gz$/i.test(p.url)
        : /\.AppImage$/i.test(p.url);
    if (!ok) return null;
    return { url: p.url, signature: p.signature, version: String(latest.version || '') };
}

/** Clé publique minisign (texte base64 de tauri.conf.json) → { keyId, key } (32 octets Ed25519). */
function parsePublicKey(b64) {
    const text = Buffer.from(b64, 'base64').toString('utf8');
    const line = text.split('\n').map(s => s.trim()).filter(Boolean).find(l => !l.startsWith('untrusted comment'));
    const raw = Buffer.from(line, 'base64');
    if (raw.length !== 42 || raw.subarray(0, 2).toString() !== 'Ed') throw new Error('clé publique invalide');
    return { keyId: raw.subarray(2, 10), key: raw.subarray(10) };
}

/**
 * Vérifie une signature minisign (format Tauri : texte de la signature encodé en base64).
 * « ED » = signature de l'empreinte BLAKE2b-512 du fichier ; « Ed » = signature du fichier.
 */
function verifyMinisign(data, signatureB64, pubkeyB64) {
    const pub = parsePublicKey(pubkeyB64);
    const text = Buffer.from(signatureB64, 'base64').toString('utf8');
    const lines = text.split('\n').map(s => s.trim()).filter(Boolean);
    const sigLine = lines.find(l => !l.startsWith('untrusted comment') && !l.startsWith('trusted comment'));
    if (!sigLine) return false;
    const sig = Buffer.from(sigLine, 'base64');
    if (sig.length !== 74) return false;
    const alg = sig.subarray(0, 2).toString();
    if (!sig.subarray(2, 10).equals(pub.keyId)) return false;
    let message;
    if (alg === 'ED') {
        // Electron (BoringSSL) n'a pas toujours BLAKE2b : repli en JavaScript pur.
        try { message = crypto.createHash('blake2b512').update(data).digest(); } catch { message = blake2b512(data); }
    }
    else if (alg === 'Ed') message = data;
    else return false;
    const key = crypto.createPublicKey({ key: { kty: 'OKP', crv: 'Ed25519', x: pub.key.toString('base64url') }, format: 'jwk' });
    return crypto.verify(null, message, key, sig.subarray(10));
}

/** Session du nouveau launcher (même format que son fichier azuriom_session.json). */
function toRustSession(account, siteUrl) {
    if (!account || typeof account.access_token !== 'string' || !account.access_token || !account.uuid || !account.name) return null;
    const info = account.user_info || {};
    const role = info.role && typeof info.role === 'object' && info.role.name
        ? { name: String(info.role.name), color: info.role.color ? String(info.role.color) : null, is_admin: false }
        : null;
    return {
        id: 0,
        username: String(account.name),
        uuid: String(account.uuid),
        access_token: account.access_token,
        email_verified: info.verified !== false,
        money: Number(info.monnaie) || 0,
        banned: false,
        role,
        site_url: /^https:\/\//.test(String(siteUrl || '')) ? String(siteUrl) : 'https://geoventure.fr/',
        skin_url: null,
    };
}

// ---------------------------------------------------------------- emplacements

function home() { return os.homedir(); }

/** Dossier de session du nouveau launcher (directories::ProjectDirs("fr", "geoventure", "GeoventureLauncher")). */
function rustSessionDir(platform = process.platform) {
    if (platform === 'win32') return path.join(process.env.LOCALAPPDATA || path.join(home(), 'AppData', 'Local'), 'geoventure', 'GeoventureLauncher', 'data');
    if (platform === 'darwin') return path.join(home(), 'Library', 'Application Support', 'fr.geoventure.GeoventureLauncher');
    return path.join(process.env.XDG_DATA_HOME || path.join(home(), '.local', 'share'), 'geoventurelauncher');
}

function linuxAppImagePath() {
    return path.join(process.env.XDG_DATA_HOME || path.join(home(), '.local', 'share'), 'geoventure-launcher', 'Geoventure-Launcher.AppImage');
}

/** Chemin du nouveau launcher s'il est déjà installé. */
function findInstalled(platform = process.platform) {
    if (platform === 'win32') {
        const dir = path.join(process.env.LOCALAPPDATA || path.join(home(), 'AppData', 'Local'), PRODUCT);
        try {
            const exe = fs.readdirSync(dir).find(f => /\.exe$/i.test(f) && !/^uninstall/i.test(f));
            return exe ? path.join(dir, exe) : null;
        } catch { return null; }
    }
    if (platform === 'darwin') {
        for (const base of [path.join(home(), 'Applications'), '/Applications']) {
            const app = path.join(base, `${PRODUCT}.app`);
            if (fs.existsSync(app)) return app;
        }
        return null;
    }
    const img = linuxAppImagePath();
    return fs.existsSync(img) ? img : null;
}

// ---------------------------------------------------------------- étapes

async function download(url, onProgress) {
    const res = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(10 * 60 * 1000) });
    if (!res.ok) throw new Error(`téléchargement refusé (HTTP ${res.status})`);
    const total = Number(res.headers.get('content-length')) || 0;
    const chunks = [];
    let got = 0;
    for await (const chunk of res.body) {
        chunks.push(chunk);
        got += chunk.length;
        if (onProgress) onProgress(got, total);
    }
    return Buffer.concat(chunks);
}

function run(file, args) {
    return new Promise((resolve, reject) => {
        execFile(file, args, { windowsHide: true, timeout: 5 * 60 * 1000 }, (err) => (err ? reject(err) : resolve()));
    });
}

async function install(asset, data) {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'geoventure-rust-'));
    try {
        if (process.platform === 'win32') {
            const exe = path.join(tmp, 'Geoventure-Launcher-setup.exe');
            fs.writeFileSync(exe, data);
            await run(exe, ['/S']); // installation silencieuse pour l'utilisateur courant (pas de droits admin)
        } else if (process.platform === 'darwin') {
            const tgz = path.join(tmp, 'launcher.app.tar.gz');
            fs.writeFileSync(tgz, data);
            const apps = path.join(home(), 'Applications');
            fs.mkdirSync(apps, { recursive: true });
            await run('/usr/bin/tar', ['-xzf', tgz, '-C', apps]);
        } else {
            const img = linuxAppImagePath();
            fs.mkdirSync(path.dirname(img), { recursive: true });
            fs.writeFileSync(`${img}.part`, data, { mode: 0o755 });
            fs.renameSync(`${img}.part`, img);
        }
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
}

/** Recopie la connexion, sans jamais écraser une session déjà ouverte dans le nouveau launcher. */
function copySession(account, siteUrl) {
    const session = toRustSession(account, siteUrl);
    if (!session) return false;
    const dir = rustSessionDir();
    const file = path.join(dir, 'azuriom_session.json');
    if (fs.existsSync(file)) return false;
    fs.mkdirSync(dir, { recursive: true });
    // Le nouveau launcher range le jeton dans le coffre-fort du système à sa première lecture.
    fs.writeFileSync(file, JSON.stringify(session, null, 2), { mode: 0o600 });
    return true;
}

function openRust(target) {
    const opts = { detached: true, stdio: 'ignore' };
    const child = process.platform === 'darwin'
        ? spawn('/usr/bin/open', ['-a', target], opts)
        : spawn(target, [], opts);
    child.unref();
}

/**
 * Passe au nouveau launcher. Renvoie { ok: true } quand il est ouvert (l'ancien doit alors se fermer),
 * ou { ok: false, error } pour continuer avec l'ancien.
 */
async function switchToRust({ account, siteUrl, onStatus }) {
    const status = (m) => { try { onStatus && onStatus(m); } catch { /* affichage seulement */ } };
    try {
        let target = findInstalled();
        if (!target) {
            status({ text: 'Installation du nouveau launcher Geoventure…' });
            const res = await fetch(LATEST_JSON, { signal: AbortSignal.timeout(15000) });
            if (!res.ok) throw new Error(`latest.json indisponible (HTTP ${res.status})`);
            const asset = pickAsset(await res.json(), process.platform, process.arch);
            if (!asset) throw new Error('aucune version pour ce système');
            const data = await download(asset.url, (got, total) => status({ text: 'Téléchargement du nouveau launcher…', got, total }));
            if (!verifyMinisign(data, asset.signature, PUBKEY_B64)) throw new Error('signature du fichier invalide');
            status({ text: 'Installation du nouveau launcher Geoventure…' });
            await install(asset, data);
            target = findInstalled();
            if (!target) throw new Error('nouveau launcher introuvable après installation');
        }
        try { copySession(account, siteUrl); } catch (e) { console.warn('[rust-switch] session non recopiée :', e.message); }
        status({ text: 'Ouverture du nouveau launcher…' });
        openRust(target);
        return { ok: true };
    } catch (err) {
        console.error('[rust-switch]', err);
        return { ok: false, error: String((err && err.message) || err) };
    }
}

module.exports = { switchToRust, pickAsset, platformKey, verifyMinisign, parsePublicKey, toRustSession, rustSessionDir, PUBKEY_B64 };
