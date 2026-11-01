use crate::{
    CollisionPolicy, DiscoveryLimits, TransferDirection, TransferError, valid_remote_source,
};
use nocterm_session::{HostAddress, RemoteFs, fs::path, fs::valid_publication_name};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone)]
pub struct UploadRequest {
    pub sources: Vec<PathBuf>,
    pub host: HostAddress,
    pub destination: String,
    pub fs: Arc<dyn RemoteFs>,
    pub collisions: CollisionPolicy,
}
#[derive(Clone)]
pub struct DownloadRequest {
    pub sources: Vec<String>,
    pub host: HostAddress,
    pub local_destination: PathBuf,
    pub fs: Arc<dyn RemoteFs>,
    pub collisions: CollisionPolicy,
}
/// Remote to remote copy relayed through this application. `host`, `destination`
/// and `fs` describe the destination, like the other requests; the `source_*`
/// fields describe where the sources are read.
#[derive(Clone)]
pub struct RelayRequest {
    pub source_host: HostAddress,
    pub source_fs: Arc<dyn RemoteFs>,
    pub sources: Vec<String>,
    pub host: HostAddress,
    pub destination: String,
    pub fs: Arc<dyn RemoteFs>,
    pub collisions: CollisionPolicy,
}
#[derive(Clone)]
pub enum Request {
    Upload(UploadRequest),
    Download(DownloadRequest),
    Relay(RelayRequest),
}
impl Request {
    /// The destination host; for a download this is the host being read.
    pub fn host(&self) -> &HostAddress {
        match self {
            Self::Upload(value) => &value.host,
            Self::Download(value) => &value.host,
            Self::Relay(value) => &value.host,
        }
    }
    pub fn destination(&self) -> String {
        match self {
            Self::Upload(value) => value.destination.clone(),
            Self::Download(value) => value.local_destination.display().to_string(),
            Self::Relay(value) => value.destination.clone(),
        }
    }
    pub fn direction(&self) -> TransferDirection {
        match self {
            Self::Upload(_) => TransferDirection::Upload,
            Self::Download(_) => TransferDirection::Download,
            Self::Relay(_) => TransferDirection::Relay,
        }
    }
    /// The remote filesystem the request is anchored to; the destination for a relay.
    pub fn fs(&self) -> &Arc<dyn RemoteFs> {
        match self {
            Self::Upload(value) => &value.fs,
            Self::Download(value) => &value.fs,
            Self::Relay(value) => &value.fs,
        }
    }
    /// The remote filesystem that is read: downloads and relays only.
    pub fn source_fs(&self) -> Option<&Arc<dyn RemoteFs>> {
        match self {
            Self::Upload(_) => None,
            Self::Download(value) => Some(&value.fs),
            Self::Relay(value) => Some(&value.source_fs),
        }
    }
    /// Replaces the anchored filesystem (the destination for a relay).
    pub fn with_fs(mut self, fs: Arc<dyn RemoteFs>) -> Self {
        match &mut self {
            Self::Upload(value) => value.fs = fs,
            Self::Download(value) => value.fs = fs,
            Self::Relay(value) => value.fs = fs,
        };
        self
    }
    pub fn collisions(&self) -> CollisionPolicy {
        match self {
            Self::Upload(value) => value.collisions,
            Self::Download(value) => value.collisions,
            Self::Relay(value) => value.collisions,
        }
    }
    pub fn validate(&self, limits: DiscoveryLimits) -> Result<(), TransferError> {
        let hosts = [Some(self.host()), self.source_host()];
        if hosts.into_iter().flatten().any(|host| {
            host.target
                .host
                .len()
                .saturating_add(host.target.user.len())
                > limits.source_path_bytes()
        }) {
            return Err(TransferError::RequestTooLarge);
        }
        match self {
            Self::Upload(value) => {
                limits.validate_sources(
                    value
                        .sources
                        .iter()
                        .map(|path| path.as_os_str().as_encoded_bytes().len()),
                )?;
                if value.destination.len() > limits.source_path_bytes() {
                    return Err(TransferError::RequestTooLarge);
                }
                if !valid_remote_directory(&value.destination)
                    || !value.sources.iter().all(|path| absolute_local(path))
                {
                    return Err(TransferError::InvalidDestination);
                }
            }
            Self::Download(value) => {
                limits.validate_sources(value.sources.iter().map(String::len))?;
                if value.local_destination.as_os_str().as_encoded_bytes().len()
                    > limits.source_path_bytes()
                {
                    return Err(TransferError::RequestTooLarge);
                }
                if !absolute_local(&value.local_destination)
                    || value.sources.iter().any(|path| !valid_remote_source(path))
                {
                    return Err(TransferError::InvalidDestination);
                }
            }
            Self::Relay(value) => {
                limits.validate_sources(value.sources.iter().map(String::len))?;
                if value.destination.len() > limits.source_path_bytes() {
                    return Err(TransferError::RequestTooLarge);
                }
                if !valid_remote_directory(&value.destination)
                    || value.sources.iter().any(|path| !valid_relay_source(path))
                {
                    return Err(TransferError::InvalidDestination);
                }
            }
        }
        Ok(())
    }
    fn source_host(&self) -> Option<&HostAddress> {
        match self {
            Self::Relay(value) => Some(&value.source_host),
            _ => None,
        }
    }
}
/// An absolute native path without `..` components; `~` and relative paths are not expanded.
fn absolute_local(path: &std::path::Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}
/// A `/`-rooted path without NUL, empty or `.`/`..` components whose last component
/// can be published by name at the destination.
pub fn valid_relay_source(source: &str) -> bool {
    valid_remote_directory(source) && valid_publication_name(path::file_name(source))
}
/// A `/`-rooted path without NUL, `.`/`..` or empty components. One trailing `/` is
/// allowed, so `//` cannot spell a path that differs from its plain form only in text.
fn valid_remote_directory(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    !path.contains('\0')
        && (rest.is_empty() || !rest.split('/').any(|part| matches!(part, "" | "." | "..")))
}
