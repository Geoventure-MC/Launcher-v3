/**
 * @file Validation sécurisée des chemins Java et détection automatique des JDK installés.
 * @module utils/java
 */

const JAVA_BIN_RE = /^javaw?(\.exe)?$/i;
const WINDOWS_ABS_RE = /^[A-Za-z]:[\\/]/;

/**
 * Vérifie la syntaxe et la sûreté d'un chemin de binaire Java sans accès disque.
 * Refuse les chemins relatifs, traversées `..`, chemins UNC réseau et caractères de contrôle.
 * @param {*} binPath
 * @returns {boolean}
 */
function isValidJavaBinarySyntax(binPath) {
    if (typeof binPath !== 'string') return false;
    const trimmed = binPath.trim();
    if (!trimmed || trimmed.length > 1024) return false;
    if (/[\0\r\n]/.test(trimmed)) return false;
    // Refuse les chemins UNC réseau (\\serveur\partage) ou URLs
    if (trimmed.startsWith('\\\\') || trimmed.startsWith('//') || /^[a-z]+:\/\//i.test(trimmed)) {
        return false;
    }
    const isAbs = WINDOWS_ABS_RE.test(trimmed) || trimmed.startsWith('/');
    if (!isAbs) return false;
    const segments = trimmed.split(/[\\/]+/).filter(Boolean);
    if (segments.length === 0) return false;
    if (segments.some(seg => seg === '..' || seg === '.')) return false;
    const base = segments[segments.length - 1];
    return JAVA_BIN_RE.test(base);
}

/**
 * Vérifie qu'un chemin pointe vers un exécutable Java réel et valide sur disque.
 * @param {*} binPath
 * @returns {boolean}
 */
function isValidJavaBinary(binPath) {
    if (!isValidJavaBinarySyntax(binPath)) return false;
    const fs = require('fs');
    const path = require('path');
    try {
        const resolved = path.resolve(binPath.trim());
        if (!JAVA_BIN_RE.test(path.basename(resolved))) return false;
        return fs.statSync(resolved).isFile();
    } catch {
        return false;
    }
}

/**
 * Parse la sortie de `java -version` pour en extraire la version majeure et le fournisseur.
 * @param {string} output
 * @returns {{version: string|null, major: number|null, vendor: string|null, isJava17: boolean}}
 */
function parseJavaVersionOutput(output) {
    const text = String(output || '');
    const match = text.match(/version\s+"([^"]+)"/i);
    const version = match ? match[1] : null;
    let major = null;
    if (version) {
        const parts = version.split(/[._+-]/).filter(Boolean);
        if (parts[0] === '1' && parts[1]) {
            const m = Number.parseInt(parts[1], 10);
            if (Number.isFinite(m)) major = m;
        } else if (parts[0]) {
            const m = Number.parseInt(parts[0], 10);
            if (Number.isFinite(m)) major = m;
        }
    }
    let vendor = null;
    if (/Temurin|Eclipse Adoptium/i.test(text)) vendor = 'Eclipse Adoptium';
    else if (/AdoptOpenJDK/i.test(text)) vendor = 'AdoptOpenJDK';
    else if (/Corretto/i.test(text)) vendor = 'Amazon Corretto';
    else if (/Zulu/i.test(text)) vendor = 'Azul Zulu';
    else if (/Microsoft/i.test(text)) vendor = 'Microsoft';
    else if (/GraalVM/i.test(text)) vendor = 'GraalVM';
    else if (/Liberica|BellSoft/i.test(text)) vendor = 'BellSoft Liberica';
    else if (/JetBrains/i.test(text)) vendor = 'JetBrains';
    else if (/Java\(TM\)/i.test(text)) vendor = 'Oracle';
    else if (/OpenJDK/i.test(text)) vendor = 'OpenJDK';

    return {
        version,
        major,
        vendor,
        isJava17: major === 17,
    };
}

/**
 * Exécute `binPath -version` sans shell afin de lire la version Java en toute sécurité.
 * @param {string} binPath
 * @returns {{path: string, version: string|null, major: number|null, vendor: string|null, isJava17: boolean}|null}
 */
function probeJavaVersion(binPath) {
    if (!isValidJavaBinary(binPath)) return null;
    const path = require('path');
    const { spawnSync } = require('child_process');
    try {
        const resolved = path.resolve(binPath.trim());
        const res = spawnSync(resolved, ['-version'], {
            encoding: 'utf8',
            timeout: 4000,
            windowsHide: true,
            shell: false,
        });
        if (res.error || (typeof res.status === 'number' && res.status !== 0)) {
            return null;
        }
        const parsed = parseJavaVersionOutput(`${res.stderr || ''}\n${res.stdout || ''}`);
        return { path: resolved, ...parsed };
    } catch {
        return null;
    }
}

/**
 * Construit un libellé lisible pour un JDK détecté ou sélectionné.
 * @param {{version?: string|null, major?: number|null, vendor?: string|null, path?: string}} info
 * @returns {string}
 */
function formatJavaLabel(info) {
    if (!info) return 'Java inconnu';
    const ver = info.version ? `Java ${info.version}` : (info.major ? `Java ${info.major}` : 'Java');
    const vend = info.vendor ? ` (${info.vendor})` : '';
    const suffix = info.major === 17 ? ' — Recommandé (JDK 17)' : (info.major ? ` — JDK ${info.major}` : '');
    return `${ver}${vend}${suffix}`;
}

