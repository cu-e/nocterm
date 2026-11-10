//! One owner's bounded book of transfers over the narrow request port.
use crate::{Progress, Request, TransferError, TransferId, TransferRequests};
use std::{collections::VecDeque, sync::Arc};

const MAX_ACTIVE: usize = 4;
const MAX_RETAINED: usize = 16;

/// How many transfers one owner may keep running and remember.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedLimits {
    active: usize,
    retained: usize,
}
impl Default for OwnedLimits {
    fn default() -> Self {
        Self {
            active: MAX_ACTIVE,
            retained: MAX_RETAINED,
        }
    }
}
impl OwnedLimits {
    /// `active` is 1..=4 and `retained`, counting active entries, is `active`..=16.
    pub fn new(active: usize, retained: usize) -> Result<Self, TransferError> {
        if active == 0 || active > MAX_ACTIVE {
            return Err(TransferError::InvalidLimit("owned active transfers"));
        }
        if retained < active || retained > MAX_RETAINED {
            return Err(TransferError::InvalidLimit("owned retained transfers"));
        }
        Ok(Self { active, retained })
    }
    pub fn active(self) -> usize {
        self.active
    }
    pub fn retained(self) -> usize {
        self.retained
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OwnedTransferError {
    #[error("Too many transfers are running ({limit}). Wait for one to finish or cancel it.")]
    TooManyActive { limit: usize },
    #[error("This transfer was not started by this chat.")]
    NotOwned,
    #[error("This transfer is no longer available.")]
    Expired,
    #[error("{0}")]
    Transfer(#[from] TransferError),
}

struct Owned<B> {
    id: TransferId,
    binding: B,
    /// The last snapshot, kept once the transfer finished because the shared
    /// runtime trims its own history.
    finished: Option<Progress>,
    /// The port forgot the transfer before a final snapshot was seen.
    lost: bool,
}
impl<B> Owned<B> {
    fn active(&self) -> bool {
        self.finished.is_none() && !self.lost
    }
}

/// One owner's transfers: submits, follows and cancels only its own ids.
/// `B` is whatever the owner needs to decide later that a transfer is stale.
pub struct OwnedTransfers<B> {
    port: Arc<dyn TransferRequests>,
    entries: VecDeque<Owned<B>>,
    limits: OwnedLimits,
}
impl<B> OwnedTransfers<B> {
    pub fn new(port: Arc<dyn TransferRequests>, limits: OwnedLimits) -> Self {
        Self {
            port,
            entries: VecDeque::new(),
            limits,
        }
    }
    /// Refreshes, refuses an active transfer beyond the limit, then evicts the
    /// oldest finished entries beyond the retained bound.
    pub fn submit(
        &mut self,
        request: Request,
        binding: B,
    ) -> Result<TransferId, OwnedTransferError> {
        self.refresh();
        if self.active_count() >= self.limits.active {
            return Err(OwnedTransferError::TooManyActive {
                limit: self.limits.active,
            });
        }
        let id = self.port.submit(request)?;
        self.entries.push_back(Owned {
            id,
            binding,
            finished: None,
            lost: false,
        });
        self.evict();
        Ok(id)
    }
    pub fn progress(&mut self, id: TransferId) -> Result<Progress, OwnedTransferError> {
        let index = self.position(id)?;
        self.snapshot(index)
    }
    /// Requests cancellation and returns the snapshot taken right after it;
    /// a transfer that already finished is returned unchanged.
    pub fn cancel(&mut self, id: TransferId) -> Result<Progress, OwnedTransferError> {
        let index = self.position(id)?;
        if self.entries[index].active() {
            match self.port.cancel(id) {
                Ok(()) => {}
                Err(TransferError::Missing) => {
                    self.entries[index].lost = true;
                    return Err(OwnedTransferError::Expired);
                }
                Err(error) => return Err(error.into()),
            }
        }
        self.snapshot(index)
    }
    /// Cancels every active transfer whose binding `keep` rejects.
    /// Cancellation is best effort: a refusal leaves the transfer to finish.
    pub fn retain_active(&mut self, mut keep: impl FnMut(&B) -> bool) {
        self.refresh();
        for entry in &self.entries {
            if entry.active() && !keep(&entry.binding) {
                let _ = self.port.cancel(entry.id);
            }
        }
    }
    pub fn cancel_all(&mut self) {
        self.retain_active(|_| false);
    }
    /// Refreshes every active entry, caching final snapshots; true while one is unfinished.
    pub fn has_active(&mut self) -> bool {
        self.refresh();
        self.active_count() > 0
    }
    fn position(&self, id: TransferId) -> Result<usize, OwnedTransferError> {
        self.entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(OwnedTransferError::NotOwned)
    }
    fn snapshot(&mut self, index: usize) -> Result<Progress, OwnedTransferError> {
        let entry = &mut self.entries[index];
        if let Some(progress) = &entry.finished {
            return Ok(progress.clone());
        }
        if entry.lost {
            return Err(OwnedTransferError::Expired);
        }
        match self.port.progress(entry.id) {
            Ok(progress) => {
                if progress.state.finished() {
                    entry.finished = Some(progress.clone());
                }
                Ok(progress)
            }
            Err(TransferError::Missing) => {
                entry.lost = true;
                Err(OwnedTransferError::Expired)
            }
            Err(error) => Err(error.into()),
        }
    }
    fn refresh(&mut self) {
        for index in 0..self.entries.len() {
            if self.entries[index].active() {
                let _ = self.snapshot(index);
            }
        }
    }
    fn active_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.active()).count()
    }
    fn evict(&mut self) {
        while self.entries.len() > self.limits.retained {
            let Some(oldest) = self.entries.iter().position(|entry| !entry.active()) else {
                break;
            };
            self.entries.remove(oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CollisionPolicy, DownloadRequest, TransferEvent, TransferState, testing::FakeTransfers,
    };
    use nocterm_session::{HostAddress, Target, testing::InMemoryFs};
    use std::path::PathBuf;

    fn request() -> Request {
        Request::Download(DownloadRequest {
            sources: vec!["/srv/a".into()],
            host: HostAddress::direct(Target {
                host: "host".into(),
                port: 22,
                user: "user".into(),
            }),
            local_destination: PathBuf::from("/tmp/in"),
            fs: Arc::new(InMemoryFs::default()),
            collisions: CollisionPolicy::Skip,
        })
    }
    fn book(fake: &Arc<FakeTransfers>) -> OwnedTransfers<u8> {
        OwnedTransfers::new(fake.clone(), OwnedLimits::default())
    }
    fn finish(fake: &FakeTransfers, id: TransferId) {
        fake.advance(id, TransferEvent::DiscoveryComplete).unwrap();
        fake.advance(id, TransferEvent::WorkersDrained).unwrap();
    }

    #[test]
    fn a_fifth_active_transfer_is_refused_until_one_finishes() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let ids: Vec<_> = (0..4).map(|_| book.submit(request(), 0).unwrap()).collect();
        assert!(matches!(
            book.submit(request(), 0),
            Err(OwnedTransferError::TooManyActive { limit: 4 })
        ));
        finish(&fake, ids[0]);
        assert!(book.submit(request(), 0).is_ok());
    }

    #[test]
    fn the_oldest_finished_transfer_is_evicted_beyond_sixteen() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let mut ids = Vec::new();
        for _ in 0..17 {
            let id = book.submit(request(), 0).unwrap();
            finish(&fake, id);
            ids.push(id);
        }
        assert!(matches!(
            book.progress(ids[0]),
            Err(OwnedTransferError::NotOwned)
        ));
        assert!(book.progress(ids[1]).is_ok());
        assert!(book.progress(ids[16]).is_ok());
    }

    #[test]
    fn an_id_this_book_did_not_submit_is_not_owned() {
        let fake = Arc::new(FakeTransfers::default());
        let mut mine = book(&fake);
        let mut theirs = book(&fake);
        let foreign = theirs.submit(request(), 0).unwrap();
        assert!(matches!(
            mine.progress(foreign),
            Err(OwnedTransferError::NotOwned)
        ));
        assert!(matches!(
            mine.cancel(foreign),
            Err(OwnedTransferError::NotOwned)
        ));
    }

    #[test]
    fn a_final_snapshot_survives_the_port_forgetting_the_transfer() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let id = book.submit(request(), 0).unwrap();
        finish(&fake, id);
        assert_eq!(book.progress(id).unwrap().state, TransferState::Completed);
        fake.forget(id);
        assert_eq!(book.progress(id).unwrap().state, TransferState::Completed);
    }

