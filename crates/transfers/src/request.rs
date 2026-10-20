use crate::{
    CollisionPolicy, DiscoveryLimits, TransferDirection, TransferError, valid_remote_source,
};
use nocterm_session::{HostAddress, RemoteFs};
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
#[derive(Clone)]
pub enum Request {
    Upload(UploadRequest),
    Download(DownloadRequest),
}
impl Request {
    pub fn host(&self) -> &HostAddress {
        match self {
            Self::Upload(value) => &value.host,
            Self::Download(value) => &value.host,
        }
    }
    pub fn destination(&self) -> String {
        match self {
            Self::Upload(value) => value.destination.clone(),
            Self::Download(value) => value.local_destination.display().to_string(),
        }
    }
    pub fn direction(&self) -> TransferDirection {
        match self {
            Self::Upload(_) => TransferDirection::Upload,
            Self::Download(_) => TransferDirection::Download,
        }
    }
    pub fn fs(&self) -> &Arc<dyn RemoteFs> {
        match self {
            Self::Upload(value) => &value.fs,
            Self::Download(value) => &value.fs,
        }
    }
    pub fn with_fs(mut self, fs: Arc<dyn RemoteFs>) -> Self {
        match &mut self {
            Self::Upload(value) => value.fs = fs,
            Self::Download(value) => value.fs = fs,
        };
        self
    }
    pub fn collisions(&self) -> CollisionPolicy {
        match self {
            Self::Upload(value) => value.collisions,
            Self::Download(value) => value.collisions,
        }
    }
    pub fn validate(&self, limits: DiscoveryLimits) -> Result<(), TransferError> {
        if self
            .host()
            .target
            .host
            .len()
            .saturating_add(self.host().target.user.len())
            > limits.source_path_bytes()
        {
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
                if !value.destination.starts_with('/')
                    || value.destination.contains('\0')
                    || value
                        .destination
                        .split('/')
                        .any(|part| matches!(part, "." | ".."))
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
                if !value.local_destination.is_absolute()
                    || value
                        .local_destination
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir))
                    || value.sources.iter().any(|path| !valid_remote_source(path))
                {
                    return Err(TransferError::InvalidDestination);
                }
            }
        }
        Ok(())
    }
}
