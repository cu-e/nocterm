//! Resolve an ordinary folder terminal configuration without opening another tab.
use crate::ProjectTerminalSession;
use gpui_kit::App;
use nocterm_core::SharedCause;
use nocterm_project_model::{ProjectTarget, ProjectValidationError};
use std::{error::Error, rc::Rc};

pub trait ProjectTerminalResolve: 'static {
    fn resolve(
        &self,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError>;
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectConnectionError {
    #[error("The workspace's saved server profile no longer exists")]
    MissingProfile,
    #[error("The saved server destination or jump route changed; update the workspace first")]
    ChangedTargetOrRoute,
    #[error("The workspace server's jump route cannot be resolved")]
    InvalidRoute(#[source] SharedCause),
    #[error("A saved server profile is required for this operation")]
    UnsupportedHost,
    #[error("The saved server profiles are unavailable")]
    Unavailable,
    #[error("The workspace provider is unavailable or its resource format is unsupported")]
    ProviderUnavailable,
    #[error("A workspace provider is already registered: {0}")]
    DuplicateProvider(nocterm_project_model::ProviderId),
    #[error(transparent)]
    InvalidHost(#[from] ProjectValidationError),
}
impl ProjectConnectionError {
    pub fn invalid_route(source: impl Error + Send + Sync + 'static) -> Self {
        Self::InvalidRoute(SharedCause::new(source))
    }
}
