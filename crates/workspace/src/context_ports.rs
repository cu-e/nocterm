//! Consumer UI ports for ordinary terminals inside captured folder surfaces.
use gpui_kit::{App, Window};
use nocterm_workspace_ports::ContextSelectionError;

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
