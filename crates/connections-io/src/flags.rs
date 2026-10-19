//! A downloaded flag is usable even when atomic cache publication fails.
use crate::http::{self, MAX_FLAG_BYTES};
use nocterm_connections_model::observation::{CountryCode, FlagRepository, ObservationError};
use std::{io::Read as _, path::PathBuf};

pub(crate) struct CachedFlags {
    directory: PathBuf,
    endpoint: Option<String>,
}
impl CachedFlags {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            endpoint: Some("https://flagcdn.com/".into()),
        }
    }
    #[cfg(test)]
    pub(crate) fn at(directory: PathBuf, endpoint: Option<String>) -> Self {
        Self {
            directory,
            endpoint,
        }
    }
}
impl FlagRepository for CachedFlags {
    fn load(&self, code: CountryCode) -> Result<Option<Vec<u8>>, ObservationError> {
        let path = self.directory.join(format!("{}.svg", code.as_str()));
        if let Ok(file) = std::fs::File::open(&path) {
            let mut bytes = Vec::new();
            if file
                .take(MAX_FLAG_BYTES + 1)
                .read_to_end(&mut bytes)
                .is_ok()
                && bytes.len() as u64 <= MAX_FLAG_BYTES
                && is_svg(&bytes)
            {
                return Ok(Some(bytes));
            }
        }
        let Some(endpoint) = &self.endpoint else {
            return Ok(None);
        };
        let bytes = http::get(&format!("{endpoint}{}.svg", code.as_str()), MAX_FLAG_BYTES)?;
        if !is_svg(&bytes) {
            return Err(ObservationError::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "flag response is not an SVG image",
            ))));
        }
        if let Err(source) = nocterm_core::fs::write_private_atomic(&path, &bytes) {
            let error = ObservationError::Cache(Box::new(source));
            tracing::warn!(error = ?error, "could not cache flag");
        }
        Ok(Some(bytes))
    }
}

/// Preserves the previous document sniff; this is not complete SVG sanitization.
fn is_svg(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).contains("<svg")
}

#[cfg(test)]
mod tests;
