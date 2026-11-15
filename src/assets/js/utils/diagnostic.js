/**
 * diagnostic — rapport de diagnostic en 1 clic (Réglages > Avancé).
 *
 * Les fonctions du haut (redact, tailLines, formatReport) sont PURES et testables depuis Node ;
 * collectDiagnostic() (renderer uniquement) lit le disque et teste le réseau.
 * Tout le rapport passe par redact() AVANT d'être affiché, copié, enregistré ou envoyé :
 * jetons, mots de passe, e-mails, UUID passés en argument, dossier personnel (remplacé par ~).
 */
'use strict';

export const MASK = '[MASQUÉ]';
export const LOG_LINES = 300;

function escapeRe(s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }

/**
 * Masque les données sensibles d'un texte.
 * @param {string} text
 * @param {{homes?: string[], secrets?: string[]}} opts  homes : dossiers personnels à remplacer par ~ ; secrets : valeurs exactes à masquer.
 */
export function redact(text, { homes = [], secrets = [] } = {}) {
    let out = String(text == null ? '' : text);

    // 1) Valeurs secrètes connues (jetons des comptes enregistrés), les plus longues d'abord.
    for (const s of [...new Set(secrets.filter(x => typeof x === 'string' && x.length >= 6))].sort((a, b) => b.length - a.length)) {
        out = out.split(s).join(MASK);
    }

    // 2) Dossier personnel -> ~ (formes / et \, insensible à la casse pour Windows).
    const variants = new Set();
    for (const h of homes) {
        if (!h || h.length < 3) continue;
        const clean = h.replace(/[\\/]+$/, '');
        variants.add(clean);
        variants.add(clean.replace(/\\/g, '/'));
        variants.add(clean.replace(/\//g, '\\'));
    }
    for (const v of [...variants].sort((a, b) => b.length - a.length)) {
        out = out.replace(new RegExp(escapeRe(v), 'gi'), '~');
    }
    // Chemins personnels d'un autre utilisateur ou d'une forme non détectée.
    out = out
        .replace(/([A-Za-z]:[\\/]+Users[\\/]+)[^\\/\s"']+/gi, '$1<user>')
        .replace(/(\/home\/)[^/\s"']+/g, '$1<user>')
        .replace(/(\/Users\/)[^/\s"']+/g, '$1<user>');

    // 3) Jetons / mots de passe.
    out = out
        .replace(/(--(?:accessToken|session|password|uuid|xuid|clientId|userProperties)\s+)\S+/gi, `$1${MASK}`)
        .replace(/(["']?(?:access[_-]?token|client[_-]?token|refresh[_-]?token|id[_-]?token|token|password|passwd|pwd|mot[_ ]de[_ ]passe|secret|api[_-]?key|authorization|cookie)["']?\s*[:=]\s*)(?:Bearer\s+|Basic\s+)?(?:"[^"]*"|'[^']*'|[^\s"',;&}]+)/gi, `$1${MASK}`)
        .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{6,}/g, `$1 ${MASK}`)
        .replace(/\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*/g, MASK)
        .replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, '[e-mail masqué]')
        .replace(/\b[A-Fa-f0-9]{32,}\b/g, MASK);
    return out;
}

/** Dernières `n` lignes d'un texte. */
export function tailLines(text, n = LOG_LINES) {
    const lines = String(text || '').replace(/\r/g, '').split('\n');
    if (lines.length && lines[lines.length - 1] === '') lines.pop();
    return lines.slice(-n).join('\n');
}

/** Assemble les sections en un rapport texte. sections : [{title, body}] */
export function formatReport(sections) {
    return sections.map(s => `=== ${s.title} ===\n${s.body}`).join('\n\n') + '\n';
}

/** Dernières lignes d'un fichier (lecture des derniers 256 Ko seulement). null si absent. */
function readTail(file, n = LOG_LINES) {
    const fs = require('fs');
    let fd;
    try {
        const st = fs.statSync(file);
        if (!st.isFile()) return null;
        const size = Math.min(st.size, 262144);
        const buf = Buffer.alloc(size);
        fd = fs.openSync(file, 'r');
        fs.readSync(fd, buf, 0, size, st.size - size);
        return tailLines(buf.toString('utf8'), n);
    } catch { return null; }
    finally { if (fd !== undefined) try { fs.closeSync(fd); } catch { /* */ } }
}

async function probe(fetch, name, url, method = 'HEAD') {
    const t0 = Date.now();
    try {
        const res = await fetch(url, { method, timeout: 6000, redirect: 'follow' });
        return `${name}: ${res.status < 500 ? 'joignable' : 'ERREUR SERVEUR'} (HTTP ${res.status}, ${Date.now() - t0} ms) ${url}`;
    } catch (e) {
        return `${name}: INJOIGNABLE (${String(e && e.code || e && e.message || e).slice(0, 80)}, ${Date.now() - t0} ms) ${url}`;
    }
}

function detectJava(gameDir, javaPath) {
    const fs = require('fs'), path = require('path'), cp = require('child_process');
    const lines = [];
    if (javaPath) {
        try {
            const r = cp.spawnSync(javaPath, ['-version'], { encoding: 'utf8', timeout: 5000 });
            const txt = String((r.stderr || r.stdout || '')).split('\n')[0] || (r.error ? r.error.message : 'sortie vide');
            lines.push(`Java personnalisé: ${javaPath} -> ${txt.trim()}`);
        } catch (e) { lines.push(`Java personnalisé: ${javaPath} -> illisible (${e.message})`); }
    } else {
        lines.push('Java personnalisé: aucun (le launcher télécharge le sien)');
    }
    try {
        const rt = path.join(gameDir, 'runtime');
        const found = fs.existsSync(rt) ? fs.readdirSync(rt) : [];
        if (!found.length) lines.push('Java téléchargé par le launcher: aucun (pas encore téléchargé)');
        for (const d of found) {
            let ver = '?';
            const cands = [path.join(rt, d, 'release')];
            try { for (const sub of fs.readdirSync(path.join(rt, d))) cands.push(path.join(rt, d, sub, 'release')); } catch { /* */ }
            for (const f of cands) {
                try {
                    const m = /JAVA_VERSION="?([^"\n]+)/.exec(fs.readFileSync(f, 'utf8'));
                    if (m) { ver = m[1]; break; }
                } catch { /* */ }
            }
            lines.push(`Java téléchargé par le launcher: runtime/${d} (version ${ver})`);
        }
    } catch { lines.push('Java téléchargé par le launcher: illisible'); }
    return lines.join('\n');
}

/**
 * Collecte et assemble le rapport (renderer uniquement).
 * @param {object} c { pkg, config, gameDir, settingsUrl, azauthUrl, instance, hw, javaPath, ram, jvmArgs, profile, secrets }
 */
export async function collectDiagnostic(c) {
    const os = require('os'), path = require('path'), fetch = require('node-fetch');
    const hw = c.hw;
    const now = new Date();
    const linux = process.platform === 'linux';
    const sys = [
        `Launcher: ${c.pkg.productName || c.pkg.name} ${c.pkg.version} (env ${c.pkg.env})`,
        `Electron ${process.versions.electron} / Chrome ${process.versions.chrome} / Node ${process.versions.node}`,
        `OS: ${os.type()} ${os.release()} (${process.platform}) ${os.arch()}`,
    ];
    if (linux) {
        const e = process.env;
        sys.push(`Session: ${e.XDG_SESSION_TYPE || 'inconnue'} (WAYLAND_DISPLAY=${e.WAYLAND_DISPLAY ? 'oui' : 'non'}, DISPLAY=${e.DISPLAY ? 'oui' : 'non'}, bureau=${e.XDG_CURRENT_DESKTOP || '?'})`);
    }
    const hardware = [
        `RAM: ${hw.totalGb} Go au total, ${hw.freeGb} Go libres`,
        `CPU: ${hw.cores} coeurs - ${hw.cpuModel}`,
        `GPU: ${hw.gpu || 'inconnu'}`,
    ];
    const config = [
        `Instance active: ${c.instance || '(globale)'}`,
        `Dossier de jeu: ${c.gameDir}`,
        `Loader: ${c.config && c.config.loader ? `${c.config.loader.type} ${c.config.loader.build}` : '?'} / Minecraft ${c.config && c.config.game_version || '?'}`,
        `RAM allouée: ${c.ram ? `${c.ram.ramMin}-${c.ram.ramMax} Go` : 'défaut'}`,
        `Profil de performance: ${c.profile || 'manuel'}`,
        `Arguments JVM: ${(c.jvmArgs || []).join(' ') || '(aucun)'}`,
    ];

    const base = c.settingsUrl.endsWith('/') ? c.settingsUrl : `${c.settingsUrl}/`;
    const net = await Promise.all([
        probe(fetch, 'Panel', `${base}api-schema.json`, 'GET'),
        probe(fetch, 'Azuriom', c.azauthUrl),
        probe(fetch, 'Mojang', 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'),
        probe(fetch, 'Forge', 'https://maven.minecraftforge.net/'),
    ]);

    const logs = path.join(c.gameDir, 'logs');
    const launcherLog = readTail(path.join(logs, 'launcher.log'));
    const latestLog = readTail(path.join(logs, 'latest.log'));

    const raw = formatReport([
        { title: 'Nexus - rapport de diagnostic', body: `Généré le ${now.toISOString()}` },
        { title: 'Système', body: sys.join('\n') },
        { title: 'Matériel', body: hardware.join('\n') },
        { title: 'Java', body: detectJava(c.gameDir, c.javaPath) },
        { title: 'Configuration', body: config.join('\n') },
        { title: 'Réseau', body: net.join('\n') },
        { title: `launcher.log (${LOG_LINES} dernières lignes)`, body: launcherLog || '(absent)' },
        { title: `latest.log (${LOG_LINES} dernières lignes)`, body: latestLog || '(absent)' },
    ]);
    return redact(raw, { homes: [os.homedir(), process.env.HOME, process.env.USERPROFILE].filter(Boolean), secrets: c.secrets || [] });
}

function findFirstMatchLine(lines, regex) {
    for (const line of lines) {
        if (regex.test(line)) return line.trim().slice(0, 220);
    }
    return null;
}

/**
 * Analyse le texte d'un crash-report ou des derniers logs pour en extraire
 * une cause lisible en français (fonction PURE, sans accès disque).
 * @param {string} text
 * @param {number|string|null} [exitCode]
 * @returns {{cause: string, title: string, summary: string, hint: string, matchedLine: string|null}}
 */
export function analyzeCrashText(text, exitCode = null) {
    const raw = String(text == null ? '' : text);
    const lines = raw.replace(/\r/g, '').split('\n');
    const codeStr = exitCode !== null && exitCode !== undefined ? String(exitCode) : '?';

    // 1. Version Java incompatible
    const javaVerLine = findFirstMatchLine(lines, /UnsupportedClassVersionError|has been compiled by a more recent version of the Java Runtime|class file version \d+(?:\.\d+)?/i);
    if (javaVerLine) {
        return {
            cause: 'java_version',
            title: 'Version de Java incompatible',
            summary: 'Le jeu ou un mod nécessite une autre version de Java (JDK 17 requis pour Minecraft 1.20.1 / Forge).',
            hint: 'Sélectionnez un JDK 17 dans Paramètres → Avancé → Java / JDK ou réinitialisez sur Automatique.',
            matchedLine: javaVerLine,
        };
    }

    // 2. Mémoire insuffisante (OOM / tas JVM)
    const oomLine = findFirstMatchLine(lines, /java\.lang\.OutOfMemoryError|Java heap space|GC overhead limit exceeded|Direct buffer memory|Could not reserve enough space for .* object heap|There is insufficient memory for the Java Runtime Environment/i);
    if (oomLine || exitCode === 137) {
        return {
            cause: 'oom',
            title: 'Mémoire insuffisante (OutOfMemoryError)',
            summary: 'Minecraft a épuisé la mémoire vive allouée ou la JVM n’a pas pu réserver le tas demandé.',
            hint: 'Ajustez la RAM allouée dans Paramètres → RAM (4 à 8 Go selon votre machine) et fermez les applications lourdes.',
            matchedLine: oomLine || `Process terminated with signal / exit code ${codeStr} (OOM)`,
        };
    }

    // 3. Mod manquant ou dépendance incompatible
    const missingModLine = findFirstMatchLine(lines, /Missing or unsupported mandatory dependencies|MissingModException|Missing language provider|Mod ["']?[A-Za-z0-9_.-]+["']? requires /i);
    if (missingModLine) {
        const depMatch = raw.match(/Mod\s+["']?([A-Za-z0-9_.-]+)["']?\s+requires\s+([A-Za-z0-9_.-]+)(?:\s+([^\n\r]{1,40}))?/i);
        const extraDetail = depMatch ? ` (${depMatch[1]} requiert ${depMatch[2]})` : '';
        return {
            cause: 'missing_mod',
            title: 'Mod ou dépendance manquant(e)',
            summary: `Un mod obligatoire ou une bibliothèque requise est manquant(e) ou dans une mauvaise version${extraDetail}.`,
            hint: 'Utilisez « Réparer l’installation » dans Paramètres → Avancé ou vérifiez vos mods ajoutés manuellement.',
            matchedLine: missingModLine,
        };
    }

    // 4. Conflit / erreur de chargement de mod (Forge / Mixin)
    const modErrLine = findFirstMatchLine(lines, /DuplicateModsFoundException|MixinApplyError|MixinTransformerError|ModLoadingException|Caught exception during mod loading/i);
    if (modErrLine) {
        const suspect = raw.match(/(?:Suspected Mods:|Mod File:|Failure message:)\s*([^\n\r]{1,120})/i);
        const extra = suspect ? ` Détail : ${suspect[1].trim()}` : '';
        return {
            cause: 'mod_conflict',
            title: 'Erreur ou conflit de mod (Forge / Mixin)',
            summary: `Un mod a déclenché une erreur fatale lors du chargement ou en jeu.${extra}`,
            hint: 'Désactivez vos mods optionnels/communautaires récents ou lancez « Réparer l’installation ».',
            matchedLine: modErrLine,
        };
    }

    // 5. Erreur graphique / pilote GPU (OpenGL / GLFW)
    const gpuLine = findFirstMatchLine(lines, /GLFW error|Pixel format not accelerated|driver does not appear to support OpenGL|EXCEPTION_ACCESS_VIOLATION.*(?:nvoglv64|atio6axx|ig[0-9]*icd|amdocl)/i);
    if (gpuLine) {
        return {
            cause: 'graphics',
            title: 'Erreur graphique / Pilote GPU (OpenGL)',
            summary: 'L’initialisation graphique (OpenGL / GLFW) a échoué.',
            hint: 'Mettez à jour les pilotes de votre carte graphique et vérifiez que Java utilise le GPU dédié.',
            matchedLine: gpuLine,
        };
    }

    // 6. Crash natif JVM (access violation / SIGSEGV / codes natifs Windows)
    const nativeLine = findFirstMatchLine(lines, /EXCEPTION_ACCESS_VIOLATION|SIGSEGV|A fatal error has been detected by the Java Runtime Environment/i);
    if (nativeLine || exitCode === -1073740791 || exitCode === -1073741819) {
        return {
            cause: 'native_crash',
            title: `Arrêt brutal de la JVM (code ${codeStr})`,
            summary: 'La machine virtuelle Java s’est arrêtée sur une erreur native critique.',
            hint: 'Vérifiez la version de Java (JDK 17), mettez à jour vos pilotes GPU et réinitialisez vos arguments JVM.',
            matchedLine: nativeLine || `Code de sortie natif ${codeStr}`,
        };
    }

    // 7. Fallback générique
    const genericErrorLine = findFirstMatchLine(lines, /\b(?:FATAL|ERROR)\b|Exception:|Error:|Caused by:/);
    return {
        cause: 'unknown',
        title: `Minecraft s’est arrêté avec une erreur (code ${codeStr})`,
        summary: genericErrorLine
            ? `Fermeture inattendue du jeu. Dernière erreur relevée : ${genericErrorLine}`
            : `Le processus Minecraft s’est terminé de façon inattendue avec le code de sortie ${codeStr}.`,
        hint: 'Copiez le diagnostic ci-dessous et transmettez-le au support pour obtenir de l’aide.',
        matchedLine: genericErrorLine,
    };
}

/**
 * Extrait les lignes d'erreur les plus pertinentes d'un journal Minecraft.
 * @param {string} logText
 * @param {number} [maxLines]
 * @returns {string}
 */
export function extractLogErrors(logText, maxLines = 45) {
    const lines = String(logText == null ? '' : logText).replace(/\r/g, '').split('\n');
    const picked = [];
    let captureContext = 0;
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        if (/(?:FATAL|ERROR|Exception|Error:|Caused by:|Suppressed:|---- Minecraft Crash Report ----|Description:)/.test(line)) {
            picked.push(line);
            captureContext = 5;
        } else if (captureContext > 0 && line.trim()) {
            picked.push(line);
            captureContext--;
        }
    }
    const result = picked.length > 0 ? picked.slice(-maxLines) : lines.filter(Boolean).slice(-maxLines);
    return result.join('\n');
}

/**
 * Recherche et lit le rapport de crash le plus récent dans `<gameDir>/crash-reports`.
 * @param {string} gameDir
 * @param {number} [sinceMs] Timestamp de début du lancement (0 pour ignorer le filtre temporel)
 * @returns {{filePath: string, fileName: string, content: string, mtimeMs: number}|null}
 */
export function findLatestCrashReport(gameDir, sinceMs = 0) {
    if (!gameDir) return null;
    const fs = require('fs');
    const path = require('path');
    const dir = path.join(gameDir, 'crash-reports');
    try {
        if (!fs.existsSync(dir)) return null;
        const files = fs.readdirSync(dir)
            .filter(name => /^crash-.*\.(txt|log)$/i.test(name))
            .map(name => {
                const filePath = path.join(dir, name);
                try {
                    const st = fs.statSync(filePath);
                    return st.isFile() ? { filePath, fileName: name, mtimeMs: st.mtimeMs } : null;
                } catch {
                    return null;
                }
            })
            .filter(Boolean)
            .sort((a, b) => b.mtimeMs - a.mtimeMs);

        const match = sinceMs > 0
            ? files.find(f => f.mtimeMs >= sinceMs - 5000)
            : files[0];
        if (!match) return null;

        const raw = fs.readFileSync(match.filePath, 'utf8');
        const headLines = raw.replace(/\r/g, '').split('\n').slice(0, 180).join('\n');
        return { ...match, content: headLines };
    } catch {
        return null;
    }
}

/**
 * Construit un rapport de crash complet et entièrement masqué (`redact()`) prêt à être
 * affiché dans la modale d'erreur et copié dans le presse-papiers.
 * @param {object} opts
 * @returns {object}
 */
export function collectCrashReport(opts = {}) {
    const os = require('os');
    const path = require('path');
    const homes = [os.homedir(), process.env.HOME, process.env.USERPROFILE].filter(Boolean);
    const secrets = Array.isArray(opts.secrets) ? opts.secrets : [];
    const scrub = (s) => redact(s, { homes, secrets });

    const exitCode = opts.exitCode !== undefined ? opts.exitCode : null;
    const crashFile = opts.gameDir ? findLatestCrashReport(opts.gameDir, opts.launchStartMs || 0) : null;
    const latestLogPath = opts.gameDir ? path.join(opts.gameDir, 'logs', 'latest.log') : null;
    const latestLogTail = latestLogPath ? readTail(latestLogPath, 160) : null;
    const consoleTail = Array.isArray(opts.logBuffer) ? opts.logBuffer.slice(-160).join('\n') : '';

    let source = 'console';
    let sourceName = 'console';
    let primaryText = consoleTail;

    if (crashFile && crashFile.content) {
        source = 'crash-report';
        sourceName = crashFile.fileName;
        primaryText = crashFile.content;
    } else if (latestLogTail) {
        source = 'latest.log';
        sourceName = 'logs/latest.log';
        primaryText = latestLogTail;
    }

    const analysisRaw = analyzeCrashText(primaryText || consoleTail, exitCode);
    const analysis = {
        cause: analysisRaw.cause,
        title: scrub(analysisRaw.title),
        summary: scrub(analysisRaw.summary),
        hint: scrub(analysisRaw.hint),
        matchedLine: analysisRaw.matchedLine ? scrub(analysisRaw.matchedLine) : null,
    };

    const excerptRaw = source === 'crash-report'
        ? primaryText.split('\n').slice(0, 50).join('\n')
        : extractLogErrors(primaryText, 40);
    const excerpt = scrub(excerptRaw || '(aucune sortie de log disponible)');

    const pkg = opts.pkg || {};
    const cfg = opts.config || {};
    const metaLines = [
        `Code de sortie : ${exitCode !== null ? exitCode : 'inconnu'}`,
        `Source analysée : ${sourceName}`,
        `Cause détectée : ${analysis.cause} — ${analysis.title}`,
        `Synthèse : ${analysis.summary}`,
        `Conseil : ${analysis.hint}`,
        `Launcher : ${pkg.productName || pkg.name || 'Launcher'} ${pkg.version || '?'}`,
        `OS : ${os.type()} ${os.release()} (${process.platform} ${process.arch})`,
        `Instance : ${opts.instance || '(défaut)'}`,
        `Loader : ${cfg.loader ? `${cfg.loader.type} ${cfg.loader.build}` : '?'} / MC ${cfg.game_version || '?'}`,
        `Java : ${opts.javaPath || 'automatique (launcher)'}`,
        `RAM : ${opts.ram ? `${opts.ram.ramMin}-${opts.ram.ramMax} Go` : '?'}`,
        `Arguments JVM : ${(opts.jvmArgs || []).join(' ') || '(défaut)'}`,
    ];

    const report = scrub(formatReport([
        { title: 'Nexus - Rapport de crash automatique', body: metaLines.join('\n') },
        { title: `Extrait (${sourceName})`, body: excerpt },
        ...(source === 'crash-report' && latestLogTail
            ? [{ title: 'Erreurs latest.log', body: extractLogErrors(latestLogTail, 35) }]
            : []),
    ]));

    return {
        exitCode,
        source,
        sourceName,
        analysis,
        excerpt,
        report,
    };
}
