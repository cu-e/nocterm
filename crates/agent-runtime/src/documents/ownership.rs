//! The shared keyed worker loans actual writer capabilities for native execution.
use super::ChatIntent;
use nocterm_store::{KeyedCapture, KeyedValueOwnership, StoreError};
pub(super) struct ChatValues;
pub(super) enum ChatOperation {
    Managed(nocterm_ai::history::writing::ManagedWrite),
    Delete,
}
impl KeyedValueOwnership<ChatIntent> for ChatValues {
    type Write = ChatOperation;
    type Guard = ();
    fn accept(
        &self,
        _incoming: &mut ChatIntent,
        _previous: Option<&mut ChatIntent>,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    fn capture(&self, latest: &ChatIntent) -> Result<KeyedCapture<ChatOperation, ()>, StoreError> {
        let operation = match latest {
            ChatIntent::Managed(latest) => {
                ChatOperation::Managed(latest.capture().map_err(StoreError::backend)?)
            }
            ChatIntent::Delete => ChatOperation::Delete,
        };
        Ok(KeyedCapture::new(operation, ()))
    }
}
