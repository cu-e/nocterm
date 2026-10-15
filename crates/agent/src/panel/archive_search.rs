//! The coordinator supplies original catalog identities and its existing reader capability.
use super::{AgentPanel, history_column::SearchTicket};
use crate::runtime::Runtime;
use gpui_kit::Context;
use std::collections::HashSet;
impl AgentPanel {
    pub(super) fn search_archives(&mut self, cx: &mut Context<Self>) {
        self.on_history_catalog_changed(cx);
    }
    pub(super) fn on_history_query_changed(&mut self, cx: &mut Context<Self>) {
        self.start_history_search(cx);
    }
    pub(super) fn on_history_catalog_changed(&mut self, cx: &mut Context<Self>) {
        self.start_history_search(cx);
    }
    fn start_history_search(&mut self, cx: &mut Context<Self>) {
        if !self.history.search_requested(cx) {
            return;
        }
        let ids = self
            .threads
            .iter()
            .filter(|thread| thread.read(cx).is_archived())
            .map(|thread| thread.read(cx).document().chat_id().to_owned())
            .collect();
        let reader = Runtime::global(cx).read(cx).history_reader();
        let catalog = self
            .threads
            .iter()
            .map(|thread| (thread.entity_id(), thread.read(cx).is_archived()))
            .collect();
        self.history.start_search(ids, reader, catalog, cx);
        cx.notify();
    }
    pub(super) fn on_archive_search_completed(
        &mut self,
        ticket: SearchTicket,
        matches: HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        if self.history.accept_matches(&ticket, matches) {
            cx.notify();
        }
    }
}
