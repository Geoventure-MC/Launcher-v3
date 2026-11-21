use crate::error::{LauncherError, LauncherResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JavaInstallation {
    pub path: String,
    pub version: Option<String>,
    pub major: Option<u32>,
    pub vendor: Option<String>,
    pub is_java17: bool,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedJavaVersion {
    pub version: Option<String>,
    pub major: Option<u32>,
    pub vendor: Option<String>,
    pub is_java17: bool,
}

fn is_java_executable_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "java" | "javaw" | "java.exe" | "javaw.exe"
    )
}

/// Validation pure de la syntaxe d'un chemin d'exécutable Java (sans accès disque).
/// Refuse les chemins relatifs, traversées `..`, chemins UNC réseau et caractères de contrôle.
pub fn is_valid_java_binary_syntax(bin_path: &str) -> bool {
    let trimmed = bin_path.trim();
    if trimmed.is_empty() || trimmed.len() > 1024 {
        return false;
    }
    if trimmed.chars().any(|c| matches!(c, '\0' | '\r' | '\n')) {
        return false;
    }
    if trimmed.starts_with("\\\\") || trimmed.starts_with("//") || trimmed.contains("://") {
        return false;
    }

    let bytes = trimmed.as_bytes();
    let is_windows_abs = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let is_unix_abs = trimmed.starts_with('/');
    if !is_windows_abs && !is_unix_abs {
        return false;
    }

    let segments: Vec<&str> = trimmed
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() || segments.iter().any(|s| *s == ".." || *s == ".") {
        return false;
    }

    segments
        .last()
        .map(|base| is_java_executable_name(base))
        .unwrap_or(false)
}

pub fn is_valid_java_binary(bin_path: &Path) -> bool {
    let as_str = bin_path.to_string_lossy();
    if !is_valid_java_binary_syntax(&as_str) {
        return false;
    }
    bin_path.is_file()
}

/// Parse la sortie de `java -version` pour en extraire la version majeure et le fournisseur.
pub fn parse_java_version_output(output: &str) -> ParsedJavaVersion {
    let lower = output.to_ascii_lowercase();
    let version = extract_quoted_version(output);

    let major = version.as_deref().and_then(|v| {
        let parts: Vec<&str> = v
            .split(['.', '_', '+', '-'])
            .filter(|s| !s.is_empty())
            .collect();
        if parts.first() == Some(&"1") {
            parts.get(1).and_then(|p| p.parse::<u32>().ok())
        } else {
            parts.first().and_then(|p| p.parse::<u32>().ok())
        }
    });

    let vendor = if lower.contains("temurin") || lower.contains("eclipse adoptium") {
        Some("Eclipse Adoptium")
    } else if lower.contains("adoptopenjdk") {
        Some("AdoptOpenJDK")
    } else if lower.contains("corretto") {
        Some("Amazon Corretto")
    } else if lower.contains("zulu") {
        Some("Azul Zulu")
    } else if lower.contains("microsoft") {
        Some("Microsoft")
    } else if lower.contains("graalvm") {
        Some("GraalVM")
    } else if lower.contains("liberica") || lower.contains("bellsoft") {
        Some("BellSoft Liberica")
    } else if lower.contains("jetbrains") {
        Some("JetBrains")
    } else if lower.contains("java(tm)") {
        Some("Oracle")
    } else if lower.contains("openjdk") {
        Some("OpenJDK")
    } else {
        None
    }
    .map(String::from);

    ParsedJavaVersion {
        version,
        major,
        vendor,
        is_java17: major == Some(17),
    }
}

