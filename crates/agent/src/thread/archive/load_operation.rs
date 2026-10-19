//! Hydration receipts surround a prepaid gate/native task without another future Box.
use super::LoadId;
use nocterm_ai::{
    history::{HistoryPermit, PreparedReaderLoad, ReaderDelivery, ReaderSourceError},
    model::json::deferred::NativeAllocationError,
    session::Ticket,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Debug, thiserror::Error)]
pub(in crate::thread) enum ArchiveLoadError {
    #[error(transparent)]
    Document(#[from] super::super::document::DocumentError),
    #[error(transparent)]
    Admission(#[from] NativeAllocationError),
    #[error(transparent)]
    Reader(#[from] ReaderSourceError),
}
pub(in crate::thread) struct LoadOperation {
    work: PreparedReaderLoad,
    id: LoadId,
    ticket: Ticket,
}
impl LoadOperation {
    pub(in crate::thread) fn prepare(
        id: LoadId,
        ticket: Ticket,
        document: &mut super::super::document::ChatDocument,
        reader: crate::runtime::HistoryReader,
    ) -> Result<Self, ArchiveLoadError> {
        let source = document
            .transcript()
            .ok_or(super::super::document::DocumentError::Retired)?
            .reader()
            .clone();
        let chat = document.prepare_reader_id()?;
        let work = reader.load_managed(chat, source)?;
        Ok(Self { work, id, ticket })
    }
}
impl Future for LoadOperation {
    type Output = LoadCompletion;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let Poll::Ready(result) = Pin::new(&mut this.work).poll(cx) else {
            return Poll::Pending;
        };
        let (result, permit) = match result {
            Ok((delivery, permit)) => (Ok(delivery), Some(permit)),
            Err(error) => (Err(error.into()), None),
        };
        Poll::Ready(LoadCompletion {
            id: this.id,
            ticket: this.ticket,
            result,
            permit,
        })
    }
}
pub(in crate::thread) struct LoadCompletion {
    id: LoadId,
    ticket: Ticket,
    result: Result<ReaderDelivery, ArchiveLoadError>,
    permit: Option<HistoryPermit>,
}
impl LoadCompletion {
    pub(in crate::thread) fn refused(id: LoadId, ticket: Ticket, error: ArchiveLoadError) -> Self {
        Self {
            id,
            ticket,
            result: Err(error),
            permit: None,
        }
    }
    pub(in crate::thread) fn into_parts(
        self,
    ) -> (
        LoadId,
        Ticket,
        Result<ReaderDelivery, ArchiveLoadError>,
        Option<HistoryPermit>,
    ) {
        (self.id, self.ticket, self.result, self.permit)
    }
}
