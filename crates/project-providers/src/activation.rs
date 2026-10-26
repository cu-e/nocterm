//! Workspace-only session activation, composed independently of container routing.
use crate::LocalTransportFactory;
use nocterm_session::Transport;
use std::sync::Arc;

/// Adds the user's managed workspace tools without changing ordinary connections.
pub trait WorkspaceSessionFactory: Send + Sync + 'static {
    fn wrap(&self, base: Arc<dyn Transport>) -> Arc<dyn Transport>;
    fn wrap_local(&self, factory: LocalTransportFactory) -> Arc<dyn Transport>;
}

/// Fixed workspace tool activation applied before resource-specific request wrapping.
pub trait WorkspaceCommandActivation: Send + Sync + 'static {
    fn activate_request(
        &self,
        request: nocterm_session::ExecRequest,
    ) -> nocterm_session::ExecRequest;
}
