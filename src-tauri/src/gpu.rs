//! Détection des cartes graphiques (diagnostic, conseils de performance).

use std::process::Command;
use std::sync::OnceLock;

static CACHE: OnceLock<Vec<String>> = OnceLock::new();

/// Noms des cartes graphiques du PC (mis en cache : la détection peut prendre une seconde).
pub fn detect() -> Vec<String> {
    CACHE.get_or_init(detect_uncached).clone()
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn detect_uncached() -> Vec<String> {
    #[cfg(windows)]
    {
        return run(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", "Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name }"],
        )
        .map(|s| parse_lines(&s))
        .unwrap_or_default();
    }
    #[cfg(target_os = "macos")]
    {
        return run("system_profiler", &["SPDisplaysDataType"])
            .map(|s| parse_macos(&s))
            .unwrap_or_default();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let found = run("lspci", &[]).map(|s| parse_lspci(&s)).unwrap_or_default();
        if !found.is_empty() {
            return found;
        }
        // Sans lspci : fabricant lu dans /sys (0x10de NVIDIA, 0x1002 AMD, 0x8086 Intel).
        let mut v: Vec<String> = std::fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("device/vendor")).ok())
            .filter_map(|id| vendor_name(id.trim()).map(String::from))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

pub fn vendor_name(pci_id: &str) -> Option<&'static str> {
    match pci_id.to_ascii_lowercase().as_str() {
        "0x10de" => Some("NVIDIA"),
        "0x1002" => Some("AMD"),
        "0x8086" => Some("Intel"),
        _ => None,
    }
}

pub fn parse_lines(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    v.dedup();
    v
}

pub fn parse_macos(s: &str) -> Vec<String> {
    s.lines()
        .filter_map(|l| l.trim().strip_prefix("Chipset Model:"))
        .map(|n| n.trim().to_string())
        .collect()
}

pub fn parse_lspci(s: &str) -> Vec<String> {
    s.lines()
        .filter(|l| l.contains("VGA compatible controller") || l.contains("3D controller") || l.contains("Display controller"))
        .filter_map(|l| l.splitn(3, ':').nth(2))
        .map(|n| n.trim().to_string())
        .collect()
}

/// `true` si toutes les cartes trouvées sont des puces intégrées modestes (Intel HD/UHD…).
pub fn only_integrated(gpus: &[String]) -> bool {
    !gpus.is_empty()
        && gpus.iter().all(|g| {
            let g = g.to_lowercase();
            (g.contains("intel") && !g.contains("arc")) || g.contains("microsoft basic") || g.contains("llvmpipe")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lspci() {
        let s = "00:02.0 VGA compatible controller: Intel Corporation UHD Graphics 620 (rev 07)\n01:00.0 3D controller: NVIDIA Corporation GP108M [GeForce MX150] (rev a1)\n00:1f.3 Audio device: Intel";
        let g = parse_lspci(s);
        assert_eq!(g.len(), 2);
        assert!(g[1].contains("GeForce MX150"));
        assert!(!only_integrated(&g));
        assert!(only_integrated(&g[..1]));
    }

    #[test]
    fn parses_macos() {
        let s = "Graphics/Displays:\n\n    Apple M1:\n\n      Chipset Model: Apple M1\n      Type: GPU\n";
        assert_eq!(parse_macos(s), vec!["Apple M1".to_string()]);
    }
}
