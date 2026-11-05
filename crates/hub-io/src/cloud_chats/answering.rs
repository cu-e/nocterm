//! Keeps the bound Cloud chats of each hub read while the account is unlocked (ADR 0147).
//!
//! A chat whose runner lists bound servers in its summary is opened and subscribed to; its
//! reading task then answers each certificate request the runner publishes, with the device
//! core certifying only inside the binding the person signed. A chat that is no longer bound
//! (unbound, revoked or expired) is let go; a reading that ended with its account is opened
//! again at the next sweep, once the account is unlocked again.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use futures::StreamExt as _;
use nocterm_cloud_agents_model::{ChatHistoryEntry, CloudChatError};
use nocterm_hubs_model::HubId;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

/// One chat read only so its requests are answered; dropping it stops the reading.
struct Kept(JoinHandle<()>);
impl Drop for Kept {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The bound chats read per hub, by chat key.
#[derive(Default)]
pub(super) struct Answering {
    hubs: Mutex<HashMap<HubId, HashMap<String, Kept>>>,
}
impl Answering {
    /// Reads the bound chats of `entries` and lets the others go; how many are read now.
    pub(super) async fn sweep(
        &self,
        hub: &HubId,
        entries: Vec<Arc<dyn ChatHistoryEntry>>,
        handle: &Handle,
    ) -> Result<usize, CloudChatError> {
        let bound: Vec<_> = entries
            .into_iter()
            .filter(|entry| !entry.summary().servers.is_empty())
            .collect();
        let missing: Vec<_> = {
            let mut hubs = self.hubs.lock().unwrap_or_else(PoisonError::into_inner);
            let kept = hubs.entry(hub.clone()).or_default();
            kept.retain(|key, reading| {
                !reading.0.is_finished() && bound.iter().any(|entry| &entry.summary().key == key)
            });
            bound
                .into_iter()
                .filter(|entry| !kept.contains_key(&entry.summary().key))
                .collect()
        };
        for entry in missing {
            let key = entry.summary().key.clone();
            let opened = match entry.open().await {
                Ok(opened) => opened,
                Err(error) => {
                    tracing::debug!(%error, "a bound Cloud chat could not be read");
                    continue;
                }
            };
            let mut updates = opened.conversation.subscribe()?;
            let reading = handle.spawn(async move { while updates.next().await.is_some() {} });
            self.hubs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(hub.clone())
                .or_default()
                .insert(key, Kept(reading));
        }
        Ok(self
            .hubs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(hub)
            .map_or(0, HashMap::len))
    }
    /// Stops reading the chats of `hub`.
    pub(super) fn forget(&self, hub: &HubId) {
        self.hubs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(hub);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_agents_model::{ChatFuture, ChatSummary, OpenedChat};
    use nocterm_cloud_conversation_model::testing::FakeConversation;

    struct Listed {
        summary: ChatSummary,
        conversation: FakeConversation,
    }
    impl ChatHistoryEntry for Listed {
        fn summary(&self) -> &ChatSummary {
            &self.summary
        }
        fn open(&self) -> ChatFuture<OpenedChat> {
            let opened = OpenedChat {
                conversation: Arc::new(self.conversation.clone()),
                title: self.summary.title.clone(),
                key: self.summary.key.clone(),
            };
            Box::pin(async move { Ok(opened) })
        }
    }
    fn listed(
        key: &str,
        servers: &[&str],
        conversation: &FakeConversation,
    ) -> Arc<dyn ChatHistoryEntry> {
        let mut summary = ChatSummary::plain(key.into(), key.into(), "Scribe".into(), None);
        summary.servers = servers.iter().map(|server| (*server).to_owned()).collect();
        Arc::new(Listed {
            summary,
            conversation: conversation.clone(),
        })
    }
    fn hub() -> HubId {
        crate::account::testing::hub()
    }

    #[tokio::test]
    async fn only_bound_chats_are_read_and_a_chat_no_longer_bound_is_let_go() {
        let answering = Answering::default();
        let (bound, plain) = (FakeConversation::new("a"), FakeConversation::new("b"));
        let handle = Handle::current();
        let entries = vec![
            listed("a", &["deploy@db"], &bound),
            listed("b", &[], &plain),
        ];
        assert_eq!(answering.sweep(&hub(), entries, &handle).await.unwrap(), 1);
        assert_eq!((bound.subscribers(), plain.subscribers()), (1, 0));
        // Sweeping again keeps the one reading.
        let again = vec![listed("a", &["deploy@db"], &bound)];
        assert_eq!(answering.sweep(&hub(), again, &handle).await.unwrap(), 1);
        assert_eq!(bound.subscribers(), 1);

        let unbound = vec![listed("a", &[], &bound)];
        assert_eq!(answering.sweep(&hub(), unbound, &handle).await.unwrap(), 0);
        tokio::task::yield_now().await;
        assert_eq!(bound.subscribers(), 0);
    }

    #[tokio::test]
    async fn a_forgotten_hub_reads_nothing_and_an_ended_reading_is_opened_again() {
        let answering = Answering::default();
        let bound = FakeConversation::new("a");
        let handle = Handle::current();
        answering
            .sweep(&hub(), vec![listed("a", &["deploy@db"], &bound)], &handle)
            .await
            .unwrap();
        answering.forget(&hub());
        tokio::task::yield_now().await;
        assert_eq!(bound.subscribers(), 0);

        let again = FakeConversation::new("a");
        answering
            .sweep(&hub(), vec![listed("a", &["deploy@db"], &again)], &handle)
            .await
            .unwrap();
        assert_eq!(again.subscribers(), 1);
    }
}
