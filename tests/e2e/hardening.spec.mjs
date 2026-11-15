import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { startMock } from './mock-panel.mjs';
import { launchLauncher, panelActive, gameDirFor, writeFile, ROOT } from './helpers.mjs';

const loadEsm = (rel) => import('data:text/javascript;base64,' + fs.readFileSync(path.join(ROOT, rel)).toString('base64'));
const { isValidJavaBinarySyntax, parseJavaVersionOutput, resolveLaunchJavaConfig } = await loadEsm('src/assets/js/utils/java.js');
const { analyzeCrashText, extractLogErrors } = await loadEsm('src/assets/js/utils/diagnostic.js');

const fatal = (logs) => logs
    .filter(l => l.type === 'pageerror' || /Uncaught|TypeError|ReferenceError|is not a function|Cannot read/.test(l.text))
    .map(l => l.text);

test.describe('Java / JDK : validation pure et résolution', () => {
    test('isValidJavaBinarySyntax rejette traversées, chemins réseau et non-java', () => {
        expect(isValidJavaBinarySyntax('C:\\Program Files\\Eclipse Adoptium\\jdk-17\\bin\\javaw.exe')).toBe(true);
        expect(isValidJavaBinarySyntax('C:/Java/bin/java.exe')).toBe(true);
        expect(isValidJavaBinarySyntax('/usr/lib/jvm/java-17-openjdk/bin/java')).toBe(true);

        for (const bad of [
            '', false, null, undefined, 123,
            'java.exe',
            'relative/bin/java',
            'C:\\Java\\..\\Windows\\System32\\cmd.exe',
            'C:\\Java\\..\\jdk-17\\bin\\javaw.exe',
            '\\\\fileserver\\share\\javaw.exe',
            '//fileserver/share/java',
            'file:///C:/Java/bin/javaw.exe',
            'C:\\Windows\\System32\\notepad.exe',
            'C:\\Java\\bin\\javaw.exe\0.txt',
        ]) {
            expect(isValidJavaBinarySyntax(bad), String(bad)).toBe(false);
        }
    });

    test('parseJavaVersionOutput identifie JDK 17 vs autres versions et fournisseurs', () => {
        const j17 = parseJavaVersionOutput('openjdk version "17.0.12" 2024-07-16\nOpenJDK Runtime Environment Temurin-17.0.12+7 (build 17.0.12+7)');
        expect(j17).toEqual({ version: '17.0.12', major: 17, vendor: 'Eclipse Adoptium', isJava17: true });

        const j21 = parseJavaVersionOutput('openjdk version "21.0.4" 2024-07-16\nCorretto-21.0.4.7.1');
        expect(j21.major).toBe(21);
        expect(j21.vendor).toBe('Amazon Corretto');
        expect(j21.isJava17).toBe(false);

        const j8 = parseJavaVersionOutput('java version "1.8.0_401"');
        expect(j8.major).toBe(8);
        expect(j8.isJava17).toBe(false);
    });

    test('resolveLaunchJavaConfig transmet java-path valide et replie sur config.java si invalide', () => {
        const okValidator = (p) => p === 'C:\\JDK17\\bin\\javaw.exe';
        const resOk = resolveLaunchJavaConfig({ uuid: '1234', path: 'C:\\JDK17\\bin\\javaw.exe' }, false, okValidator);
        expect(resOk.customPath).toBe('C:\\JDK17\\bin\\javaw.exe');
        expect(resOk.java).toEqual({ version: null, type: 'jre', path: 'C:\\JDK17\\bin\\javaw.exe' });

        const resObj = resolveLaunchJavaConfig({ uuid: '1234', path: 'C:\\JDK17\\bin\\javaw.exe' }, { version: 17, type: 'jdk' }, okValidator);
        expect(resObj.java).toEqual({ version: 17, type: 'jdk', path: 'C:\\JDK17\\bin\\javaw.exe' });

        const resFallback = resolveLaunchJavaConfig({ uuid: '1234', path: false }, false, okValidator);
        expect(resFallback.customPath).toBeNull();
        expect(resFallback.java).toBe(false);

        const resBad = resolveLaunchJavaConfig({ uuid: '1234', path: 'C:\\bad\\notepad.exe' }, { version: 17, type: 'jre' }, okValidator);
        expect(resBad.customPath).toBeNull();
        expect(resBad.java).toEqual({ version: 17, type: 'jre' });
    });
});

