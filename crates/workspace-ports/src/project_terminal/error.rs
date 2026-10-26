//! Typed original-source, destination and provider-resolution refusals.
use nocterm_core::SharedCause;
use nocterm_project_model::ProjectValidationError;
use std::error::Error;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ContextSelectionError {
    #[error(transparent)]
    Parent(#[from] crate::ProjectParentError),
    #[error("the originating workspace or window is no longer available")]
    StaleWindow,
    #[error("the originating context surface is no longer available")]
    StaleSurface,
    #[error("the originating UI selection is no longer current")]
    StaleSelection,
    #[error("the original first terminal request has already been issued")]
    InitialTerminalAlreadyRequested,
    #[error("this window cannot retain another context")]
    Capacity,
    #[error("this operation requires the General context")]
    ProtectedContext,
    #[error("the original server terminal is unavailable")]
    ParentUnavailable,
    #[error("no folder terminal provider is installed")]
    Unavailable,
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
