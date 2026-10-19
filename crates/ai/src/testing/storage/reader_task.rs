//! The shared repository fake retains its original load barrier by value.
use super::{InMemoryAgentRepository, Release, StorageOperation};
use crate::{
    history::{self, ReaderDocumentId, ReaderMaterializer},
    model::json::deferred::DecodeLedger,
    storage::AgentStorageError,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

enum Readiness {
    Initial,
    Waiting(Release),
    Ready,
}
pub(super) struct ReadTask {
    repository: InMemoryAgentRepository,
    id: ReaderDocumentId,
    readiness: Readiness,
}
impl ReadTask {
    pub(super) fn new(repository: InMemoryAgentRepository, id: ReaderDocumentId) -> Self {
        Self {
            repository,
            id,
            readiness: Readiness::Initial,
        }
    }
}
impl ReaderMaterializer for ReadTask {
    fn poll_ready(
        &mut self,
        cx: &mut Context<'_>,
        _: &DecodeLedger,
    ) -> Poll<Result<(), AgentStorageError>> {
        if matches!(self.readiness, Readiness::Initial) {
            self.readiness = match self
                .repository
                .begin_barrier(StorageOperation::Load(self.id.as_str().to_owned()))
            {
                Ok(Some(receiver)) => Readiness::Waiting(receiver),
                Ok(None) => Readiness::Ready,
                Err(error) => return Poll::Ready(Err(error)),
            };
        }
        if let Readiness::Waiting(receiver) = &mut self.readiness {
            let Poll::Ready(result) = Pin::new(receiver).poll(cx) else {
                return Poll::Pending;
            };
            if let Err(error) = result.unwrap_or(Err(AgentStorageError::Interrupted)) {
                return Poll::Ready(Err(error));
            }
            self.readiness = Readiness::Ready;
        }
        Poll::Ready(Ok(()))
    }
    fn materialize(
        &mut self,
        ledger: &DecodeLedger,
    ) -> Result<history::NativeMaterialized, AgentStorageError> {
        let chat = {
            let state = self.repository.state.lock().unwrap();
            let bytes = state
                .chats
                .get(self.id.as_str())
                .ok_or_else(|| AgentStorageError::NotFound(self.id.as_str().to_owned()))?;
            history::decode(&self.repository.retention, bytes, self.id.as_str())?
        };
        Ok(history::NativeMaterialized::new(
            history::native::NativeChat::from_fixture(chat, ledger)?,
            history::HistoryOrigin::Version2,
            None,
            0,
        ))
    }
}
