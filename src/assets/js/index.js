/**
 * @author Luuxis
 * Licensed under CC BY-NC 4.0
 * https://creativecommons.org/licenses/by-nc/4.0/
 *
 * Edited by CentralCorp Team
 */
'use strict';
// Fenêtre isolée (sandbox, sans Node) : tout passe par les fonctions du preload (windows/updatePreload.js).
const api = window.nexusUpdate;

// Les messages d'erreur (réseau, panel) sont du TEXTE : jamais interprétés comme du HTML dans setStatus.
const esc = (s) => String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

// Données du processus principal (plateforme, dépôt des releases, traductions) : lues une seule fois.
const boot = api.init((navigator.language || 'en').split('-')[0] || 'en');
const t = (key) => boot.then(b => (b.translations && b.translations[key]) || key);
const ls = (k) => { try { return localStorage.getItem(k); } catch { return null; } };

// Config du panel (récupérée par le processus principal). Même cache hors-ligne que utils/config.js :
// si le panel est injoignable, la dernière config valide de l'instance est reservie au lieu de bloquer.
function cacheKey() {
    const slug = ls('geoventure_selected_instance') || 'default';
    return 'geoventure_config_cache_' + slug + (ls('geoventure_selected_channel') === 'test' ? '_test' : '');
}
const config = {
    async GetConfig() {
        try {
            const res = await api.fetchConfig({
                serverUrl: ls('geoventure_server_url'),
                instance: ls('geoventure_selected_instance'),
                channel: ls('geoventure_selected_channel'),
            });
            if (!res || !res.ok) throw new Error((res && res.error) || 'configuration indisponible');
            try { localStorage.setItem(cacheKey(), JSON.stringify({ savedAt: Date.now(), config: res.config })); } catch { /* non bloquant */ }
            return res.config;
        } catch (error) {
            console.error("Failed to fetch config:", error);
            try {
                const parsed = JSON.parse(ls(cacheKey()));
                if (parsed && typeof parsed.config === 'object' && parsed.config !== null) {
                    console.warn('Panel injoignable : utilisation de la config en cache (mode hors-ligne).');
                    return parsed.config;
                }
            } catch { /* pas de cache */ }
            throw error;
        }
    }
};

class Splash {
    constructor() {
        this.splash = document.querySelector(".splash");
        this.splashMessage = document.querySelector(".splash-message");
        this.splashAuthor = document.querySelector(".splash-author");
        this.message = document.querySelector(".message");
        this.progress = document.querySelector("progress");
        document.addEventListener('DOMContentLoaded', async () => {
            if ((await boot).platform == 'win32') api.progressLoad()
            this.startAnimation();
        })
    }
    async startAnimation() {
        config.GetConfig().then(res => {
            let splashes = [
                { "message": res.splash, "author": res.splash_author },
            ];
            let splash = splashes[Math.floor(Math.random() * splashes.length)];
            this.splashMessage.textContent = splash.message;
            this.splashAuthor.children[0].textContent = "@" + splash.author;
        })

        document.getElementById('splash-message').textContent = await t('welcome_message');
        document.getElementById('splash-author').textContent = await t('developed_by');
        document.getElementById('update-message').textContent = await t('checking_updates');

        await sleep(100);
        document.querySelector("#splash").style.display = "block";
        await sleep(500);
        this.splash.classList.add("opacity");
        await sleep(500);
        this.splash.classList.add("translate");
        this.splashMessage.classList.add("opacity");
        this.splashAuthor.classList.add("opacity");
        this.message.classList.add("opacity");
        await sleep(1000);
        this.checkUpdate();
    }

    async checkUpdate() {
        this.setStatus(`Recherche de mise à jour...`);

        api.checkForUpdate().catch(err => {
            // A failed update check (no release feed, network down, unpacked dev
            // build…) must NOT block the launcher. Log it and continue startup.
            console.error('Recherche de mise à jour échouée :', err && err.message ? err.message : err);
            return this.maintenanceCheck();
        });

        api.onUpdateAvailable(async () => {
            this.setStatus(`Mise à jour disponible !`);
            // Windows : mise à jour automatique. Linux en AppImage : electron-updater sait aussi
            // remplacer l'AppImage (variable APPIMAGE posée par le lanceur) → même chemin, avec
            // barre de progression. Les autres cas (macOS, .deb, dossier décompressé) restent manuels.
            const b = await boot;
            const auto = b.platform == 'win32' || (b.platform == 'linux' && b.appImage);
            if (auto) {
                this.updateDownloading = true;
                this.toggleProgress();
                api.startUpdate();
            }
            else return this.dowloadUpdate();
        })

        api.onUpdateError((err) => {
            if (!err) return;
            // Une erreur de l'outil de mise à jour (ex. « APPIMAGE env is not defined » hors AppImage,
            // réseau, flux de releases absent) ne doit JAMAIS arrêter le launcher tant qu'aucun
            // téléchargement n'est en cours : on journalise et on continue le démarrage.
            console.error('Erreur de mise à jour :', err.message || err);
            if (this.updateDownloading) return this.shutdown(esc(err.message));
            return this.maintenanceCheck();
        })

        api.onDownloadProgress((progress) => {
            api.progress(progress.transferred, progress.total);
            this.setProgress(progress.transferred, progress.total);
        })

        api.onUpdateNotAvailable(() => {
            console.error("Mise à jour non disponible");
            this.maintenanceCheck();
        })
    }

