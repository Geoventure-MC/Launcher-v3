/**
 * splash-ipc : canaux IPC (processus principal) dont la fenêtre de mise à jour a besoin depuis qu'elle
 * tourne isolée (sandbox + contextIsolation, sans Node). Le rendu ne peut plus lire de fichier, interroger
 * le panel ni ouvrir le navigateur lui-même : il demande, et ici on valide chaque demande.
 *  - splash-init   : plateforme, dépôt des releases, textes traduits (fichier choisi par un code de langue validé) ;
 *  - splash-config : GET du panel sur un chemin FIXE (/utils/api), adresse https (ou boucle locale) seulement ;
 *  - open-external : http(s) uniquement (même règle que window-guard).
 */
const fs = require('fs');
const path = require('path');

const LOCAL_HOSTS = ['localhost', '127.0.0.1', '[::1]'];

/** Adresse de base du panel : https, ou http en boucle locale (développement, tests). null sinon. */
function panelBase(url) {
    try {
        const u = new URL(String(url));
        const local = LOCAL_HOSTS.includes(u.hostname);
        if (!(u.protocol === 'https:' || (u.protocol === 'http:' && local))) return null;
        const href = u.href.split('#')[0].split('?')[0];
        return href.endsWith('/') ? href : `${href}/`;
    } catch {
        return null;
    }
}

/** URL de la config du panel, avec la même construction que utils/config.js + instance.js (withInstance). */
function configUrl(pkg, opts) {
    const o = opts && typeof opts === 'object' ? opts : {};
    const fallback = pkg.user ? `${pkg.settings}/${pkg.user}` : pkg.settings;
    const base = panelBase(o.serverUrl) || panelBase(fallback);
    if (!base) return null;
    let url = pkg.env === 'azuriom' ? `${base}api/centralcorp/options` : `${base}utils/api`;
    const instance = typeof o.instance === 'string' && /^[\w.-]{1,64}$/.test(o.instance) ? o.instance : null;
    if (instance) url += `${url.includes('?') ? '&' : '?'}instance=${encodeURIComponent(instance)}`;
    if (o.channel === 'test') url += `${url.includes('?') ? '&' : '?'}channel=test`;
    return url;
}

function readTranslations(dir, lang) {
    const code = typeof lang === 'string' && /^[a-z]{2,3}$/.test(lang) ? lang : 'en';
    for (const name of [code, 'en']) {
        const file = path.join(dir, `${name}.json`);
        try {
            if (fs.existsSync(file)) return JSON.parse(fs.readFileSync(file, 'utf8'));
        } catch (e) {
            console.error('[splash-ipc] traduction illisible :', name, e && e.message);
        }
    }
    return {};
}

function install({ ipcMain, shell, pkg, translationsDir, openExternalSafe }) {
    ipcMain.handle('splash-init', (_e, lang) => ({
        platform: process.platform,
        appImage: !!process.env.APPIMAGE,
        repositoryUrl: String((pkg.repository && pkg.repository.url) || ''),
        translations: readTranslations(translationsDir, lang),
    }));

    ipcMain.handle('splash-config', async (_e, opts) => {
        const url = configUrl(pkg, opts);
        if (!url) return { ok: false, error: 'adresse du panel invalide' };
        try {
            const res = await require('node-fetch')(url);
            if (!res.ok) return { ok: false, error: `HTTP ${res.status} sur ${url}` };
            return { ok: true, config: await res.json() };
        } catch (e) {
            return { ok: false, error: String((e && e.message) || e) };
        }
    });

    ipcMain.on('open-external', (_e, url) => { openExternalSafe(shell, url); });
}

module.exports = { install, panelBase, configUrl };
