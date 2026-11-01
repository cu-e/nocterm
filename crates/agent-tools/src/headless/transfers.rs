//! File transfers requested by headless runs, owned per chat and bounded by `OwnedTransfers`.
//!
//! A run's authority names exactly one session. Both endpoints of `transfer_files`
//! are resolved independently against that receipt, so today only terminals of the
//! run's own session are admitted; any other terminal is a stale session.
use super::{HeadlessError, SessionRegistry};
use nocterm_ai::{CancelTransfer, ReadTransfer, TransferFiles, tools::transfer::report};
use nocterm_cloud_model::tool::{ScopedToolRequest, ToolAuthority, ToolExecutionError, ToolTimer};
use nocterm_transfers::{
    CollisionPolicy, OwnedLimits, OwnedTransfers, Request, TransferId, TransferRequests,
};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

const DEFAULT_START_YIELD_MS: u64 = 1000;
const POLL: Duration = Duration::from_millis(25);

/// Opaque, cloneable handle to one chat's transfers. Every run view of a chat
/// shares it, so a later run can read or cancel a transfer an earlier run started.
#[derive(Clone)]
pub struct ChatTransferBook(Arc<Mutex<OwnedTransfers<()>>>);
impl ChatTransferBook {
    pub fn new(port: Arc<dyn TransferRequests>) -> Self {
        Self(Arc::new(Mutex::new(OwnedTransfers::new(
            port,
            OwnedLimits::default(),
        ))))
    }
    /// Cancels every running transfer of this chat; published files stay.
    pub fn cancel_all(&self) {
        self.with(OwnedTransfers::cancel_all);
    }
    fn with<R>(&self, action: impl FnOnce(&mut OwnedTransfers<()>) -> R) -> R {
        action(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The three transfer tool calls of one run view over the chat's book.
pub(super) struct RunTransfers {
    book: ChatTransferBook,
    registry: Arc<Mutex<SessionRegistry>>,
    authority: Arc<dyn ToolAuthority>,
    timer: Arc<dyn ToolTimer>,
}
impl RunTransfers {
    pub(super) fn new(
        book: ChatTransferBook,
        registry: Arc<Mutex<SessionRegistry>>,
        authority: Arc<dyn ToolAuthority>,
        timer: Arc<dyn ToolTimer>,
    ) -> Self {
        Self {
            book,
            registry,
            authority,
            timer,
        }
    }
    pub(super) fn cancel_all(&self) {
        self.book.cancel_all();
    }
    /// Starts one transfer, then waits `yield_ms` for it to finish.
    pub(super) async fn start(
        &self,
        request: &ScopedToolRequest,
        call: &TransferFiles,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        self.authority.check(request)?;
        let (source, destination) = {
            let registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
            (
                registry.endpoint(request, &call.source_terminal_id)?,
                registry.endpoint(request, &call.destination_terminal_id)?,
            )
        };
        let collisions = if call.overwrite {
            CollisionPolicy::Replace
        } else {
            CollisionPolicy::Skip
        };
        let transfer = Request::between(
            source,
            call.sources.clone(),
            destination,
            call.destination.clone(),
            collisions,
        )
        .map_err(HeadlessError::from)?;
        let id = self
            .book
            .with(|book| book.submit(transfer, ()))
            .map_err(HeadlessError::from)?;
        if let Err(error) = self.authority.check(request) {
            let _ = self.book.with(|book| book.cancel(id));
            return Err(error);
        }
        self.wait(request, id, call.yield_ms.unwrap_or(DEFAULT_START_YIELD_MS))
            .await
    }
    /// Reports a transfer of this chat, waiting up to `yield_ms` while it runs.
    pub(super) async fn read(
        &self,
        request: &ScopedToolRequest,
        call: &ReadTransfer,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        let id = parse(&call.transfer_id)?;
        self.wait(request, id, call.yield_ms.unwrap_or(0)).await
    }
    pub(super) fn cancel(
        &self,
        request: &ScopedToolRequest,
        call: &CancelTransfer,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        let id = parse(&call.transfer_id)?;
        self.authority.check(request)?;
        let progress = self
            .book
            .with(|book| book.cancel(id))
            .map_err(HeadlessError::from)?;
        Ok(report(&progress, false))
    }
    async fn wait(
        &self,
        request: &ScopedToolRequest,
        id: TransferId,
        yield_ms: u64,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        let started = self.timer.now();
        let until = started.as_millis().saturating_add(yield_ms);
        loop {
            self.authority.check(request)?;
            if self.timer.now() < started {
                return Err(ToolExecutionError::Revoked);
            }
            let progress = self
                .book
                .with(|book| book.progress(id))
                .map_err(HeadlessError::from)?;
            if progress.state.finished() || self.timer.now().as_millis() >= until {
                return Ok(report(&progress, false));
            }
            self.timer.delay(POLL).await;
        }
    }
}

fn parse(id: &str) -> Result<TransferId, HeadlessError> {
    id.parse::<TransferId>().map_err(HeadlessError::from)
}
