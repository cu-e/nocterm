use futures::future::BoxFuture;
use nocterm_containers::Engine;
use nocterm_core::SharedError;
use nocterm_session::{ExecError, HostExec, local_fs::CancelFlag};
use std::{sync::Arc, time::Duration};

pub type ImageSearchFuture<T> = BoxFuture<'static, Result<T, ImageSearchError>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSearchQuery(String);
impl ImageSearchQuery {
    pub fn parse(value: &str) -> Result<Self, ImageSearchError> {
        if value.chars().any(char::is_control) {
            return Err(ImageSearchError::InvalidQuery);
        }
        let value = value.trim();
        if value.len() > 256
            || value.chars().count() < 2
            || value.starts_with('-')
            || value.chars().any(char::is_control)
        {
            return Err(ImageSearchError::InvalidQuery);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryImage {
    pub reference: String,
    pub description: String,
    pub official: bool,
    pub stars: Option<u64>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ImageSearchError {
    #[error("enter at least two characters to search images")]
    InvalidQuery,
    #[error("the selected container engine is unavailable")]
    Unavailable,
    #[error("image search was canceled")]
    Canceled,
    #[error("image search timed out")]
    TimedOut,
    #[error("the registry returned too much data")]
    OutputLimit,
    #[error("the registry returned invalid image data")]
    InvalidResponse,
    #[error("the registry temporarily limited searches; try again in a minute")]
    RateLimited { cooldown: Duration },
    #[error("image search failed (exit status {status:?})")]
    CommandFailed { status: Option<u32> },
    #[error(transparent)]
    Execution(#[from] ExecError),
    #[error("the registry returned malformed image data")]
    Codec(#[source] SharedError<serde_json::Error>),
}

pub trait ImageSearch: Send + Sync + 'static {
    fn search(
        &self,
        exec: Arc<dyn HostExec>,
        engine: Engine,
        query: ImageSearchQuery,
        cancel: CancelFlag,
    ) -> ImageSearchFuture<Vec<RegistryImage>>;
}
impl From<serde_json::Error> for ImageSearchError {
    fn from(error: serde_json::Error) -> Self {
        Self::Codec(SharedError::from_error(error))
    }
}
