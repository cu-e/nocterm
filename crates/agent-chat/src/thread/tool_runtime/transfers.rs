//! The chat's file transfers: it reads and cancels only ids it submitted, and a transfer
//! ends when either endpoint terminal detaches or its session is replaced.
use nocterm_ai::ToolFailure;
use nocterm_transfers::{
    OwnedLimits, OwnedTransferError, OwnedTransfers, Progress, Request, TransferId,
    TransferRequests,
};
use nocterm_workspace_ports::ExpectedSession;
use std::sync::Arc;

/// The terminals a transfer was admitted for and the sessions it captured from them.
pub(super) struct TransferBinding {
    destination: (String, ExpectedSession),
    source: (String, ExpectedSession),
}
impl TransferBinding {
    pub(super) fn new(
        destination: (String, ExpectedSession),
        source: (String, ExpectedSession),
    ) -> Self {
        Self {
            destination,
            source,
        }
    }
    fn holds(&self, current: &[(String, Option<ExpectedSession>)]) -> bool {
        [&self.destination, &self.source]
            .into_iter()
            .all(|(id, expected)| {
                current
                    .iter()
                    .any(|(attached, session)| attached == id && *session == Some(*expected))
            })
    }
}

pub(super) struct ChatTransfers {
    book: OwnedTransfers<TransferBinding>,
}
impl ChatTransfers {
    pub(super) fn new(port: Arc<dyn TransferRequests>) -> Self {
        Self {
            book: OwnedTransfers::new(port, OwnedLimits::default()),
        }
    }
    pub(super) fn start(
        &mut self,
        request: Request,
        binding: TransferBinding,
    ) -> Result<TransferId, ToolFailure> {
        self.book
            .submit(request, binding)
            .map_err(|error| error.to_string().into())
    }
    pub(super) fn progress(&mut self, id: &str) -> Result<Progress, ToolFailure> {
        let id = owned_id(id)?;
        self.book
            .progress(id)
            .map_err(|error| error.to_string().into())
    }
    pub(super) fn cancel(&mut self, id: &str) -> Result<Progress, ToolFailure> {
        let id = owned_id(id)?;
        self.book
            .cancel(id)
            .map_err(|error| error.to_string().into())
    }
    /// `current`: every attached terminal id with its present session.
    pub(super) fn revoke_replaced(&mut self, current: &[(String, Option<ExpectedSession>)]) {
        self.book.retain_active(|binding| binding.holds(current));
    }
    pub(super) fn cancel_all(&mut self) {
        self.book.cancel_all();
    }
    pub(super) fn has_active(&mut self) -> bool {
        self.book.has_active()
    }
}
/// A malformed id cannot be one this chat submitted.
fn owned_id(text: &str) -> Result<TransferId, ToolFailure> {
    text.parse()
        .map_err(|_| OwnedTransferError::NotOwned.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_session::{HostAddress, Target, testing::InMemoryFs};
    use nocterm_transfers::{
        CollisionPolicy, TransferEndpoint, TransferEvent, TransferState, testing::FakeTransfers,
    };

    fn host(name: &str) -> HostAddress {
        HostAddress::direct(Target {
            host: name.into(),
            port: 22,
            user: "user".into(),
        })
    }
    fn relay() -> Request {
        let endpoint = |name: &str| TransferEndpoint::Remote {
            host: host(name),
            fs: Arc::new(InMemoryFs::default()),
        };
        Request::between(
            endpoint("source"),
            vec!["/srv/a".into()],
            endpoint("destination"),
            "/srv/in".into(),
            CollisionPolicy::Skip,
        )
        .unwrap()
    }
    fn session(item: u64, epoch: u64) -> ExpectedSession {
        ExpectedSession::new(gpui::EntityId::from(item), epoch)
    }
    fn binding(epoch: u64) -> TransferBinding {
        TransferBinding::new(
            ("destination".into(), session(1, epoch)),
            ("source".into(), session(2, 0)),
        )
    }
    fn attached(destination_epoch: u64) -> Vec<(String, Option<ExpectedSession>)> {
        vec![
            ("destination".into(), Some(session(1, destination_epoch))),
            ("source".into(), Some(session(2, 0))),
        ]
    }

    #[test]
    fn another_chats_or_malformed_ids_are_not_owned() {
        let fake = Arc::new(FakeTransfers::default());
        let mut other = ChatTransfers::new(fake.clone());
        let foreign = other.start(relay(), binding(0)).unwrap();
        let mut mine = ChatTransfers::new(fake);
        let refused = mine.progress(&foreign.to_string()).unwrap_err();
        assert_eq!(
            refused.to_string(),
            OwnedTransferError::NotOwned.to_string()
        );
        assert!(mine.cancel(&foreign.to_string()).is_err());
        assert!(mine.progress("not an id").is_err());
        assert!(other.progress(&foreign.to_string()).is_ok());
    }

    #[test]
    fn a_replaced_or_detached_endpoint_cancels_only_its_transfer() {
        let fake = Arc::new(FakeTransfers::default());
        let mut transfers = ChatTransfers::new(fake.clone());
        let kept = transfers.start(relay(), binding(0)).unwrap();
        let replaced = transfers.start(relay(), binding(1)).unwrap();
        transfers.revoke_replaced(&attached(0));
        let state = |transfers: &mut ChatTransfers, id: TransferId| {
            transfers.progress(&id.to_string()).unwrap().state
        };
        assert_eq!(state(&mut transfers, kept), TransferState::Queued);
        assert_eq!(state(&mut transfers, replaced), TransferState::Cancelled);
        transfers.revoke_replaced(&attached(0)[..1]);
        assert_eq!(state(&mut transfers, kept), TransferState::Cancelled);
        assert!(!transfers.has_active());
    }

    #[test]
    fn finished_transfers_keep_their_final_report_after_the_runtime_forgets_them() {
        let fake = Arc::new(FakeTransfers::default());
        let mut transfers = ChatTransfers::new(fake.clone());
        let id = transfers.start(relay(), binding(0)).unwrap();
        fake.advance(id, TransferEvent::DiscoveryComplete).unwrap();
        fake.advance(id, TransferEvent::WorkersDrained).unwrap();
        assert!(!transfers.has_active());
        fake.forget(id);
        let report = transfers.progress(&id.to_string()).unwrap();
        assert_eq!(report.state, TransferState::Completed);
    }
}
