//! Consumer UI ports for ordinary terminals inside captured folder surfaces.
use crate::ProjectTerminalTarget;
use gpui_kit::{App, Window};
use nocterm_project_model::RootPath;
use std::rc::Rc;

/// Open an explicitly selected saved server in the chat's original surface.
/// Unlike UI selection, a live source remains valid while its workspace is hidden.
pub trait ChatConnectionOpen: 'static {
    fn open(
        &self,
        profile: &str,
        source: crate::ProjectSurfaceReceipt,
        workspace: &gpui_kit::WeakEntity<crate::Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<gpui_kit::EntityId, ChatConnectionOpenError>;
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ChatConnectionOpenError {
    #[error("The saved server profiles are unavailable")]
    Unavailable,
    #[error("The selected saved server no longer exists")]
    MissingProfile,
    #[error("The selected server's jump route cannot be resolved")]
    InvalidRoute(#[source] nocterm_core::SharedCause),
    #[error(transparent)]
    Destination(#[from] ContextSelectionError),
}

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

/// Installed independently of SessionFactory. Async completions register through
/// the original ProjectSurface with its captured UI receipt.
pub trait ProjectTerminalOpen: 'static {
    fn capture(
        &self,
        parent: Rc<dyn crate::ItemHandle>,
        expected: crate::ExpectedSession,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ContextSelectionError>;
}

/// A captured ordinary connection launch configuration; independent of parent tab lifetime.
pub trait ProjectTerminalSession: 'static {
    fn open(
        &self,
        target: ProjectTerminalTarget,
        root: &RootPath,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ContextSelectionError>;
}