    getLatestReleaseForOS(os, preferredFormat, asset) {
        return asset.filter(asset => {
            const name = asset.name.toLowerCase();
            const isOSMatch = name.includes(os);
            const isFormatMatch = name.endsWith(preferredFormat);
            return isOSMatch && isFormatMatch;
        }).sort((a, b) => new Date(b.created_at) - new Date(a.created_at))[0];
    }

    async dowloadUpdate() {
        // Ne JAMAIS laisser l'écran figé sur « Mise à jour disponible ! » : toute erreur (réseau, limite
        // de débit de l'API GitHub, asset absent) retombe sur la page des releases + bouton « Continuer ».
        const b = await boot;
        const repoURL = b.repositoryUrl.replace("git+", "").replace(".git", "").replace("https://github.com/", "").split("/");
        const releasesPage = `https://github.com/${repoURL[0]}/${repoURL[1]}/releases/latest`;
        let url = releasesPage;
        try {
            const res = await fetch(`https://api.github.com/repos/${repoURL[0]}/${repoURL[1]}/releases/latest`);
            if (res.ok) {
                const latestRelease = await res.json();
                const assets = Array.isArray(latestRelease.assets) ? latestRelease.assets : [];
                let found;
                if (b.platform == 'darwin') found = this.getLatestReleaseForOS('mac', '.dmg', assets);
                else if (b.platform == 'linux') found = this.getLatestReleaseForOS('linux', '.appimage', assets);
                if (found && found.browser_download_url) url = found.browser_download_url;
            } else {
                console.error("Releases GitHub indisponibles :", res.status);
            }
        } catch (err) {
            console.error("Impossible de récupérer les releases GitHub :", err && err.message ? err.message : err);
        }

        this.setStatus(`Mise à jour disponible !<br><div class="download-update">Télécharger</div><div class="download-update skip-update">Continuer</div>`);
        const dl = document.querySelector(".download-update:not(.skip-update)");
        const skip = document.querySelector(".skip-update");
        if (dl) dl.addEventListener("click", () => {
            api.openExternal(url);
            return this.shutdown("Téléchargement en cours...");
        });
        if (skip) skip.addEventListener("click", () => this.maintenanceCheck());
    }


    async maintenanceCheck() {
        if (this.started) return; // idempotent : erreur de mise à jour + « pas de mise à jour » ne démarrent qu'une fois
        this.started = true;
        // En maintenance, on n'arrête plus le launcher : on l'ouvre normalement
        // et le panneau d'accueil affiche le bandeau + bloque le bouton Jouer
        // (voir initMaintenance() dans panels/home.js). Seule une vraie erreur
        // de connexion au panel empêche le démarrage.
        config.GetConfig().then(() => {
            this.startLauncher();
        }).catch(e => {
            console.error(e);
            return this.shutdown(`Erreur de connexion:<br>${esc(e.message || String(e))}`);
        })
    }


    startLauncher() {
        this.setStatus(`Démarrage du launcher`);
        api.openMainWindow();
        api.closeWindow();
    }

    shutdown(text) {
        this.setStatus(`${text}<br>Arrêt dans 5s`);
        let i = 4;
        setInterval(() => {
            this.setStatus(`${text}<br>Arrêt dans ${i--}s`);
            if (i < 0) api.closeWindow();
        }, 1000);
    }

    setStatus(text) {
        this.message.innerHTML = text;
    }

    toggleProgress() {
        if (this.progress.classList.toggle("show")) this.setProgress(0, 1);
    }

    setProgress(value, max) {
        this.progress.value = value;
        this.progress.max = max;
    }
}

function sleep(ms) {
    return new Promise(r => setTimeout(r, ms));
}

document.addEventListener("keydown", (e) => {
    if (e.ctrlKey && e.shiftKey && e.keyCode == 73 || e.keyCode == 123) {
        api.devTools();
    }
})

new Splash();