//! Hydration returns its original local generation and delivery permit together.
use super::LoadId;
use nocterm_ai::{
    history::{HistoryPermit, SavedChat},
    session::Ticket,
    storage::AgentStorageError,
};
pub(in crate::thread) struct LoadOperation {
    id: LoadId,
    ticket: Ticket,
    chat: String,
    reader: crate::runtime::HistoryReader,
}
impl LoadOperation {
    pub(in crate::thread) fn new(
        id: LoadId,
        ticket: Ticket,
        chat: String,
        reader: crate::runtime::HistoryReader,
    ) -> Self {
        Self {
            id,
            ticket,
            chat,
            reader,
        }
    }
    pub(in crate::thread) async fn run(self) -> LoadCompletion {
        let (result, permit) = self.reader.load(self.chat).await;
        LoadCompletion {
            id: self.id,
            ticket: self.ticket,
            result,
            permit,
        }
    }
}
pub(in crate::thread) struct LoadCompletion {
    id: LoadId,
    ticket: Ticket,
    result: Result<SavedChat, AgentStorageError>,
    permit: HistoryPermit,
}
impl LoadCompletion {
    pub(in crate::thread) fn into_parts(
        self,
    ) -> (
        LoadId,
        Ticket,
        Result<SavedChat, AgentStorageError>,
        HistoryPermit,
    ) {
        (self.id, self.ticket, self.result, self.permit)
    }
}
