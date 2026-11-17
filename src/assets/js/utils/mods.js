/**
 * Règles partagées sur les fichiers de mods (mods optionnels et mods communauté).
 *
 * - Un nom de fichier venu du panel n'est JAMAIS un chemin : pas de / \ .. ni de caractère de contrôle,
 *   et il doit finir en .jar. Sinon `path.join(modsDir, nom)` pourrait écrire ou supprimer un fichier
 *   n'importe où sur le disque (dossier Démarrage de Windows…).
 * - Un mod optionnel désactivé porte le suffixe `-disable` (`foo.jar-disable`), seul suffixe que la
 *   bibliothèque de lancement reconnaît : avec un autre suffixe, la vérification du modpack supprimait
 *   le fichier puis retéléchargeait le mod ACTIF.
 * - La correspondance clé → fichier est EXACTE (clé = nom de fichier, ou nom sans .jar) : un préfixe
 *   (`create`) visait aussi `createaddition-….jar`.
 */
export const DISABLED_SUFFIX = '-disable';

export function safeModFileName(name) {
    if (typeof name !== 'string') return null;
    const n = name.trim();
    if (!n || n.length > 200) return null;
    if (/[\/\\:*?"<>|\x00-\x1f]/.test(n)) return null;
    if (n.includes('..') || n.startsWith('.')) return null;
    if (!/\.jar$/i.test(n)) return null;
    return n;
}

/** Fichier (actif ou désactivé) correspondant à la clé de mod optionnel `mod`, ou null. */
export function findModFile(files, mod) {
    if (typeof mod !== 'string' || !mod) return null;
    const list = Array.isArray(files) ? files : [];
    const base = /\.jar$/i.test(mod) ? mod : `${mod}.jar`;
    const candidates = [base, base + DISABLED_SUFFIX];
    for (const c of candidates) if (list.includes(c)) return c;
    // Ancienne clé sans version (« minimap » pour « minimap-1.2.jar ») : préfixe suivi d'un séparateur
    // seulement, jamais d'une lettre ou d'un chiffre (« create » ne vise pas « createaddition »).
    if (!/\.jar$/i.test(mod)) {
        const re = new RegExp('^' + mod.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '[-_ .][^/\\\\]*\\.jar(' + DISABLED_SUFFIX + ')?$');
        const hit = list.find(f => re.test(f));
        if (hit) return hit;
    }
    return null;
}

/** Chemin cible d'un mod optionnel selon qu'il est activé ou non. */
export function modTargetName(file, enabled) {
    const active = file.endsWith(DISABLED_SUFFIX) ? file.slice(0, -DISABLED_SUFFIX.length) : file;
    return enabled ? active : active + DISABLED_SUFFIX;
}

// Mods communauté installés par le joueur, notés dans launcher_config/ (dossier jamais vérifié) pour être
// ajoutés à la liste `ignored` au lancement : sans cela, la vérification du modpack les supprimait.
// require() paresseux : les règles pures ci-dessus restent chargeables hors Electron (tests Node).
const fs = { get m() { return require('fs'); } };
const path = { get m() { return require('path'); } };
const crypto = { get m() { return require('crypto'); } };
function communityFile(gameDir) {
    return path.m.join(gameDir, 'launcher_config', 'community_mods.json');
}

export function readCommunityMods(gameDir) {
    try {
        const list = JSON.parse(fs.m.readFileSync(communityFile(gameDir), 'utf8'));
        return Array.isArray(list) ? list.map(safeModFileName).filter(Boolean) : [];
    } catch {
        return [];
    }
}

export function recordCommunityMod(gameDir, name, installed) {
    const n = safeModFileName(name);
    if (!n) return;
    const set = new Set(readCommunityMods(gameDir));
    if (installed) set.add(n); else set.delete(n);
    try {
        fs.m.mkdirSync(path.m.dirname(communityFile(gameDir)), { recursive: true });
        fs.m.writeFileSync(communityFile(gameDir), JSON.stringify([...set].sort(), null, 2));
    } catch (e) {
        console.warn('[mods] liste des mods communauté non enregistrée :', e && e.message);
    }
}

/** Chemins relatifs (`mods/<nom>`) des mods communauté installés, pour la liste `ignored`. */
export function communityModPaths(gameDir) {
    return readCommunityMods(gameDir)
        .filter(n => { try { return fs.m.existsSync(path.m.join(gameDir, 'mods', n)); } catch { return false; } })
        .map(n => `mods/${n}`);
}

/** Calcule le SHA-1 hexadécimal d'un fichier local. */
export function sha1File(filePath) {
    return new Promise((resolve, reject) => {
        const hash = crypto.m.createHash('sha1');
        const stream = fs.m.createReadStream(filePath);
        stream.on('data', chunk => hash.update(chunk));
        stream.on('end', () => resolve(hash.digest('hex')));
        stream.on('error', reject);
    });
}

function isSafeHttpUrl(rawUrl) {
    try {
        const u = new URL(String(rawUrl));
        return u.protocol === 'http:' || u.protocol === 'https:';
    } catch {
        return false;
    }
}

/**
 * Compare le manifeste modpack aux fichiers locaux (taille + SHA-1).
 * Retourne { valid, missing, corrupted, total } sans modifier le disque.
 */
export async function diffVerifyManifest(manifest, gameDir, { isIgnoredPath, onProgress } = {}) {
    const entries = Array.isArray(manifest) ? manifest : [];
    const baseDir = path.m.resolve(gameDir);
    const valid = [];
    const missing = [];
    const corrupted = [];
    const checkable = entries.filter(e => e && typeof e.path === 'string' && e.path && !(typeof isIgnoredPath === 'function' && isIgnoredPath(e.path)));

    for (let i = 0; i < checkable.length; i++) {
        const entry = checkable[i];
        const target = path.m.resolve(baseDir, entry.path);
        if (target !== baseDir && !target.startsWith(baseDir + path.m.sep)) continue;

        let stat = null;
        try {
            stat = fs.m.statSync(target);
        } catch {
            missing.push(entry);
            if (onProgress) onProgress({ phase: 'scan', done: i + 1, total: checkable.length, entry });
            continue;
        }
        if (!stat.isFile()) {
            missing.push(entry);
            if (onProgress) onProgress({ phase: 'scan', done: i + 1, total: checkable.length, entry });
            continue;
        }

        let bad = false;
        if (entry.size != null && stat.size !== Number(entry.size)) {
            bad = true;
        } else if (entry.hash) {
            const localHash = await sha1File(target);
            bad = localHash.toLowerCase() !== String(entry.hash).toLowerCase();
        }
        if (bad) corrupted.push(entry);
        else valid.push(entry);

        if (onProgress) onProgress({ phase: 'scan', done: i + 1, total: checkable.length, entry });
    }

    return { valid, missing, corrupted, total: checkable.length };
}

/**
 * Réparation différentielle SHA-1 en 1 clic :
 * ne retélécharge que les fichiers manquants ou corrompus du manifeste.
 */
export async function repairDifferential(manifest, gameDir, { isIgnoredPath, fetchFn = fetch, onProgress } = {}) {
    const scan = await diffVerifyManifest(manifest, gameDir, { isIgnoredPath, onProgress });
    const toRepair = [...scan.missing, ...scan.corrupted];
    const baseDir = path.m.resolve(gameDir);
    let repairedCount = 0;
    let failedCount = 0;
    const errors = [];

    for (let i = 0; i < toRepair.length; i++) {
        const entry = toRepair[i];
        const target = path.m.resolve(baseDir, entry.path);
        if (target !== baseDir && !target.startsWith(baseDir + path.m.sep)) continue;

        try {
            if (!entry.url || !isSafeHttpUrl(entry.url)) {
                throw new Error(`Invalid URL for ${entry.path}`);
            }
            const res = await fetchFn(entry.url);
            if (!res.ok) throw new Error(`HTTP ${res.status} for ${entry.path}`);
            const buf = Buffer.from(await res.arrayBuffer());
            if (entry.hash) {
                const got = crypto.m.createHash('sha1').update(buf).digest('hex');
                if (got.toLowerCase() !== String(entry.hash).toLowerCase()) {
                    throw new Error(`SHA-1 mismatch after download for ${entry.path}`);
                }
            }
            fs.m.mkdirSync(path.m.dirname(target), { recursive: true });
            fs.m.writeFileSync(target, buf);
            repairedCount++;
        } catch (err) {
            failedCount++;
            errors.push({ path: entry.path, error: err && err.message ? err.message : String(err) });
        }

        if (onProgress) onProgress({ phase: 'download', done: i + 1, total: toRepair.length, entry });
    }

    return {
        total: scan.total,
        validCount: scan.valid.length,
        missingCount: scan.missing.length,
        corruptedCount: scan.corrupted.length,
        repairedCount,
        failedCount,
        errors,
    };
}

/**
 * Le jeu tourne-t-il déjà sur ce dossier ? Sous Windows, Java garde ses jars ouverts sans autoriser leur
 * renommage : on tente de renommer un jar du dossier mods puis de le remettre. Échec « fichier occupé » = un
 * Minecraft est ouvert, la mise à jour ne pourrait ni remplacer ni supprimer ses mods. Ailleurs (Linux, macOS),
 * les fichiers ouverts se renomment librement : la réponse est toujours non, sans conséquence.
 */
export function modsInUse(gameDir) {
    let modsDir;
    try {
        modsDir = path.m.join(gameDir, 'mods');
        const jar = fs.m.readdirSync(modsDir).find(f => /\.jar$/i.test(f));
        if (!jar) return false;
        const from = path.m.join(modsDir, jar);
        const probe = `${from}.geocheck`;
        try {
            fs.m.renameSync(from, probe);
        } catch (e) {
            return !!e && ['EBUSY', 'EPERM', 'EACCES'].includes(e.code);
        }
        // Remise en place (plusieurs essais : un antivirus peut ouvrir le fichier entre les deux renommages).
        for (let i = 0; i < 5; i++) {
            try { fs.m.renameSync(probe, from); return false; } catch { /* nouvel essai */ }
        }
        return false;
    } catch {
        return false;
    }
}
