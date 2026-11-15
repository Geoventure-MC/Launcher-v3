/**
 * Module de télémétrie opt-in.
 * N'envoie jamais rien sans consentement explicite de l'utilisateur ('telemetry_consent').
 * Assainit systématiquement les données annexes afin de ne jamais fuiter de jeton, mot de passe ou chemin personnel.
 */
'use strict';

import { withInstance } from './instance.js';
import { redact } from './diagnostic.js';

let pkg = { version: '0.0.0', settings: '', user: '' };
try {
    if (typeof require === 'function') pkg = require('../package.json');
} catch { /* contexte sans CommonJS (tests unitaires) */ }

const CONSENT_KEY = 'telemetry_consent';
const RESERVED_KEYS = new Set(['event', 'serverId', 'launcherVersion', 'os']);
const SENSITIVE_KEY_RE = /token|secret|password|passwd|auth|cookie|session|key|credential|email|mail/i;

function isConsented() {
    return localStorage.getItem(CONSENT_KEY) === 'true';
}

function setConsent(value) {
    localStorage.setItem(CONSENT_KEY, value ? 'true' : 'false');
}

function getPanelUrl() {
    const stored = localStorage.getItem('geoventure_server_url');
    const base = stored || (pkg.user ? `${pkg.settings}/${pkg.user}` : pkg.settings);
    return base.endsWith('/') ? base : `${base}/`;
}

/**
 * Assainit un objet `extra` avant envoi télémétrique :
 * supprime toute clé sensible, masque chemins personnels / jetons / e-mails et borne les longueurs.
 * @param {object} extra
 * @param {string[]} [secrets]
 * @returns {object}
 */
function sanitizeTelemetryExtra(extra, secrets = []) {
    if (!extra || typeof extra !== 'object' || Array.isArray(extra)) return {};
    const homes = [];
    try {
        if (typeof process !== 'undefined' && process && process.env) {
            if (process.env.HOME) homes.push(process.env.HOME);
            if (process.env.USERPROFILE) homes.push(process.env.USERPROFILE);
        }
    } catch { /* ignore */ }

    const clean = {};
    for (const [k, v] of Object.entries(extra)) {
        if (RESERVED_KEYS.has(k) || SENSITIVE_KEY_RE.test(k)) continue;
        if (v === null || v === undefined) continue;
        if (typeof v === 'number') {
            if (Number.isFinite(v)) clean[k] = v;
        } else if (typeof v === 'boolean') {
            clean[k] = v;
        } else if (typeof v === 'string') {
            const scrubbed = redact(v, { homes, secrets })
                .replace(/Bearer\s+[A-Za-z0-9._-]+/gi, 'Bearer [MASQUÉ]')
                .replace(/\b(access_token|refresh_token|client_token|password)=[^\s&]+/gi, '$1=[MASQUÉ]');
            clean[k] = scrubbed.slice(0, 240);
        }
    }
    return clean;
}

async function sendEvent(event, extra = {}) {
    if (!isConsented()) return;

    const panelUrl = getPanelUrl();
    const serverId = localStorage.getItem('geoventure_selected_instance') || 'default';
    const osLabel = (typeof process !== 'undefined' && process && process.platform)
        ? `${process.platform}-${process.arch}`
        : 'unknown';

    const payload = {
        ...sanitizeTelemetryExtra(extra),
        event: String(event || 'unknown').slice(0, 80),
        serverId,
        launcherVersion: pkg.version,
        os: osLabel,
    };

    const controller = typeof AbortController !== 'undefined' ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), 4000) : null;

    try {
        // Endpoint dédié côté panel : POST {panel}/utils/telemetry
        await fetch(withInstance(`${panelUrl}utils/telemetry`), {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'X-Requested-With': 'XMLHttpRequest',
            },
            body: JSON.stringify(payload),
            ...(controller ? { signal: controller.signal } : {}),
        });
    } catch {
        // Non-bloquant — la télémétrie ne doit jamais perturber le launcher
    } finally {
        if (timer) clearTimeout(timer);
    }
}

/**
 * Envoie un événement d'erreur critique catégorisé (non-bloquant, opt-in et assaini).
 * @param {'minecraft_crash'|'modpack_error'|'azuriom_auth_error'|'forge_patch_error'|string} errorType
 * @param {object} [details]
 */
function sendErrorEvent(errorType, details = {}) {
    return sendEvent(errorType, { category: 'error', ...details });
}

export { isConsented, setConsent, sanitizeTelemetryExtra, sendEvent, sendErrorEvent };
