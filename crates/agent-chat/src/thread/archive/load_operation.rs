//! Original archive intent observes the shared admitted native-load service.
use super::LoadId;
use crate::thread::AgentThread;
use gpui::Context;
use nocterm_agent_runtime::{LoadedHistory, history_admission::HistoryPayloadPermit};
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
    pub(in crate::thread) fn start(
        self,
        admission: nocterm_agent_runtime::history_admission::HistoryAdmissionHandle,
        cx: &mut Context<AgentThread>,
    ) {
        let loading = self.reader.load_admitted(self.chat, admission, cx);
        cx.spawn(async move |thread, cx| {
            let loaded = loading.await;
            let completion = LoadCompletion {
                id: self.id,
                ticket: self.ticket,
                loaded,
            };
            let _ = thread.update(cx, |thread, cx| thread.on_archive_loaded(completion, cx));
        })
        .detach();
    }
}
pub(in crate::thread) struct LoadCompletion {
    id: LoadId,
    ticket: Ticket,
    loaded: LoadedHistory,
}
impl LoadCompletion {
    pub(in crate::thread) fn publish(
        self,
        publish: impl FnOnce(
            LoadId,
            Ticket,
            Result<SavedChat, AgentStorageError>,
            Option<HistoryPermit>,
            Option<HistoryPayloadPermit>,
        ),
    ) {
        self.loaded.publish(|result, permit, payload| {
            publish(self.id, self.ticket, result, permit, payload)
        });
    }
}
