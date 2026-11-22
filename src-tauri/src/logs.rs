use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const DEFAULT_LOG_CAPACITY: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

/// Tampon circulaire thread-safe des journaux du launcher et de Minecraft.
#[derive(Debug, Clone)]
pub struct LogStore {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
    /// Copie sur le disque (`launcher.log`), pour retrouver ce qui s'est passé après coup.
    file: Arc<Mutex<Option<(std::fs::File, u64)>>>,
}

/// Taille maximale de `launcher.log` pour une session (au-delà, on n'écrit plus).
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

impl Default for LogStore {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_LOG_CAPACITY)
    }
}

impl LogStore {
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
            capacity: cap,
            file: Arc::new(Mutex::new(None)),
        }
    }

    /// Écrit désormais aussi dans `<dir>/launcher.log`. L'ancien fichier devient `launcher.old.log`.
    /// Les lignes déjà en mémoire y sont recopiées.
    pub fn attach_file(&self, dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        let path = dir.join("launcher.log");
        if path.exists() {
            let _ = std::fs::rename(&path, dir.join("launcher.old.log"));
        }
        let mut f = std::fs::File::create(&path)?;
        let mut written = 0u64;
        for e in self.list() {
            let line = format!("{}\n", Self::format_line(&e));
            written += line.len() as u64;
            f.write_all(line.as_bytes())?;
        }
        if let Ok(mut g) = self.file.lock() {
            *g = Some((f, written));
        }
        Ok(path)
    }

    fn format_line(e: &LogEntry) -> String {
        format!("[{}] [{}] [{}] {}", e.timestamp, e.level.as_str(), e.source, e.message)
    }

    pub fn push(&self, level: LogLevel, source: impl Into<String>, message: impl Into<String>) -> LogEntry {
        let entry = LogEntry {
            timestamp: Utc::now().format("%H:%M:%S%.3f").to_string(),
            level,
            source: source.into(),
            message: message.into(),
        };
        if let Ok(mut guard) = self.inner.lock() {
            if guard.len() >= self.capacity {
                guard.pop_front();
            }
            guard.push_back(entry.clone());
        }
        if let Ok(mut g) = self.file.lock() {
            if let Some((f, written)) = g.as_mut() {
                if *written < MAX_FILE_BYTES {
                    use std::io::Write;
                    let line = format!("{}\n", Self::format_line(&entry));
                    *written += line.len() as u64;
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
        entry
    }

    pub fn list(&self) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.clear();
        }
    }

    pub fn export_text(&self) -> String {
        self.list()
            .into_iter()
            .map(|e| Self::format_line(&e))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest_entries() {
        let store = LogStore::with_capacity(3);
        store.push(LogLevel::Info, "core", "msg-1");
        store.push(LogLevel::Warn, "core", "msg-2");
        store.push(LogLevel::Error, "core", "msg-3");
        store.push(LogLevel::Debug, "game", "msg-4");

        let entries = store.list();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "msg-2");
        assert_eq!(entries[2].message, "msg-4");
    }

    #[test]
    fn file_sink_copies_lines_and_rotates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LogStore::with_capacity(10);
        store.push(LogLevel::Info, "core", "avant");
        store.attach_file(tmp.path()).unwrap();
        store.push(LogLevel::Warn, "launch", "après");
        let txt = std::fs::read_to_string(tmp.path().join("launcher.log")).unwrap();
        assert!(txt.contains("avant") && txt.contains("[WARN] [launch] après"));
        store.attach_file(tmp.path()).unwrap();
        assert!(tmp.path().join("launcher.old.log").exists());
    }

    #[test]
    fn export_text_formats_lines() {
        let store = LogStore::with_capacity(10);
        store.push(LogLevel::Info, "launcher", "Démarrage OK");
        let txt = store.export_text();
        assert!(txt.contains("[INFO] [launcher] Démarrage OK"));
    }
}