/**
 * Résout l'option `java` passée à `minecraft-java-core-azbetter` à partir du chemin IndexedDB.
 * Si un chemin valide est configuré, il remplace `configJava.path`. Sinon `configJava` est conservé.
 * @param {*} storedRecordValue Valeur du champ `.value` issu de la table `java-path`
 * @param {*} configJava Valeur de `this.config.java`
 * @param {(p: string) => boolean} [validator] Fonction de validation (injectable pour les tests)
 * @returns {{customPath: string|null, java: *}}
 */
function resolveLaunchJavaConfig(storedRecordValue, configJava, validator = isValidJavaBinary) {
    const raw = storedRecordValue && typeof storedRecordValue === 'object'
        ? storedRecordValue.path
        : (typeof storedRecordValue === 'string' ? storedRecordValue : null);
    const candidate = typeof raw === 'string' ? raw.trim() : '';
    if (candidate && validator(candidate)) {
        const base = (typeof configJava === 'object' && configJava !== null)
            ? configJava
            : { version: null, type: 'jre' };
        return {
            customPath: candidate,
            java: { ...base, path: candidate },
        };
    }
    return {
        customPath: null,
        java: configJava,
    };
}

/**
 * Scanne le système à la recherche des installations Java / JDK (en priorisant JDK 17).
 * @param {{gameDir?: string, probe?: boolean}} [options]
 * @returns {Array<{path: string, version: string|null, major: number|null, vendor: string|null, isJava17: boolean, label: string}>}
 */
function detectInstalledJdks(options = {}) {
    const fs = require('fs');
    const path = require('path');
    const isWin = process.platform === 'win32';
    const binNames = isWin ? ['javaw.exe', 'java.exe'] : ['java'];
    const candidates = [];
    const seenDirs = new Set();

    const addBinDir = (binDir) => {
        if (!binDir || typeof binDir !== 'string') return;
        const norm = isWin ? path.resolve(binDir).toLowerCase() : path.resolve(binDir);
        if (seenDirs.has(norm)) return;
        seenDirs.add(norm);
        for (const name of binNames) {
            const p = path.join(binDir, name);
            if (isValidJavaBinary(p)) {
                candidates.push(path.resolve(p));
                break;
            }
        }
    };

    const scanVendorParent = (parentDir, depth = 0) => {
        if (!parentDir || depth > 2) return;
        let entries;
        try {
            entries = fs.readdirSync(parentDir, { withFileTypes: true });
        } catch {
            return;
        }
        for (const entry of entries) {
            if (!entry.isDirectory()) continue;
            const full = path.join(parentDir, entry.name);
            addBinDir(path.join(full, 'bin'));
            if (process.platform === 'darwin') {
                addBinDir(path.join(full, 'Contents', 'Home', 'bin'));
            }
            if (depth < 1) {
                scanVendorParent(full, depth + 1);
            }
        }
    };

    if (process.env.JAVA_HOME) {
        addBinDir(path.join(process.env.JAVA_HOME, 'bin'));
    }

    if (process.env.PATH) {
        for (const entry of process.env.PATH.split(path.delimiter)) {
            if (entry) addBinDir(entry);
        }
    }

    if (isWin) {
        const roots = [
            process.env.ProgramFiles,
            process.env['ProgramFiles(x86)'],
            process.env.LOCALAPPDATA ? path.join(process.env.LOCALAPPDATA, 'Programs') : null,
        ].filter(Boolean);
        const vendors = [
            'Java',
            'Eclipse Adoptium',
            'AdoptOpenJDK',
            'Temurin',
            'Microsoft',
            'Zulu',
            'Amazon Corretto',
            'BellSoft',
            'SapMachine',
            'JetBrains',
            'Semeru',
        ];
        for (const root of roots) {
            for (const vendor of vendors) {
                scanVendorParent(path.join(root, vendor), 0);
            }
        }
    } else if (process.platform === 'darwin') {
        scanVendorParent('/Library/Java/JavaVirtualMachines', 0);
        const home = process.env.HOME;
        if (home) {
            scanVendorParent(path.join(home, 'Library', 'Java', 'JavaVirtualMachines'), 0);
            scanVendorParent(path.join(home, '.sdkman', 'candidates', 'java'), 0);
        }
    } else {
        scanVendorParent('/usr/lib/jvm', 0);
        scanVendorParent('/usr/java', 0);
        const home = process.env.HOME;
        if (home) {
            scanVendorParent(path.join(home, '.sdkman', 'candidates', 'java'), 0);
        }
    }

    if (options.gameDir) {
        scanVendorParent(path.join(options.gameDir, 'runtime'), 0);
    }

    const dedup = [];
    const seenPaths = new Set();
    for (const p of candidates) {
        const key = isWin ? p.toLowerCase() : p;
        if (seenPaths.has(key)) continue;
        seenPaths.add(key);
        const info = options.probe === false
            ? { path: p, version: null, major: null, vendor: null, isJava17: false }
            : (probeJavaVersion(p) || { path: p, version: null, major: null, vendor: null, isJava17: false });
        dedup.push({ ...info, label: formatJavaLabel(info) });
    }

    // Trie les JDK 17 en premier, puis les autres versions décroissantes.
    dedup.sort((a, b) => {
        if (a.isJava17 !== b.isJava17) return a.isJava17 ? -1 : 1;
        return (b.major || 0) - (a.major || 0);
    });

    return dedup;
}

export {
    isValidJavaBinarySyntax,
    isValidJavaBinary,
    parseJavaVersionOutput,
    probeJavaVersion,
    formatJavaLabel,
    resolveLaunchJavaConfig,
    detectInstalledJdks,
};