fn extract_quoted_version(output: &str) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    let idx = lower.find("version")?;
    let rest = &output[idx + "version".len()..];
    let start = rest.find('"')? + 1;
    let tail = &rest[start..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

pub fn format_java_label(parsed: &ParsedJavaVersion) -> String {
    let ver = match (&parsed.version, parsed.major) {
        (Some(v), _) => format!("Java {v}"),
        (None, Some(m)) => format!("Java {m}"),
        (None, None) => "Java".to_string(),
    };
    let vend = parsed
        .vendor
        .as_deref()
        .map(|v| format!(" ({v})"))
        .unwrap_or_default();
    let suffix = match parsed.major {
        Some(17) => " — Recommandé (JDK 17)".to_string(),
        Some(m) => format!(" — JDK {m}"),
        None => String::new(),
    };
    format!("{ver}{vend}{suffix}")
}

/// Exécute `bin_path -version` sans shell afin de lire la version Java en toute sécurité.
pub fn probe_java_version(bin_path: &Path) -> Option<JavaInstallation> {
    if !is_valid_java_binary(bin_path) {
        return None;
    }

    let mut cmd = Command::new(bin_path);
    cmd.arg("-version");
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let parsed = parse_java_version_output(&combined);
    let label = format_java_label(&parsed);

    Some(JavaInstallation {
        path: bin_path.to_string_lossy().to_string(),
        version: parsed.version,
        major: parsed.major,
        vendor: parsed.vendor,
        is_java17: parsed.is_java17,
        label,
    })
}

/// Scanne le système à la recherche des installations Java/JDK (JDK 17 trié en priorité).
pub fn detect_installed_jdks(game_dir: Option<&Path>) -> Vec<JavaInstallation> {
    let bin_names: &[&str] = if cfg!(target_os = "windows") {
        &["javaw.exe", "java.exe"]
    } else {
        &["java"]
    };

    let mut seen_dirs = HashSet::<String>::new();
    let mut candidates = Vec::<PathBuf>::new();

    let mut add_bin_dir = |dir: PathBuf| {
        let key = if cfg!(target_os = "windows") {
            dir.to_string_lossy().to_ascii_lowercase()
        } else {
            dir.to_string_lossy().to_string()
        };
        if !seen_dirs.insert(key) {
            return;
        }
        for name in bin_names {
            let p = dir.join(name);
            if is_valid_java_binary(&p) {
                candidates.push(p);
                break;
            }
        }
    };

    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        add_bin_dir(PathBuf::from(java_home).join("bin"));
    }

    if let Ok(path_env) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path_env) {
            add_bin_dir(entry);
        }
    }

    let mut vendor_roots = Vec::<PathBuf>::new();
    if cfg!(target_os = "windows") {
        let roots = [
            std::env::var("ProgramFiles").ok().map(PathBuf::from),
            std::env::var("ProgramFiles(x86)").ok().map(PathBuf::from),
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|p| PathBuf::from(p).join("Programs")),
        ];
        let vendors = [
            "Java",
            "Eclipse Adoptium",
            "AdoptOpenJDK",
            "Temurin",
            "Microsoft",
            "Zulu",
            "Amazon Corretto",
            "BellSoft",
            "SapMachine",
            "JetBrains",
            "Semeru",
        ];
        for root in roots.into_iter().flatten() {
            for vendor in vendors {
                vendor_roots.push(root.join(vendor));
            }
        }
    } else if cfg!(target_os = "macos") {
        vendor_roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(home);
            vendor_roots.push(h.join("Library/Java/JavaVirtualMachines"));
            vendor_roots.push(h.join(".sdkman/candidates/java"));
        }
    } else {
        vendor_roots.push(PathBuf::from("/usr/lib/jvm"));
        vendor_roots.push(PathBuf::from("/usr/java"));
        if let Ok(home) = std::env::var("HOME") {
            vendor_roots.push(PathBuf::from(home).join(".sdkman/candidates/java"));
        }
    }

    if let Some(dir) = game_dir {
        vendor_roots.push(dir.join("runtime"));
    }

    for parent in vendor_roots {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    add_bin_dir(p.join("bin"));
                    if cfg!(target_os = "macos") {
                        add_bin_dir(p.join("Contents/Home/bin"));
                    }
                }
            }
        }
    }

    let mut results = Vec::new();
    let mut seen_paths = HashSet::<String>::new();

    for bin in candidates {
        let key = if cfg!(target_os = "windows") {
            bin.to_string_lossy().to_ascii_lowercase()
        } else {
            bin.to_string_lossy().to_string()
        };
        if !seen_paths.insert(key) {
            continue;
        }
        if let Some(info) = probe_java_version(&bin) {
            results.push(info);
        }
    }

    sort_jdks_java17_first(&mut results);
    results
}

