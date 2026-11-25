//! Analyse d'un plantage du jeu : cause probable et conseil en clair pour le joueur.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CrashCause {
    /// Identifiant stable (pour l'interface et les statistiques).
    pub id: &'static str,
    pub title: &'static str,
    pub advice: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrashAnalysis {
    pub exit_code: Option<i32>,
    pub cause: Option<CrashCause>,
    /// Rapport de plantage écrit par le jeu pendant cette partie, s'il y en a un.
    pub crash_report: Option<String>,
    /// Extrait (redacté) qui a permis de conclure.
    pub excerpt: String,
}

/// Règles dans l'ordre : la première trouvée l'emporte (la plus précise d'abord).
const RULES: &[(&[&str], CrashCause)] = &[
    // Mémoire du PC épuisée (RAM et fichier d'échange), à ne pas confondre avec un tas Java trop petit :
    // ici, donner plus de RAM au jeu aggrave le problème.
    (
        &[
            "insufficient memory for the Java Runtime Environment",
            "Native memory allocation (",
            "Failed to commit memory (",
        ],
        CrashCause {
            id: "system_memory",
            title: "Le PC n'a plus de mémoire libre",
            advice: "Fermez les autres programmes (navigateur, Discord…) puis relancez. Si ça recommence, baissez la RAM du jeu dans Performance et libérez de la place sur le disque système (fichier d'échange de Windows).",
        },
    ),
    (
        &["Could not reserve enough space", "Invalid maximum heap size", "Initial heap size set to a larger value"],
        CrashCause {
            id: "ram_reserve",
            title: "Java n'a pas pu réserver la mémoire demandée",
            advice: "La RAM réglée est trop haute pour ce PC (ou Java est en 32 bits). Choisissez le profil Économe ou baissez la RAM dans Performance.",
        },
    ),
    (
        &["java.lang.OutOfMemoryError", "Out of memory", "GC overhead limit exceeded"],
        CrashCause {
            id: "out_of_memory",
            title: "Le jeu a manqué de mémoire",
            advice: "Augmentez la RAM allouée dans Performance (6 Go conseillés), fermez les autres programmes, ou retirez des packs de ressources lourds.",
        },
    ),
    (
        &["UnsupportedClassVersionError", "has been compiled by a more recent version of the Java Runtime"],
        CrashCause {
            id: "java_version",
            title: "Mauvaise version de Java",
            advice: "Le Java personnalisé ne convient pas. Videz le champ « Java » dans les réglages : le launcher installera le Java 17 qu'il faut.",
        },
    ),
    (
        &["DuplicateModsFoundException", "Found duplicate mods", "Duplicate mods found"],
        CrashCause {
            id: "duplicate_mods",
            title: "Un mod est installé en double",
            advice: "Retirez le mod ajouté à la main dans le dossier mods, puis utilisez « Réparer » dans les réglages.",
        },
    ),
    (
        &["Missing or unsupported mandatory dependencies", "ModResolutionException"],
        CrashCause {
            id: "mod_dependency",
            title: "Un mod n'a pas ce qu'il lui faut",
            advice: "Un mod ajouté à la main ou un mod communauté n'est pas compatible. Désactivez vos mods ajoutés, puis « Réparer ».",
        },
    ),
    (
        &["Mixin apply failed", "MixinApplyError", "InvalidMixinException", "Mixin transformation of"],
        CrashCause {
            id: "mixin",
            title: "Conflit entre deux mods",
            advice: "Désactivez les mods facultatifs et communauté que vous avez ajoutés, puis relancez. Si ça continue, envoyez un diagnostic.",
        },
    ),
    (
        &["Pixel format not accelerated", "GLFW error 65542", "WGL: The driver does not appear to support OpenGL", "No OpenGL context", "Failed to create window", "GLFW error 65543"],
        CrashCause {
            id: "gpu_driver",
            title: "Pilote de carte graphique trop ancien ou absent",
            advice: "Mettez à jour le pilote de votre carte graphique (site de NVIDIA, AMD ou Intel). Sur un portable, lancez le jeu sur la carte graphique dédiée.",
        },
    ),
    (
        &["EXCEPTION_ACCESS_VIOLATION", "SIGSEGV", "A fatal error has been detected by the Java Runtime"],
        CrashCause {
            id: "native_crash",
            title: "Plantage du pilote graphique ou de Java",
            advice: "Mettez à jour le pilote de la carte graphique et retirez les shaders. Si ça recommence, envoyez un diagnostic.",
        },
    ),
    (
        &["No space left on device", "There is not enough space on the disk"],
        CrashCause {
            id: "disk_full",
            title: "Le disque est plein",
            advice: "Libérez de la place sur le disque (au moins 5 Go), puis relancez.",
        },
    ),
    (
        &["ShaderCompileException", "Couldn't compile shader", "Failed to load shader"],
        CrashCause {
            id: "shader",
            title: "Les shaders ou un pack de ressources ont fait planter le jeu",
            advice: "Désactivez les shaders et les packs de ressources récents, puis relancez.",
        },
    ),
];

