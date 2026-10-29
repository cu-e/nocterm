//! One finite admission follows SDK bodies and actual blocking read completion.
use super::{RunnerHeadError, mapping::HeadReadValue};
use nocterm_cloud_model::{
    identity::MAX_NAMESPACE_BYTES,
    journal::{
        CurrentChatHead, CurrentHeadsPage, JournalError, MAX_CURRENT_HEADS_PAGE,
        PublicationAllocation, PublicationWorkBudget,
    },
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_client::cloud::{
    head_decoded_metadata_bytes, head_query_work_bytes, head_reply_work_bytes,
};
use nocterm_hub_model::cloud::{
    MAX_ID_BYTES,
    head::{
        ChatHead, MAX_HEAD_PROBES_GLOBAL, MAX_HEAD_QUERY_BYTES, MAX_HEAD_REPLY_ENVELOPE_BYTES,
        MAX_HEAD_WITNESS_BYTES,
    },
};
use std::{fmt, sync::Arc};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};

/// Share one instance across the process's responders. This supplies a finite
/// pool and default two actual reads, not a global or SQLite/RSS accounting claim.
pub struct RunnerHeadWork {
    budget: Arc<dyn PublicationWorkBudget>,
    active: Arc<Semaphore>,
    runtime: Handle,
}
impl RunnerHeadWork {
    pub fn new(budget: Arc<dyn PublicationWorkBudget>, runtime: Handle) -> Arc<Self> {
        Arc::new(Self {
            budget,
            active: Arc::new(Semaphore::new(2)),
            runtime,
        })
    }
    pub fn with_max_reads(
        budget: Arc<dyn PublicationWorkBudget>,
        runtime: Handle,
        maximum: usize,
    ) -> Result<Arc<Self>, RunnerHeadError> {
        if maximum == 0 || maximum > MAX_HEAD_PROBES_GLOBAL {
            return Err(RunnerHeadError::Configuration);
        }
        Ok(Arc::new(Self {
            budget,
            active: Arc::new(Semaphore::new(maximum)),
            runtime,
        }))
    }
    /// SDK work plus returned native/mapped pages and actual signing/sealing
    /// generations. Internal SQLite/catalog scratch belongs to JournalLimits;
    /// the separate read semaphore bounds that existing reader's concurrency.
    pub fn operation_bytes() -> usize {
        let sdk = head_query_work_bytes() + head_reply_work_bytes() + head_decoded_metadata_bytes();
        // Each native head owns stream + at most2 run scopes (6 namespace strings).
        let native_head = std::mem::size_of::<CurrentChatHead>() + 6 * MAX_NAMESPACE_BYTES;
        let native_page = 2 * MAX_CURRENT_HEADS_PAGE * native_head
            + std::mem::size_of::<CurrentHeadsPage>()
            + 8 * MAX_NAMESPACE_BYTES;
        let mapped_page = 2 * MAX_CURRENT_HEADS_PAGE * std::mem::size_of::<ChatHead>();
        // Canonical witness and signed framing may coexist. Vec append/AEAD
        // capacity can retain up to twice the bounded logical body; final box is
        // exact-sized. Query/AAD hex are separately counted, not payload guesses.
        let canonical = 2 * MAX_HEAD_WITNESS_BYTES + 2 * MAX_HEAD_REPLY_ENVELOPE_BYTES;
        let ciphertext = 2 * MAX_HEAD_REPLY_ENVELOPE_BYTES + MAX_HEAD_REPLY_ENVELOPE_BYTES;
        let context = 2 * MAX_HEAD_QUERY_BYTES;
        let hex_context = 2 * MAX_HEAD_QUERY_BYTES;
        let aad = 2 * (64 + hex_context);
        sdk + native_page
            + mapped_page
            + canonical
            + ciphertext
            + context
            + hex_context
            + aad
            + 3 * MAX_ID_BYTES
            + std::mem::size_of::<HeadReadValue>()
            + std::mem::size_of::<HeadReservation>()
    }
    pub(super) fn reserve(
        &self,
        turn: OwnedSemaphorePermit,
    ) -> Result<Arc<HeadReservation>, RunnerHeadError> {
        let allocation = self
            .budget
            .try_reserve(PublicationAllocation::new(Self::operation_bytes(), 1)?)?;
        Ok(Arc::new(HeadReservation {
            _turn: turn,
            _allocation: allocation,
        }))
    }
    pub(super) fn runtime(&self) -> &Handle {
        &self.runtime
    }
    pub(super) fn spawn_read<T: Send + 'static>(
        &self,
        reservation: Arc<HeadReservation>,
        operation: impl FnOnce() -> Result<T, RunnerHeadError> + Send + 'static,
    ) -> Result<JoinHandle<ReadOutput<T>>, RunnerHeadError> {
        let slot = self
            .active
            .clone()
            .try_acquire_owned()
            .map_err(|_| JournalError::Busy)?;
        Ok(self.runtime.spawn_blocking(move || {
            let value = operation();
            ReadOutput {
                value,
                _slot: slot,
                _reservation: reservation,
            }
        }))
    }
}
// The actual data drops before the slot and original allocation, including Err
// and unobserved blocking-task outputs after caller cancellation.
pub(super) struct ReadOutput<T> {
    value: Result<T, RunnerHeadError>,
    _slot: OwnedSemaphorePermit,
    _reservation: Arc<HeadReservation>,
}
impl<T> ReadOutput<T> {
    pub(super) fn value(&self) -> Result<&T, RunnerHeadError> {
        self.value.as_ref().map_err(Clone::clone)
    }
}
pub(super) struct HeadReservation {
    _turn: OwnedSemaphorePermit,
    _allocation: AllocationLease,
}
impl fmt::Debug for RunnerHeadWork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerHeadWork").finish_non_exhaustive()
    }
}
