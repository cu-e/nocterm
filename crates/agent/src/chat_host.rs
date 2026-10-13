//! Explicit immutable host collaborators retained by each panel and its chats.
use gpui_kit::WeakEntity;
use nocterm_ai::{history::SavedChat, metadata::ChatMetadataSource};
use nocterm_workspace::Workspace;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ChatHost {
    workspace: WeakEntity<Workspace>,
    metadata: Arc<dyn ChatMetadataSource>,
}
impl ChatHost {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        metadata: Arc<dyn ChatMetadataSource>,
    ) -> Self {
        Self {
            workspace,
            metadata,
        }
    }
    pub(crate) fn workspace(&self) -> &WeakEntity<Workspace> {
        &self.workspace
    }
    pub(crate) fn timestamp(&self) -> u64 {
        self.metadata.timestamp()
    }
    pub(crate) fn new_chat(&self, agent: String) -> SavedChat {
        let identity = self.metadata.new_identity();
        SavedChat::for_identity(agent, identity.id, identity.created)
    }
}
