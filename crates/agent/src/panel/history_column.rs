//! Search, rename and display-clock lifetimes of the history column.
use super::AgentPanel;
use crate::runtime::HistoryReader;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, Focusable as _, Subscription, Task, Window,
    component::{
        Sizable as _,
        input::{Input, InputEvent, InputState},
    },
};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub(super) struct SearchTicket {
    query: String,
    query_identity: Arc<()>,
    catalog_identity: Arc<Vec<(EntityId, bool)>>,
}
struct SearchState {
    query: String,
    query_identity: Arc<()>,
    catalog_identity: Arc<Vec<(EntityId, bool)>>,
    task: Option<Task<()>>,
    matches: HashSet<String>,
}
impl Default for SearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            query_identity: Arc::new(()),
            catalog_identity: Arc::new(Vec::new()),
            task: None,
            matches: HashSet::new(),
        }
    }
}
impl SearchState {
    fn begin(&mut self, query: String, catalog: Vec<(EntityId, bool)>) -> Option<SearchTicket> {
        let catalog_changed = *self.catalog_identity != catalog;
        if self.query == query && !catalog_changed {
            return None;
        }
        self.task = None;
        self.matches.clear();
        if self.query != query {
            self.query = query;
            self.query_identity = Arc::new(());
        }
        if catalog_changed {
            self.catalog_identity = Arc::new(catalog);
        }
        (!self.query.is_empty()).then(|| SearchTicket {
            query: self.query.clone(),
            query_identity: self.query_identity.clone(),
            catalog_identity: self.catalog_identity.clone(),
        })
    }
    fn contains(&self, chat: &str) -> bool {
        self.matches.contains(chat)
    }
    fn accept(&mut self, ticket: &SearchTicket, matches: HashSet<String>) -> bool {
        if self.query != ticket.query
            || !Arc::ptr_eq(&self.query_identity, &ticket.query_identity)
            || !Arc::ptr_eq(&self.catalog_identity, &ticket.catalog_identity)
        {
            return false;
        }
        self.task = None;
        self.matches = matches;
        true
    }
}
struct SearchCancellation(Arc<AtomicBool>);
impl Drop for SearchCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RenameBinding {
    chat: EntityId,
    input: EntityId,
}
struct RenameSession {
    chat: EntityId,
    input: Entity<InputState>,
    _bindings: [Subscription; 2],
}
pub(super) struct RenameRequest {
    chat: EntityId,
    title: String,
}
impl RenameRequest {
    pub(super) fn into_parts(self) -> (EntityId, String) {
        (self.chat, self.title)
    }
}
#[derive(Clone)]
pub(super) struct HistoryClockTicket(Arc<()>);
struct HistoryClock {
    ticket: HistoryClockTicket,
    _task: Task<()>,
}
pub(super) struct HistoryColumn {
    input: Entity<InputState>,
    search: SearchState,
    rename: Option<RenameSession>,
    tick: Option<HistoryClock>,
    _bindings: Vec<Subscription>,
}
impl HistoryColumn {
    pub(super) fn new(window: &mut Window, cx: &mut Context<AgentPanel>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats…"));
        let subscription = cx.subscribe(&input, |panel, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                panel.on_history_query_changed(cx);
            }
        });
        Self {
            input,
            search: SearchState::default(),
            rename: None,
            tick: None,
            _bindings: vec![subscription],
        }
    }
    pub(super) fn search_requested(&self, cx: &App) -> bool {
        !self.query(cx).is_empty() || !self.search.query.is_empty()
    }
    pub(super) fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_lowercase()
    }
    pub(super) fn query_input(&self) -> Input {
        Input::new(&self.input).id("agent-history-search").small()
    }
    pub(super) fn matches(&self, chat: &str) -> bool {
        self.search.contains(chat)
    }
    pub(super) fn record_loaded_match(&mut self, chat: String) {
        self.search.matches.insert(chat);
    }
    pub(super) fn start_search(
        &mut self,
        jobs: Vec<crate::thread::document::SearchJob>,
        reader: HistoryReader,
        catalog: Vec<(EntityId, bool)>,
        cx: &mut Context<AgentPanel>,
    ) {
        let Some(ticket) = self.search.begin(self.query(cx), catalog) else {
            return;
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        let flag = cancellation.clone();
        let query = ticket.query.clone();
        let work = cx.background_executor().spawn(async move {
            let mut matches = HashSet::new();
            for (id, document, source) in jobs {
                if flag.load(Ordering::Acquire) {
                    break;
                }
                let Ok(load) = reader.load_managed(document, source) else {
                    continue;
                };
                let Ok((mut delivery, permit)) = load.await else {
                    continue;
                };
                let found = !flag.load(Ordering::Acquire)
                    && delivery.error().is_none()
                    && delivery.mentions(&query).unwrap_or(false);
                // Release original physical delivery/source before opening the shared gate.
                drop(delivery);
                drop(permit);
                if found {
                    matches.insert(id);
                }
            }
            matches
        });
        let cancellation = SearchCancellation(cancellation);
        self.search.task = Some(cx.spawn(async move |panel, cx| {
            let _cancel = cancellation;
            let matches = work.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_archive_search_completed(ticket, matches, cx)
            });
        }));
    }
    pub(super) fn accept_matches(
        &mut self,
        ticket: &SearchTicket,
        matches: HashSet<String>,
    ) -> bool {
        self.search.accept(ticket, matches)
    }
    pub(super) fn begin_rename(
        &mut self,
        chat: EntityId,
        title: String,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        let binding = RenameBinding {
            chat,
            input: input.entity_id(),
        };
        let enter = cx.subscribe_in(&input, window, move |panel, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                panel.commit_history_rename(binding, window, cx);
            }
        });
        let focus = input.read(cx).focus_handle(cx);
        let blur = cx.on_blur(&focus, window, move |panel, window, cx| {
            panel.commit_history_rename(binding, window, cx)
        });
        self.rename = Some(RenameSession {
            chat,
            input,
            _bindings: [enter, blur],
        });
        window.focus(&focus, cx);
    }
    pub(super) fn rename_submission(
        &mut self,
        binding: RenameBinding,
        cx: &App,
    ) -> Option<RenameRequest> {
        let rename = self.rename.as_ref()?;
        if rename.chat != binding.chat || rename.input.entity_id() != binding.input {
            return None;
        }
        let rename = self.rename.take()?;
        Some(RenameRequest {
            chat: rename.chat,
            title: rename.input.read(cx).value().to_string(),
        })
    }
    pub(super) fn rename_binding(&self) -> Option<RenameBinding> {
        self.rename.as_ref().map(|rename| RenameBinding {
            chat: rename.chat,
            input: rename.input.entity_id(),
        })
    }
    pub(super) fn cancel_rename(&mut self) {
        self.rename = None;
    }
    pub(super) fn renaming(&self) -> bool {
        self.rename.is_some()
    }
    pub(super) fn rename_input(&self, chat: EntityId) -> Option<Input> {
        self.rename
            .as_ref()
            .filter(|rename| rename.chat == chat)
            .map(|rename| {
                Input::new(&rename.input)
                    .id(("history-rename", chat.as_u64()))
                    .xsmall()
            })
    }
    pub(super) fn clock_current(&self, ticket: &HistoryClockTicket) -> bool {
        self.tick
            .as_ref()
            .is_some_and(|clock| Arc::ptr_eq(&clock.ticket.0, &ticket.0))
    }
    pub(super) fn stop_clock(&mut self, ticket: &HistoryClockTicket) {
        if self.clock_current(ticket) {
            self.tick = None;
        }
    }
    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<AgentPanel>) {
        if !visible {
            self.tick = None;
            return;
        }
        if self.tick.is_some() {
            return;
        }
        let ticket = HistoryClockTicket(Arc::new(()));
        let completion = ticket.clone();
        let task = cx.spawn(async move |panel, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                if !panel
                    .update(cx, |panel, cx| panel.on_history_clock_tick(&completion, cx))
                    .unwrap_or(false)
                {
                    break;
                }
            }
        });
        self.tick = Some(HistoryClock {
            ticket,
            _task: task,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returning_to_the_same_query_rejects_an_old_search_incarnation() {
        let mut search = SearchState::default();
        let old = search.begin("a".into(), Vec::new()).unwrap();
        search.begin("b".into(), Vec::new());
        let current = search.begin("a".into(), Vec::new()).unwrap();
        assert!(!search.accept(&old, HashSet::from(["old".into()])));
        assert!(search.accept(&current, HashSet::from(["current".into()])));
        assert!(search.contains("current"));
        assert!(!search.contains("old"));
    }
    #[test]
    fn changing_catalog_rejects_matches_for_an_identical_query() {
        let mut search = SearchState::default();
        let old = search.begin("a".into(), Vec::new()).unwrap();
        let current = search.begin("a".into(), vec![(1.into(), true)]).unwrap();
        assert!(!search.accept(&old, HashSet::from(["removed".into()])));
        assert!(search.accept(&current, HashSet::new()));
        assert!(!search.contains("removed"));
    }
}
