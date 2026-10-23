//! Workspace-only session activation, composed independently of container routing.
use crate::LocalTransportFactory;
use nocterm_session::Transport;
use std::sync::Arc;

/// Adds the user's managed workspace tools without changing ordinary connections.
pub trait WorkspaceSessionFactory: Send + Sync + 'static {
    fn wrap(&self, base: Arc<dyn Transport>) -> Arc<dyn Transport>;
    fn wrap_local(&self, factory: LocalTransportFactory) -> Arc<dyn Transport>;
}
