//! Typed format, rule and operation diagnostics; paths are data, never opened here.
use std::{path::PathBuf, sync::Arc};
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("theme file exceeds 8 MiB")]
    FileLimit,
    #[error("invalid Zed theme: {0}")]
    Parse(#[from] serde_json_lenient::Error),
    #[error("theme family has no valid themes")]
    NoVariants,
    #[error("invalid extension identity")]
    Identity,
    #[error("manifest id does not match directory")]
    ManifestIdentity,
    #[error("duplicate or reserved theme name: {0}")]
    Duplicate(String),
    #[error("invalid UTF-8 manifest: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("invalid manifest: {0}")]
    Manifest(#[from] toml::de::Error),
}
#[derive(Debug, thiserror::Error)]
pub enum ThemeOperationError {
    #[error("theme installation is unavailable")]
    Unsupported,
    #[error("theme operation completion is unavailable")]
    CompletionLost,
    #[error("this extension is already being changed")]
    Busy,
    #[error("too many pending theme operations")]
    Full,
    #[error("invalid extension identity")]
    Identity,
    #[error("registry response exceeds size limit ({limit} bytes)")]
    BodyLimit { limit: usize },
    #[error("Zed registry: {0}")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Format(#[from] ThemeError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{}: {operation}: {source}", path.display())]
    Filesystem {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not publish {}: {source}", path.display())]
    Publication {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is not a plain directory or file", .0.display())]
    NotPlain(PathBuf),
    #[error("theme file exceeds 8 MiB")]
    FileLimit,
    #[error("archive exceeds 10000 entries")]
    ArchiveEntries,
    #[error("archive exceeds 64 MiB decompressed")]
    ArchiveDecompressed,
    #[error("archive exceeds 256 theme files")]
    ArchiveFiles,
    #[error("archive contains no valid theme files")]
    EmptyArchive,
    #[error("could not encode extension manifest: {0}")]
    ManifestEncode(#[from] toml::ser::Error),
    #[error(
        "could not publish extension: {publication}; rollback failed: {rollback}; previous pack preserved at {}", recovery.display()
    )]
    Rollback {
        publication: std::io::Error,
        #[source]
        rollback: std::io::Error,
        recovery: PathBuf,
    },
}
#[derive(Debug, Clone)]
pub struct ThemeDiagnostic(Arc<ThemeOperationError>);
impl ThemeDiagnostic {
    pub fn new(error: ThemeOperationError) -> Self {
        Self(Arc::new(error))
    }
}
impl std::fmt::Display for ThemeDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0.as_ref(), f)
    }
}
impl std::error::Error for ThemeDiagnostic {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl ThemeOperationError {
    pub fn filesystem(
        path: impl Into<PathBuf>,
        operation: &'static str,
        source: std::io::Error,
    ) -> Self {
        Self::Filesystem {
            path: path.into(),
            operation,
            source,
        }
    }
}
