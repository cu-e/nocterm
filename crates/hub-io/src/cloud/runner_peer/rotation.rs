//! Dropped/failed refresh has an unknown remote result; actual state owns that fact.
use super::RunnerHubConnection;
use std::sync::Arc;
pub(super) struct RefreshGuard {
    owner: Arc<RunnerHubConnection>,
    token: Arc<()>,
}
impl RefreshGuard {
    pub(super) fn new(owner: Arc<RunnerHubConnection>, token: Arc<()>) -> Self {
        Self { owner, token }
    }
    pub(super) fn token(&self) -> &Arc<()> {
        &self.token
    }
}
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        let changed = self
            .owner
            .session
            .lock()
            .map(|mut state| state.abandon(&self.token))
            .unwrap_or(true);
        if changed {
            self.owner.changed();
        }
    }
}