    #[test]
    fn an_unfinished_transfer_the_port_forgot_is_expired() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let id = book.submit(request(), 0).unwrap();
        fake.forget(id);
        assert!(matches!(
            book.progress(id),
            Err(OwnedTransferError::Expired)
        ));
        assert!(!book.has_active());
    }

    #[test]
    fn retain_active_cancels_only_rejected_bindings() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let kept = book.submit(request(), 1).unwrap();
        let dropped = book.submit(request(), 2).unwrap();
        book.retain_active(|binding| *binding == 1);
        assert_eq!(book.progress(kept).unwrap().state, TransferState::Queued);
        assert_eq!(
            book.progress(dropped).unwrap().state,
            TransferState::Cancelled
        );
    }

    #[test]
    fn cancel_all_cancels_every_active_transfer_and_clears_activity() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let first = book.submit(request(), 0).unwrap();
        let second = book.submit(request(), 0).unwrap();
        assert!(book.has_active());
        book.cancel_all();
        assert!(!book.has_active());
        for id in [first, second] {
            assert_eq!(book.progress(id).unwrap().state, TransferState::Cancelled);
        }
    }

    #[test]
    fn cancelling_a_finished_transfer_returns_its_snapshot() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let id = book.submit(request(), 0).unwrap();
        finish(&fake, id);
        assert_eq!(book.cancel(id).unwrap().state, TransferState::Completed);
    }

    #[test]
    fn port_errors_keep_their_cause() {
        let fake = Arc::new(FakeTransfers::default());
        let mut book = book(&fake);
        let mut invalid = request();
        if let Request::Download(value) = &mut invalid {
            value.sources.clear();
        }
        let error = book.submit(invalid, 0).unwrap_err();
        assert!(matches!(
            error,
            OwnedTransferError::Transfer(TransferError::Empty)
        ));
        let source = std::error::Error::source(&error).expect("the port error is the source");
        assert!(matches!(
            source.downcast_ref::<TransferError>(),
            Some(TransferError::Empty)
        ));
    }

    #[test]
    fn limits_reject_values_above_the_bounds() {
        assert!(OwnedLimits::new(5, 16).is_err());
        assert!(OwnedLimits::new(0, 16).is_err());
        assert!(OwnedLimits::new(4, 17).is_err());
        assert!(OwnedLimits::new(4, 3).is_err());
        assert_eq!(OwnedLimits::new(2, 8).unwrap().retained(), 8);
    }
}
