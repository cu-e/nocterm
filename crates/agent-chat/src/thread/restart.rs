//! Replacement processes transfer durable meaning without allocating a new chat identity.
use super::{Composer, Restore};
pub struct RestartData {
    document: super::document::DocumentRestart,
    config: super::config::RestartConfig,
    composer: Composer,
    restore: Option<Restore>,
}
impl RestartData {
    pub(super) fn new(
        document: super::document::DocumentRestart,
        config: super::config::RestartConfig,
        composer: Composer,
        restore: Option<Restore>,
    ) -> Self {
        Self {
            document,
            config,
            composer,
            restore,
        }
    }
    pub(super) fn into_parts(
        self,
    ) -> (
        super::document::DocumentRestart,
        super::config::RestartConfig,
        Composer,
        Option<Restore>,
    ) {
        (self.document, self.config, self.composer, self.restore)
    }

    pub fn agent_id(&self) -> &str {
        self.document.agent_id()
    }
}
