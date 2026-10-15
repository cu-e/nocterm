//! Picker delivery is bound to the original chat and composer revision.
use gpui_kit::EntityId;
pub(super) struct ImagePickerOperation {
    chat: Option<EntityId>,
    revision: u64,
}
impl ImagePickerOperation {
    pub(super) fn new(chat: Option<EntityId>, revision: u64) -> Self {
        Self { chat, revision }
    }
    pub(super) fn current(&self, chat: Option<EntityId>, revision: u64) -> bool {
        self.chat == chat && self.revision == revision
    }
}