pub fn sort_jdks_java17_first(jdks: &mut [JavaInstallation]) {
    jdks.sort_by(|a, b| {
        b.is_java17
            .cmp(&a.is_java17)
            .then_with(|| b.major.unwrap_or(0).cmp(&a.major.unwrap_or(0)))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Sélectionne le JDK 17 prioritaire (chemin personnalisé validé > JDK 17 détecté > erreur claire).
pub fn resolve_java17(
    custom_path: Option<&str>,
    game_dir: Option<&Path>,
) -> LauncherResult<JavaInstallation> {
    if let Some(custom) = custom_path.map(str::trim).filter(|s| !s.is_empty()) {
        let p = Path::new(custom);
        if let Some(info) = probe_java_version(p) {
            return Ok(info);
        }
        return Err(LauncherError::JavaNotFound(format!(
            "Chemin Java personnalisé invalide : {custom}"
        )));
    }

    let jdks = detect_installed_jdks(game_dir);
    if let Some(j17) = jdks.iter().find(|j| j.is_java17) {
        return Ok(j17.clone());
    }
    if let Some(first) = jdks.into_iter().next() {
        return Ok(first);
    }

    Err(LauncherError::JavaNotFound(
        "Aucun JDK 17 détecté. Installez Eclipse Temurin 17 pour lancer Forge 1.20.1.".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_binary_syntax_validation_blocks_unsafe_paths() {
        assert!(is_valid_java_binary_syntax(
            r"C:\Program Files\Eclipse Adoptium\jdk-17.0.12\bin\javaw.exe"
        ));
        assert!(is_valid_java_binary_syntax("/usr/lib/jvm/temurin-17/bin/java"));
        assert!(!is_valid_java_binary_syntax("java.exe"));
        assert!(!is_valid_java_binary_syntax(r"\\server\share\java.exe"));
        assert!(!is_valid_java_binary_syntax(r"C:\Java\..\cmd.exe"));
        assert!(!is_valid_java_binary_syntax(r"C:\Java\bin\powershell.exe"));
        assert!(!is_valid_java_binary_syntax("https://evil.example/java.exe"));
    }

    #[test]
    fn parse_java_version_extracts_major_and_vendor() {
        let temurin17 = r#"openjdk version "17.0.12" 2024-07-16
OpenJDK Runtime Environment Temurin-17.0.12+7 (build 17.0.12+7)"#;
        let p17 = parse_java_version_output(temurin17);
        assert_eq!(p17.major, Some(17));
        assert!(p17.is_java17);
        assert_eq!(p17.vendor.as_deref(), Some("Eclipse Adoptium"));

        let java8 = r#"java version "1.8.0_392"
Java(TM) SE Runtime Environment"#;
        let p8 = parse_java_version_output(java8);
        assert_eq!(p8.major, Some(8));
        assert!(!p8.is_java17);
        assert_eq!(p8.vendor.as_deref(), Some("Oracle"));
    }

    #[test]
    fn sort_jdks_places_java17_before_newer_or_older_versions() {
        let mut list = vec![
            JavaInstallation {
                path: "/jvm/21/bin/java".into(),
                version: Some("21.0.4".into()),
                major: Some(21),
                vendor: Some("OpenJDK".into()),
                is_java17: false,
                label: "Java 21".into(),
            },
            JavaInstallation {
                path: "/jvm/17/bin/java".into(),
                version: Some("17.0.12".into()),
                major: Some(17),
                vendor: Some("Eclipse Adoptium".into()),
                is_java17: true,
                label: "Java 17".into(),
            },
        ];
        sort_jdks_java17_first(&mut list);
        assert_eq!(list[0].major, Some(17));
    }
}