test.describe('Crash Reporter : analyse et masquage', () => {
    test('analyzeCrashText reconnaît OOM, mod manquant, Java incompatible, Mixin, GPU et fallback', () => {
        const oom = analyzeCrashText('java.lang.OutOfMemoryError: Java heap space\n\tat net.minecraft.Main.main', -1);
        expect(oom.cause).toBe('oom');
        expect(oom.title).toContain('Mémoire insuffisante');

        const javaVer = analyzeCrashText('java.lang.UnsupportedClassVersionError: net/minecraftforge/fml/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0)', 1);
        expect(javaVer.cause).toBe('java_version');
        expect(javaVer.title).toContain('Java incompatible');

        const missingMod = analyzeCrashText('Missing or unsupported mandatory dependencies:\n\tMod ID: \'geckolib\', Requested by: \'alexsmobs\'', 1);
        expect(missingMod.cause).toBe('missing_mod');

        const modReq = analyzeCrashText('Mod "create" requires flywheel 0.6.11 or above', 1);
        expect(modReq.cause).toBe('missing_mod');
        expect(modReq.summary).toContain('create requiert flywheel');

        const mixin = analyzeCrashText('org.spongepowered.asm.mixin.transformer.throwables.MixinTransformerError: An unexpected critical error was encountered\nSuspected Mods: Rubidium', 1);
        expect(mixin.cause).toBe('mod_conflict');
        expect(mixin.summary).toContain('Rubidium');

        const gpu = analyzeCrashText('org.lwjgl.glfw.GLFWError: WGL: The driver does not appear to support OpenGL', -1073740791);
        expect(gpu.cause).toBe('graphics');

        const generic = analyzeCrashText('[main/FATAL] Something unexpected happened', 42);
        expect(generic.cause).toBe('unknown');
        expect(generic.title).toContain('42');
        expect(extractLogErrors('[INFO] ok\n[ERROR] boom\n\tat Foo.bar')).toContain('boom');
    });
});

