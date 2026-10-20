//! The chats one panel holds and their lifecycle, without views.
//!
//! Owns chat membership and the open chat, creates chats (new, restarted,
//! forked, restored and archived), admits a chat's document before removing
//! it, and keeps resident history within budget. Views decide when these
//! transitions run and react to what the roster reports.
pub(crate) mod chat_list;

use crate::{
    chat_host::ChatHost,
    thread::{AgentThread, Attachment, RestartData},
};
use chat_list::{ChatList, ChatSlot};
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, EntityId, Subscription};
use nocterm_agent_runtime::AgentPersistenceError;

/// Resident transcripts above this many bytes are evicted, oldest first.
const RESIDENT_HISTORY_BUDGET: usize = 64 * 1024 * 1024;

/// How a chat the user opens begins.
pub(crate) enum ChatStart {
    /// A new chat with agent `agent`, attaching terminal `terminal`.
    New {
        agent: String,
        terminal: Option<EntityId>,
    },
    /// The same chat in a new agent process.
    Restart(Box<RestartData>),
}

/// Why a chat could not be removed: its document was not retained.
#[derive(Debug, thiserror::Error)]
#[error("Could not retain chat: {0}")]
pub(crate) struct RetainError(#[from] AgentPersistenceError);

/// What [`ChatRoster::retain`] removed.
pub(crate) struct Retained {
    removed: Vec<EntityId>,
    error: Option<RetainError>,
}
impl Retained {
    pub(crate) fn removed(&self) -> &[EntityId] {
        &self.removed
    }
    /// The last chat that stayed because its document could not be retained.
    pub(crate) fn into_error(self) -> Option<RetainError> {
        self.error
    }
}

pub(crate) struct ChatRoster {
    host: ChatHost,
    chats: ChatList,
}
impl ChatRoster {
    pub(crate) fn new(host: ChatHost) -> Self {
        Self {
            host,
            chats: ChatList::default(),
        }
    }
    pub(crate) fn current(&self) -> Option<Entity<AgentThread>> {
        self.chats.current()
    }
    pub(crate) fn find(&self, id: EntityId) -> Option<&Entity<AgentThread>> {
        self.chats.find(id)
    }
    pub(crate) fn contains(&self, id: EntityId) -> bool {
        self.chats.contains(id)
    }
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.chats.iter()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.chats.is_empty()
    }
    /// Opens chat `id`, hiding the chat open before.
    pub(crate) fn select(&mut self, id: EntityId, cx: &mut App) {
        if let Some(previous) = self.current() {
            previous.update(cx, |thread, cx| thread.set_shown(false, cx));
        }
        self.chats.select(id);
        if let Some(current) = self.current() {
            current.update(cx, |thread, cx| thread.set_shown(true, cx));
        }
    }
    /// Registers `thread`'s document and binds its background sessions to
    /// `window`; `observation` lives as long as the chat stays listed.
    pub(crate) fn track(
        &self,
        thread: &Entity<AgentThread>,
        window: AnyWindowHandle,
        observation: Subscription,
        cx: &mut App,
    ) -> ChatSlot {
        self.host
            .runtime()
            .register_document(crate::thread::client(thread), cx);
        thread.update(cx, |thread, _| thread.bind_window(window));
        ChatSlot::new(thread.clone(), observation)
    }
    pub(crate) fn append(&mut self, slot: ChatSlot) {
        self.chats.append(slot);
    }
    pub(crate) fn prepend(&mut self, slots: Vec<ChatSlot>) {
        self.chats.prepend(slots);
    }
    /// Creates the chat `start` describes, or none for an unknown agent.
    pub(crate) fn start(&self, start: ChatStart, cx: &mut App) -> Option<Entity<AgentThread>> {
        let agent = match &start {
            ChatStart::New { agent, .. } => agent.as_str(),
            ChatStart::Restart(data) => data.agent_id(),
        };
        self.host.agents().get(agent, cx)?;
        self.host.runtime().set_last_agent(agent, cx);
        let host = self.host.clone();
        Some(match start {
            ChatStart::New { agent, terminal } => {
                let thread = cx.new(|cx| {
                    let mut thread = AgentThread::new(agent, host, cx);
                    thread.preview_config(cx);
                    thread
                });
                if let Some(id) = terminal {
                    thread.update(cx, |thread, cx| {
                        thread.add_composer_attachment(Attachment::Terminal(id), cx)
                    });
                }
                thread
            }
            ChatStart::Restart(data) => cx.new(|cx| AgentThread::restarted(*data, host, cx)),
        })
    }
    /// Copies loaded chat `source` up to and including entry `last` (all of
    /// it, with `None`) into a new, saved chat.
    pub(crate) fn fork(
        &self,
        source: &Entity<AgentThread>,
        last: Option<usize>,
        cx: &mut App,
    ) -> Entity<AgentThread> {
        let fork = match last {
            Some(last) => source.read(cx).fork_at(last + 1),
            None => source.read(cx).fork(),
        };
        let host = self.host.clone();
        let thread = cx.new(|cx| AgentThread::forked(fork, host, cx));
        thread.update(cx, |thread, cx| thread.persist(cx));
        thread
    }
    /// The chats saved in earlier runs and not listed yet, newest first:
    /// loaded chats, then archived summaries.
    pub(crate) fn saved(&self, cx: &mut App) -> Vec<Entity<AgentThread>> {
        let (chats, catalog) = self.host.runtime().take_history(cx);
        let mut restored = Vec::new();
        for chat in chats.into_iter().flatten().rev() {
            let host = self.host.clone();
            restored.push(cx.new(|cx| AgentThread::restored(chat, host, cx)));
        }
        for summary in catalog.into_iter().flatten().rev() {
            if self
                .chats
                .iter()
                .chain(&restored)
                .any(|thread| thread.read(cx).chat_id() == summary.id)
            {
                continue;
            }
            let host = self.host.clone();
            restored.push(cx.new(|cx| AgentThread::archived(summary, host, cx)));
        }
        restored
    }
    /// Captures chat `id`'s document so it survives removal.
    fn admit_before_removal(&self, id: EntityId, cx: &mut App) -> Result<(), RetainError> {
        if self.host.runtime().capture_and_detach_document(id, cx)?
            && let Some(thread) = self.chats.find(id)
        {
            thread.update(cx, |thread, cx| {
                thread.document_admitted(thread.document().desired_revision(), cx)
            });
        }
        Ok(())
    }
    /// Removes the chats `keep` rejects whose documents are retained.
    pub(crate) fn retain(
        &mut self,
        keep: impl Fn(&Entity<AgentThread>, &App) -> bool,
        cx: &mut App,
    ) -> Retained {
        let proposed: Vec<_> = self
            .chats
            .iter()
            .filter(|thread| !keep(thread, cx))
            .map(|thread| thread.entity_id())
            .collect();
        let mut error = None;
        let removed = proposed
            .into_iter()
            .filter(|id| match self.admit_before_removal(*id, cx) {
                Ok(()) => true,
                Err(failure) => {
                    error = Some(failure);
                    false
                }
            })
            .collect::<Vec<_>>();
        self.chats.remove_admitted(&removed);
        Retained { removed, error }
    }
    /// Releases chat `id`'s agent and deletes its saved copy; the caller
    /// removes it from the roster.
    pub(crate) fn delete(&self, id: EntityId, cx: &mut App) -> bool {
        let Some(thread) = self.chats.find(id).cloned() else {
            return false;
        };
        let chat = thread.read(cx).document().chat_id().to_owned();
        thread.update(cx, |thread, cx| thread.release_resources(cx));
        self.host.runtime().delete_chat(chat, cx);
        true
    }
    /// Evicts the oldest resident transcripts other than the open chat's
    /// until history fits the budget. Returns the evicted chats whose
    /// transcripts mention `query`.
    pub(crate) fn trim_resident_history(&self, query: &str, cx: &mut App) -> Vec<String> {
        let active = self.current().map(|thread| thread.entity_id());
        let mut resident: usize = self
            .chats
            .iter()
            .map(|thread| thread.read(cx).resident_history_bytes())
            .sum();
        let mut candidates = self.chats.iter().cloned().collect::<Vec<_>>();
        candidates.sort_by_key(|thread| thread.read(cx).document().updated());
        let mut matched = Vec::new();
        for thread in candidates {
            if resident <= RESIDENT_HISTORY_BUDGET {
                break;
            }
            if Some(thread.entity_id()) == active {
                continue;
            }
            let bytes = thread.read(cx).resident_history_bytes();
            let mentions = !query.is_empty() && thread.read(cx).transcript().mentions(query);
            let chat_id = thread.read(cx).document().chat_id().to_owned();
            if thread.update(cx, |thread, cx| thread.evict_history(cx)) {
                resident = resident.saturating_sub(bytes);
                if mentions {
                    matched.push(chat_id);
                }
            }
        }
        matched
    }
}
