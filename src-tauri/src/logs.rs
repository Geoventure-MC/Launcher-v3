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
}

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
        }
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
            .map(|e| format!("[{}] [{}] [{}] {}", e.timestamp, e.level.as_str(), e.source, e.message))
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
    fn export_text_formats_lines() {
        let store = LogStore::with_capacity(10);
        store.push(LogLevel::Info, "launcher", "Démarrage OK");
        let txt = store.export_text();
        assert!(txt.contains("[INFO] [launcher] Démarrage OK"));
    }
}
