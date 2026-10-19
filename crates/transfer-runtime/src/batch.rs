use crate::reservations::ReservationBook;
use nocterm_session::local_fs::CancelFlag;
use nocterm_transfers::{
    Progress, ProgressTracker, Request, StreamLimits, TransferEvent, TransferId,
};
use parking_lot::Mutex;
use std::sync::Arc;

pub(crate) struct Batch {
    sequence: u64,
    request: Request,
    tracker: Mutex<ProgressTracker>,
    cancellation: CancelFlag,
    reservations: Arc<ReservationBook>,
}
impl Batch {
    pub(crate) fn new(
        sequence: u64,
        id: TransferId,
        request: Request,
        limits: StreamLimits,
    ) -> Self {
        Self {
            sequence,
            tracker: Mutex::new(ProgressTracker::new(id, &request, limits)),
            request,
            cancellation: CancelFlag::default(),
            reservations: Arc::new(ReservationBook::default()),
        }
    }
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }
    pub(crate) fn request(&self) -> &Request {
        &self.request
    }
    pub(crate) fn snapshot(&self) -> Progress {
        self.tracker.lock().snapshot()
    }
    pub(crate) fn event(&self, event: TransferEvent) {
        self.tracker.lock().apply(event);
    }
    pub(crate) fn cancellation(&self) -> CancelFlag {
        self.cancellation.clone()
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
        self.event(TransferEvent::CancelRequested);
    }
    pub(crate) fn reservations(&self) -> &Arc<ReservationBook> {
        &self.reservations
    }
    pub(crate) fn finished(&self) -> bool {
        self.snapshot().state.finished()
    }
}