test.describe('Electron E2E : JDK selector, java-path, Crash Reporter & Télémétrie', () => {
    let mock;
    test.beforeAll(async () => { mock = await startMock(8793); });
    test.afterAll(async () => { await mock.close(); });

    test('sanitizeTelemetryExtra supprime clés sensibles, jetons, emails et chemins', async () => {
        const { win, close } = await launchLauncher(mock);
        try {
            const clean = await win.evaluate(async () => {
                const { sanitizeTelemetryExtra } = await import('./assets/js/utils/telemetry.js');
                return sanitizeTelemetryExtra({
                    exitCode: 1,
                    cause: 'oom',
                    access_token: 'super-secret-token',
                    password: 'my-password',
                    email: 'player@example.com',
                    refreshToken: 'abc',
                    message: 'Auth failed for player@example.com with Bearer eyJhbGciOi.123 in C:\\Users\\Bob\\AppData\\tok',
                    event: 'hijack',
                    serverId: 'hijack',
                });
            });
            expect(clean.exitCode).toBe(1);
            expect(clean.cause).toBe('oom');
            expect(clean.access_token).toBeUndefined();
            expect(clean.password).toBeUndefined();
            expect(clean.email).toBeUndefined();
            expect(clean.refreshToken).toBeUndefined();
            expect(clean.event).toBeUndefined();
            expect(clean.serverId).toBeUndefined();
            expect(clean.message).not.toContain('player@example.com');
            expect(clean.message).not.toContain('eyJhbGciOi');
            expect(clean.message).not.toContain('Bob');
            expect(clean.message).toContain('[MASQUÉ]');
        } finally {
            await close();
        }
    });

    test('Settings JDK selector + Home getLaunchOptions respecte java-path', async () => {
        const { app, win, logs, tmp, close } = await launchLauncher(mock);
        try {
            // Crée un faux binaire Java sur disque dans le répertoire isolé du test
            const fakeBinName = process.platform === 'win32' ? 'javaw.exe' : 'java';
            const fakeJava = path.join(tmp, 'jdk-17-custom', 'bin', fakeBinName);
            writeFile(fakeJava, 'stub');

            // Va sur Paramètres > Avancé et vérifie la section Java / JDK
            await win.locator('#settings-btn').click();
            await panelActive(win, 'settings');
            await win.locator('#advanced-tab').click();
            await expect(win.locator('#java-section')).toBeVisible();

            await win.locator('#java-detect-btn').click();
            await expect(win.locator('#java-status')).toBeVisible();

            // Injecte notre binaire dans le sélecteur / IndexedDB et vérifie la persistance via Home.getLaunchOptions
            await win.evaluate(async (p) => {
                const select = document.getElementById('java-jdk-select');
                const opt = document.createElement('option');
                opt.value = p;
                opt.textContent = 'JDK 17 Test';
                select.appendChild(opt);
                select.value = p;
                select.dispatchEvent(new Event('change', { bubbles: true }));
            }, fakeJava);

            await expect(win.locator('#java-path-input')).toHaveValue(fakeJava);

            // Vérifie que Home.getLaunchOptions() transmet désormais bien le java-path personnalisé
            const opts = await win.evaluate(async () => {
                const { default: Home } = await import('./assets/js/panels/home.js');
                const { database } = await import('./assets/js/utils.js');
                const h = new Home();
                h.database = await new database().init();
                h.config = { java: false, dataDirectory: 'geoventure-e2e', ignored: [], verify: false, game_version: '1.20.1', loader: { type: 'forge', build: '47.4.20', enable: true } };
                h.writeLauncherLog = () => {};
                return await h.getLaunchOptions();
            });
            expect(opts.java).toBeTruthy();
            expect(opts.java.path).toBe(fakeJava);

            // Réinitialisation remet java-path à false et opts.java à config.java
            await win.locator('#java-reset-btn').click();
            await expect(win.locator('#java-path-input')).toHaveValue('');
            const optsAfterReset = await win.evaluate(async () => {
                const { default: Home } = await import('./assets/js/panels/home.js');
                const { database } = await import('./assets/js/utils.js');
                const h = new Home();
                h.database = await new database().init();
                h.config = { java: false, dataDirectory: 'geoventure-e2e', ignored: [], verify: false, game_version: '1.20.1', loader: { type: 'forge', build: '47.4.20', enable: true } };
                h.writeLauncherLog = () => {};
                return await h.getLaunchOptions();
            });
            expect(optsAfterReset.java).toBe(false);

            // Aucune erreur fatale dans la console Electron
            expect(fatal(logs), logs.map(l => l.text).join('\n')).toEqual([]);
        } finally {
            await close();
        }
    });

    test('Crash Reporter modal sur code !== 0, copie presse-papiers & télémétrie opt-in sans fuite', async () => {
        mock.clearHits();
        const { app, win, logs, home, close } = await launchLauncher(mock, { telemetry: true });
        try {
            await panelActive(win, 'home');
            const gd = gameDirFor(home);
            const crashFile = path.join(gd, 'crash-reports', 'crash-2026-11-15-e2e.txt');
            writeFile(crashFile, [
                '---- Minecraft Crash Report ----',
                '// OOM test',
                'Time: 2026-11-15 10:00:00',
                'Description: Rendering overlay',
                'java.lang.OutOfMemoryError: Java heap space',
                '--accessToken tok-e2e',
            ].join('\n'));

            // 0. Vérifie collectCrashReport : lecture crash-reports récent + repli latest.log si crash antérieur à launchStartMs
            const gdCrashPath = path.join(gd, 'crash-reports', 'crash-2026-11-15-e2e.txt');
            const gdLogPath = path.join(gd, 'logs', 'latest.log');
            writeFile(gdLogPath, '[12:00:00] [main/ERROR]: Fallback log error\n');
            const crashCheck = await win.evaluate(async (dir) => {
                const { collectCrashReport } = await import('./assets/js/utils/diagnostic.js');
                const recent = collectCrashReport({
                    gameDir: dir,
                    exitCode: 1,
                    launchStartMs: Date.now() - 60000,
                    logBuffer: ['console'],
                    pkg: { productName: 'Nexus', version: '1.11.2' },
                    config: { game_version: '1.20.1', loader: { type: 'forge', build: '47.4.20' } },
                    secrets: ['tok-e2e'],
                });
                const stale = collectCrashReport({
                    gameDir: dir,
                    exitCode: 2,
                    launchStartMs: Date.now() + 60000,
                    logBuffer: ['console'],
                    secrets: [],
                });
                return { recent, stale };
            }, gd);
            expect(crashCheck.recent.source).toBe('crash-report');
            expect(crashCheck.recent.sourceName).toBe('crash-2026-11-15-e2e.txt');
            expect(crashCheck.recent.report).not.toContain('tok-e2e');
            expect(crashCheck.stale.source).toBe('latest.log');
            expect(crashCheck.stale.excerpt).toContain('Fallback log error');

            // 1. Fermeture normale code=0 ne doit PAS ouvrir la modale de crash
            await win.evaluate(async () => {
                const { default: Home } = await import('./assets/js/panels/home.js');
                const { database } = await import('./assets/js/utils.js');
                const h = new Home();
                h.database = await new database().init();
                h.config = { dataDirectory: 'geoventure-e2e', ignored: [], game_version: '1.20.1', loader: { type: 'forge', build: '47.4.20' } };
                h.logBuffer = [];
                h.writeLauncherLog = () => {};
                await h.handleLaunchClose(0, document.querySelector('.text-download'), document.querySelector('.progress-bar'), document.querySelector('.play-btn'), { launcher: { close: 'keep' } });
            });
            await expect(win.locator('#crash-modal-overlay')).toBeHidden();

            // 2. Crash code=1 doit ouvrir #crash-modal-overlay, masquer tok-e2e, copier et envoyer minecraft_crash
            await win.evaluate(async () => {
                const { default: Home } = await import('./assets/js/panels/home.js');
                const { database } = await import('./assets/js/utils.js');
                const h = new Home();
                h.database = await new database().init();
                h.config = { dataDirectory: 'geoventure-e2e', ignored: [], game_version: '1.20.1', loader: { type: 'forge', build: '47.4.20' } };
                h.logBuffer = ['[FATAL] OOM'];
                h._launchStartMs = Date.now() - 5000;
                h.writeLauncherLog = () => {};
                await h.handleLaunchClose(1, document.querySelector('.text-download'), document.querySelector('.progress-bar'), document.querySelector('.play-btn'), { launcher: { close: 'keep' } });
            });

            await expect(win.locator('#crash-modal-overlay')).toBeVisible();
            await expect(win.locator('#crash-exit-badge')).toContainText('1');
            await expect(win.locator('#crash-cause-title')).toContainText('Mémoire insuffisante');
            const excerpt = await win.locator('#crash-excerpt').textContent();
            expect(excerpt).not.toContain('tok-e2e');
            expect(excerpt).toContain('[MASQUÉ]');

            // Clic Copier le diagnostic
            await win.locator('#crash-copy-btn').click();
            const clip = await app.evaluate(({ clipboard }) => clipboard.readText());
            expect(clip).toContain('Mémoire insuffisante');
            expect(clip).not.toContain('tok-e2e');
            expect(clip).toContain('[MASQUÉ]');

            await win.locator('#crash-close-btn').click();
            await expect(win.locator('#crash-modal-overlay')).toBeHidden();

            // Vérifie l'événement telemetry POST /utils/telemetry
            await expect.poll(() => {
                const telHits = mock.hits.filter(h => h.path === '/utils/telemetry');
                return telHits.map(h => JSON.parse(h.body)).find(b => b.event === 'minecraft_crash');
            }).toMatchObject({ event: 'minecraft_crash', exitCode: 1, cause: 'oom', category: 'error' });

            const crashHit = mock.hits
                .filter(h => h.path === '/utils/telemetry')
                .map(h => JSON.parse(h.body))
                .find(b => b.event === 'minecraft_crash');
            expect(JSON.stringify(crashHit)).not.toContain('tok-e2e');

            // 3. Test azuriom_auth_error via Login (aucune fuite de mot de passe ou email)
            mock.clearHits();
            mock.setAuthMode('html502');
            await win.evaluate(async () => {
                const { default: Login } = await import('./assets/js/panels/login.js');
                const l = new Login();
                const el = {
                    mailInput: { value: 'secret-user@example.com' },
                    passwordInput: { value: 'SuperSecretPass!123' },
                    infoLogin: document.createElement('div'),
                    infoLogin2f: document.createElement('div'),
                    loginBtn: document.createElement('button'),
                    cancelLogin: document.createElement('button'),
                };
                l.enableLoginForm = () => {};
                await l.handleLogin(el, `${localStorage.getItem('geoventure_server_url')}api/auth`);
            });

            await expect.poll(() => {
                return mock.hits
                    .filter(h => h.path === '/utils/telemetry')
                    .map(h => JSON.parse(h.body))
                    .find(b => b.event === 'azuriom_auth_error');
            }).toMatchObject({ event: 'azuriom_auth_error', stage: 'login', category: 'error' });

            const authHit = mock.hits
                .filter(h => h.path === '/utils/telemetry')
                .map(h => h.body)
                .find(b => b.includes('azuriom_auth_error'));
            expect(authHit).not.toContain('SuperSecretPass!123');
            expect(authHit).not.toContain('secret-user@example.com');

            // 4. Test forge_patch_error et modpack_error via setupLaunchListeners + sans consentement = zéro envoi
            mock.clearHits();
            await win.evaluate(async () => {
                const { EventEmitter } = require('events');
                const { default: Home } = await import('./assets/js/panels/home.js');
                const h = new Home();
                h._resetProgressTracking();
                h.config = { dataDirectory: 'geoventure-e2e', ignored: [] };
                h.appendLog = () => {};
                h.writeLauncherLog = () => {};
                const fakeLaunch = new EventEmitter();
                const info = document.querySelector('.text-download');
                const bar = document.querySelector('.progress-bar');
                const btn = document.querySelector('.play-btn');
                h.setupLaunchListeners(fakeLaunch, info, bar, btn, { launcher: { close: 'keep' } });

                h._setPhase('patch');
                fakeLaunch.emit('error', { error: 'Forge processor MERGE_MAPPING failed for token=secret_patch_tok' });

                h._setPhase('download');
                fakeLaunch.emit('error', { error: 'SHA1 hash mismatch on modpack.zip' });
            });

            await expect.poll(() => mock.hits.filter(h => h.path === '/utils/telemetry').map(h => JSON.parse(h.body).event))
                .toEqual(expect.arrayContaining(['forge_patch_error', 'modpack_error']));
            const patchHit = mock.hits.filter(h => h.path === '/utils/telemetry').find(h => h.body.includes('forge_patch_error'));
            expect(patchHit.body).not.toContain('secret_patch_tok');

            // Opt-out strict : lorsque telemetry_consent !== 'true', rien n'est envoyé
            mock.clearHits();
            await win.evaluate(async () => {
                const { setConsent, sendErrorEvent } = await import('./assets/js/utils/telemetry.js');
                setConsent(false);
                await sendErrorEvent('modpack_error', { message: 'should not be sent' });
            });
            expect(mock.hits.filter(h => h.path === '/utils/telemetry')).toEqual([]);

            expect(fatal(logs), logs.map(l => l.text).join('\n')).toEqual([]);
        } finally {
            mock.setAuthMode('ok');
            await close();
        }
    });
});
