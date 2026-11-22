use serde::Serialize;
use thiserror::Error;

/// Erreurs unifiées du backend Rust Geoventure.
#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("URL invalide ou non sécurisée : {0}")]
    InvalidUrl(String),

    #[error("Erreur réseau ({url}) : {message}")]
    Network { url: String, message: String },

    #[error("Réponse HTTP {status} sur {url}")]
    HttpStatus { status: u16, url: String },

    #[error("Délai réseau dépassé sur {0}")]
    Timeout(String),

    #[error("Chemin de fichier refusé (sécurité) : {0}")]
    UnsafePath(String),

    #[error("Empreinte {algorithm} invalide pour {path} (attendu {expected}, obtenu {actual})")]
    ChecksumMismatch {
        algorithm: &'static str,
        path: String,
        expected: String,
        actual: String,
    },

    #[error("Java 17 introuvable ou invalide : {0}")]
    JavaNotFound(String),

    #[error("Configuration de lancement invalide : {0}")]
    InvalidLaunchConfig(String),

    #[error("Erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),

    #[error("Erreur de sérialisation JSON : {0}")]
    Json(#[from] serde_json::Error),

    #[error("Erreur interne : {0}")]
    Internal(String),
}

#[derive(Debug, Serialize)]
struct SerializedError {
    code: &'static str,
    message: String,
}

impl LauncherError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl(_) => "INVALID_URL",
            Self::Network { .. } => "NETWORK_ERROR",
            Self::HttpStatus { .. } => "HTTP_STATUS",
            Self::Timeout(_) => "TIMEOUT",
            Self::UnsafePath(_) => "UNSAFE_PATH",
            Self::ChecksumMismatch { .. } => "CHECKSUM_MISMATCH",
            Self::JavaNotFound(_) => "JAVA_NOT_FOUND",
            Self::InvalidLaunchConfig(_) => "INVALID_LAUNCH_CONFIG",
            Self::Io(_) => "IO_ERROR",
            Self::Json(_) => "JSON_ERROR",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl Serialize for LauncherError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializedError {
            code: self.code(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

pub type LauncherResult<T> = Result<T, LauncherError>;
