//! Picker delivery is bound to the original source and exact composer generation.
use super::composer_view::InputTicket;
use gpui_kit::EntityId;
pub(super) struct ImagePickerOperation {
    chat: Option<EntityId>,
    revision: u64,
    input: InputTicket,
}
impl ImagePickerOperation {
    pub(super) fn new(chat: Option<EntityId>, revision: u64, input: InputTicket) -> Self {
        Self {
            chat,
            revision,
            input,
        }
    }
    pub(super) fn current(
        &self,
        chat: Option<EntityId>,
        revision: u64,
        input: &InputTicket,
    ) -> bool {
        self.chat == chat && self.revision == revision && &self.input == input
    }
}
