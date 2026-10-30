//! Periodic FIFO delivery of the durable outbox. The coordinator only queues
//! sealed publications; without this owner no event would reach the hub.
use nocterm_cloud_model::{
    coordinator::CoordinatorError,
    journal::{JournalCatalogReader, JournalError},
    tool::ToolTimer,
};
use nocterm_cloud_runner::{DurableRunJournal, coordinator::RunCoordinator};
use std::{sync::Arc, time::Duration};

const INTERVAL: Duration = Duration::from_secs(1);
const PAGE: u16 = 64;

pub(super) struct OutboxPublisher {
    coordinator: RunCoordinator,
    journal: DurableRunJournal,
    timer: Arc<dyn ToolTimer>,
}
impl OutboxPublisher {
    pub(super) fn new(
        coordinator: RunCoordinator,
        journal: DurableRunJournal,
        timer: Arc<dyn ToolTimer>,
    ) -> Self {
        Self {
            coordinator,
            journal,
            timer,
        }
    }
    /// Returns only with the error that ends delivery for this process.
    pub(super) async fn run(self) -> Result<(), CoordinatorError> {
        loop {
            self.timer.delay(INTERVAL).await;
            self.sweep().await?;
        }
    }
    async fn sweep(&self) -> Result<(), CoordinatorError> {
        let mut cursor = None;
        loop {
            let page = match self.journal.catalog(cursor, PAGE).await {
                Ok(page) => page,
                // A catalog that changed mid-scan restarts at the next interval.
                Err(JournalError::Conflict) => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            for chat in page.chats() {
                let stream = page.binding().stream(*chat);
                match self.coordinator.flush_outbox(stream).await {
                    Ok(()) => {}
                    Err(error) if fatal(&error) => return Err(error),
                    // The same durable delivery IDs are reloaded by the next sweep.
                    Err(_) => {}
                }
            }
            cursor = page.next_cursor();
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}
fn fatal(error: &CoordinatorError) -> bool {
    matches!(
        error,
        CoordinatorError::StaleSource
            | CoordinatorError::Stopped
            | CoordinatorError::Journal(JournalError::Corrupt | JournalError::UnsupportedFormat)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_authority_loss_or_corruption_ends_outbox_delivery() {
        assert!(fatal(&CoordinatorError::StaleSource));
        assert!(fatal(&CoordinatorError::Stopped));
        assert!(fatal(&CoordinatorError::Journal(JournalError::Corrupt)));
        assert!(!fatal(&CoordinatorError::Overloaded));
        assert!(!fatal(&CoordinatorError::Journal(JournalError::Fenced)));
    }
}
