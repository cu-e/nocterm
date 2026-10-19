//! Bounded remote reads with explicit best-effort close and Drop cancellation.
use nocterm_connections_model::{
    os::{self, OsId},
    os_observation::{OsFuture, OsObservationError, RemoteOsDetector},
};
use nocterm_session::{EntryKind, FsError, RemoteFs, fs::RemoteDownload};
use std::sync::Arc;
const MAX_OS_RELEASE_BYTES: usize = 16 * 1024;
pub struct NativeOsDetector;
impl RemoteOsDetector for NativeOsDetector {
    fn detect(&self, fs: Arc<dyn RemoteFs>) -> OsFuture {
        Box::pin(async move { detect(fs.as_ref()).await })
    }
}
fn optional(error: &FsError) -> bool {
    matches!(error, FsError::NotFound { .. } | FsError::Unsupported(_))
}
async fn detect(fs: &dyn RemoteFs) -> Result<Option<OsId>, OsObservationError> {
    let mut failure = None;
    let mut base = None;
    for path in ["/etc/os-release", "/usr/lib/os-release"] {
        match read_small(fs, path).await {
            Ok(Some(text)) => {
                base = os::from_os_release(&text);
                if base.is_some() {
                    break;
                }
            }
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    for (path, id) in os::platform_markers(base) {
        if marker(fs, path, &mut failure).await {
            return Ok(Some(id));
        }
    }
    if base.is_some() {
        return Ok(base);
    }
    for (path, id) in os::host_markers() {
        if marker(fs, path, &mut failure).await {
            return Ok(Some(id));
        }
    }
    match fs.home().await {
        Ok(home) if os::is_windows_home(&home) => return Ok(OsId::new("windows")),
        Ok(_) => {}
        Err(error) if optional(&error) => {}
        Err(error) => {
            failure.get_or_insert(OsObservationError::Home(error));
        }
    }
    failure.map_or(Ok(None), Err)
}
async fn marker(fs: &dyn RemoteFs, path: &str, failure: &mut Option<OsObservationError>) -> bool {
    match fs.stat(path).await {
        Ok(found) => found.is_some(),
        Err(error) if optional(&error) => false,
        Err(source) => {
            failure.get_or_insert(OsObservationError::Stat {
                path: path.into(),
                source,
            });
            false
        }
    }
}
async fn read_small(fs: &dyn RemoteFs, path: &str) -> Result<Option<String>, OsObservationError> {
    // Metadata identifies unsafe path shapes without parsing legacy Other strings.
    match fs.stat(path).await {
        Ok(Some(entry)) if entry.is_symlink || entry.kind != EntryKind::File => return Ok(None),
        Ok(None) => return Ok(None),
        Ok(Some(_)) => {}
        Err(FsError::Unsupported(_)) => {}
        Err(error) if optional(&error) => return Ok(None),
        Err(source) => {
            return Err(OsObservationError::Stat {
                path: path.into(),
                source,
            });
        }
    }
    let mut file = match fs.download(path).await {
        Ok(file) => file,
        Err(error) if optional(&error) => return Ok(None),
        Err(source) => {
            return Err(OsObservationError::Download {
                path: path.into(),
                source,
            });
        }
    };
    let result = read_bytes(file.as_mut(), path).await.and_then(|bytes| {
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|source| OsObservationError::InvalidUtf8 {
                path: path.into(),
                source,
            })
    });
    if let Err(source) = file.close().await {
        let error = OsObservationError::Close {
            path: path.into(),
            source,
        };
        tracing::debug!(%error, "could not close OS release reader");
    }
    result
}
async fn read_bytes(
    file: &mut dyn RemoteDownload,
    path: &str,
) -> Result<Vec<u8>, OsObservationError> {
    let mut bytes = Vec::new();
    loop {
        let requested = (MAX_OS_RELEASE_BYTES + 1 - bytes.len()).min(4096);
        let chunk = file
            .read(requested)
            .await
            .map_err(|source| OsObservationError::Read {
                path: path.into(),
                source,
            })?;
        if chunk.len() > requested {
            return Err(OsObservationError::InvalidChunk {
                path: path.into(),
                requested,
                returned: chunk.len(),
            });
        }
        if chunk.is_empty() {
            return Ok(bytes);
        }
        if bytes.len() + chunk.len() > MAX_OS_RELEASE_BYTES {
            return Err(OsObservationError::Oversize {
                path: path.into(),
                limit: MAX_OS_RELEASE_BYTES,
            });
        }
        bytes.extend(chunk);
    }
}
#[cfg(test)]
mod tests;