/// Cherche la cause la plus probable dans un texte (rapport de plantage ou fin du journal).
pub fn analyze_text(text: &str) -> Option<(CrashCause, String)> {
    for (needles, cause) in RULES {
        for needle in needles.iter() {
            if let Some(line) = text.lines().find(|l| l.contains(needle)) {
                return Some((cause.clone(), line.trim().chars().take(300).collect()));
            }
        }
    }
    None
}

/// Rapport de plantage le plus récent écrit après `since`.
pub fn crash_report_since(game_dir: &Path, since: SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(game_dir.join("crash-reports"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?.modified().ok()?;
            (m >= since).then(|| (m, e.path()))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}

/// Fichiers `hs_err_pid*.log` (plantage natif de Java) écrits après `since`.
fn native_crash_since(game_dir: &Path, since: SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(game_dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("hs_err_pid"))
        .filter_map(|e| {
            let m = e.metadata().ok()?.modified().ok()?;
            (m >= since).then(|| (m, e.path()))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}

/// Analyse après une fermeture anormale : rapport de plantage, plantage natif, puis fin du journal.
pub fn analyze_exit(game_dir: &Path, since: SystemTime, exit_code: Option<i32>, recent_log: &str) -> CrashAnalysis {
    let report = crash_report_since(game_dir, since).or_else(|| native_crash_since(game_dir, since));
    let mut found = None;
    if let Some(r) = &report {
        if let Ok(text) = std::fs::read_to_string(r) {
            found = analyze_text(&text);
        }
    }
    if found.is_none() {
        let latest = crate::extras::tail_file(&game_dir.join("logs/latest.log"), 400).unwrap_or_default();
        found = analyze_text(recent_log).or_else(|| analyze_text(&latest));
    }
    let (cause, excerpt) = match found {
        Some((c, e)) => (Some(c), e),
        None => (None, String::new()),
    };
    CrashAnalysis {
        exit_code,
        cause,
        crash_report: report.map(|p| p.to_string_lossy().to_string()),
        excerpt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_out_of_memory() {
        let (c, line) = analyze_text("[12:00] foo\nException in thread \"main\" java.lang.OutOfMemoryError: Java heap space\n").unwrap();
        assert_eq!(c.id, "out_of_memory");
        assert!(line.contains("OutOfMemoryError"));
    }

    #[test]
    fn reserve_wins_over_generic() {
        let (c, _) = analyze_text("Error occurred during initialization of VM\nCould not reserve enough space for 8388608KB object heap").unwrap();
        assert_eq!(c.id, "ram_reserve");
    }

    #[test]
    fn vague_requires_is_ignored() {
        assert!(analyze_text("This feature requires a restart").is_none());
        let (c, _) = analyze_text("Missing or unsupported mandatory dependencies:\n\tMod ID: 'geckolib', Requested by: 'geomods'").unwrap();
        assert_eq!(c.id, "mod_dependency");
    }

    #[test]
    fn system_memory_is_not_blamed_on_a_mod() {
        // Journal réel d'un PC à court de mémoire, avec l'avertissement normal de Forge au démarrage.
        let log = "[main/WARN] [JarSelector/]: Attempted to select a dependency jar for JarJar which was passed in as source: geckolib. Using Mod File: geckolib.jar
                   [30.004s][error][gc] Failed to commit memory (1455)
                   # There is insufficient memory for the Java Runtime Environment to continue.";
        assert_eq!(analyze_text(log).unwrap().0.id, "system_memory");
        let native = "# Native memory allocation (malloc) failed to allocate 1069392 bytes. Error detail: Chunk::new";
        assert_eq!(analyze_text(native).unwrap().0.id, "system_memory");
    }

    #[test]
    fn jar_selector_warning_is_not_a_crash_cause() {
        assert!(analyze_text("[JarSelector/]: Attempted to select a dependency jar. Using Mod File: geckolib.jar").is_none());
    }

    #[test]
    fn detects_gpu_driver() {
        let (c, _) = analyze_text("[Render thread/ERROR]: GLFW error 65542: WGL: The driver does not appear to support OpenGL").unwrap();
        assert_eq!(c.id, "gpu_driver");
    }

    #[test]
    fn finds_report_written_after_launch() {
        let tmp = tempfile::tempdir().unwrap();
        let since = SystemTime::now() - std::time::Duration::from_secs(5);
        std::fs::create_dir_all(tmp.path().join("crash-reports")).unwrap();
        std::fs::write(
            tmp.path().join("crash-reports/crash-2026.txt"),
            "---- Minecraft Crash Report ----\njava.lang.UnsupportedClassVersionError: x",
        )
        .unwrap();
        let a = analyze_exit(tmp.path(), since, Some(1), "");
        assert_eq!(a.cause.unwrap().id, "java_version");
        assert!(a.crash_report.unwrap().ends_with("crash-2026.txt"));
    }

    #[test]
    fn no_cause_when_nothing_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let a = analyze_exit(tmp.path(), SystemTime::now(), Some(1), "rien de spécial");
        assert!(a.cause.is_none());
    }
}
