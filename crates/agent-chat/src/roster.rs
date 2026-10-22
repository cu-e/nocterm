//! The chats one panel holds and their lifecycle, without views.
//!
//! Owns chat membership and the open chat, creates chats (new, restarted,
//! forked, restored and archived), admits a chat's document before removing
//! it, and keeps resident history within budget. Views decide when these
//! transitions run and react to what the roster reports.
pub mod chat_list;
mod contexts;

use crate::{
    chat_deps::ChatDeps,
    thread::{AgentThread, Attachment, RestartData},
};
use chat_list::{ChatList, ChatSlot};
use gpui::{AnyWindowHandle, App, Entity, EntityId, Subscription};
use nocterm_agent_runtime::AgentPersistenceError;

/// How a chat the user opens begins.
pub enum ChatStart {
    /// A new chat with agent `agent`, attaching terminal `terminal`.
    New {
        agent: String,
        terminal: Option<EntityId>,
    },
    /// The same chat in a new agent process.
    Restart {
        data: Box<RestartData>,
        expected_owner: EntityId,
    },
}

/// Why a chat could not be removed: its document was not retained.
#[derive(Debug, thiserror::Error)]
#[error("Could not retain chat: {0}")]
pub struct RetainError(#[from] AgentPersistenceError);

/// What [`ChatRoster::admit_removals`] admitted for removal.
pub struct Retained {
    removed: Vec<EntityId>,
    error: Option<RetainError>,
}
impl Retained {
    pub fn removed(&self) -> &[EntityId] {
        &self.removed
    }
    /// Why the last chat that stays could not be retained.
    pub fn into_error(self) -> Option<RetainError> {
        self.error
    }
}

pub struct ChatRoster {
    contexts: contexts::ContextChats,
}
impl ChatRoster {
    pub fn new(host: ChatDeps) -> Self {
        Self {
            contexts: contexts::ContextChats::new(host),
        }
    }
    pub fn host(&self) -> &ChatDeps {
        self.contexts.current().host()
    }
    pub fn contains_any(&self, id: EntityId) -> bool {
        self.contexts.contains_any(id)
    }
    pub fn release_quiet_hidden(&mut self, cx: &App) {
        self.contexts.release_quiet_hidden(cx);
    }
    /// Switches the visible collection; retained threads keep original hosts.
    pub fn switch_context(&mut self, host: ChatDeps, cx: &mut App) -> bool {
        if host.scope() == self.host().scope() {
            return self.contexts.select(host);
        }
        if let Some(previous) = self.current() {
            previous.update(cx, |thread, cx| {
                thread.flush_draft(cx);
                thread.set_shown(false, cx);
            });
        }
        let released: Vec<_> = self
            .iter()
            .filter(|thread| thread.read(cx).can_release_archive())
            .map(|thread| thread.entity_id())
            .collect();
        self.contexts
            .current_mut()
            .chats_mut()
            .remove_admitted(&released);
        self.contexts.select(host);
        if let Some(current) = self.current() {
            current.update(cx, |thread, cx| thread.set_shown(true, cx));
        }
        true
    }
    pub fn current(&self) -> Option<Entity<AgentThread>> {
        self.contexts.current().chats().current()
    }
    pub fn find_any(&self, id: EntityId) -> Option<&Entity<AgentThread>> {
        self.contexts.origin(id)?.chats().find(id)
    }
    pub fn has_retained_chats(&self) -> bool {
        self.contexts.all().any(|scope| !scope.chats().is_empty())
    }
    /// Append to a captured document's original context, without switching views.
    pub fn append_origin(
        &mut self,
        slot: ChatSlot,
        cx: &App,
    ) -> Result<bool, nocterm_agent_runtime::ChatOpenError> {
        let thread = slot.thread();
        let scope = thread.read(cx).document().key().scope();
        let id = thread.entity_id();
        let visible = scope == self.host().scope();
        let origin = self
            .contexts
            .scope_mut(scope)
            .ok_or(nocterm_agent_runtime::ChatOpenError::Closed)?;
        origin.chats_mut().append(slot);
        if !visible {
            origin.chats_mut().select(id);
        }
        Ok(visible)
    }
    pub fn find(&self, id: EntityId) -> Option<&Entity<AgentThread>> {
        self.contexts.current().chats().find(id)
    }
    pub fn contains(&self, id: EntityId) -> bool {
        self.contexts.current().chats().contains(id)
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.contexts.current().chats().iter()
    }
    pub fn is_empty(&self) -> bool {
        self.contexts.current().chats().is_empty()
    }
    /// Opens chat `id`, hiding the chat open before.
    pub fn select(&mut self, id: EntityId, cx: &mut App) {
        if let Some(previous) = self.current() {
            previous.update(cx, |thread, cx| thread.set_shown(false, cx));
        }
        self.contexts.current_mut().chats_mut().select(id);
        if let Some(current) = self.current() {
            current.update(cx, |thread, cx| thread.set_shown(true, cx));
        }
    }
    /// Binds an already admitted chat's background sessions to `window`;
    /// `observation` lives as long as the chat stays listed.
    pub fn track(
        &self,
        thread: &Entity<AgentThread>,
        window: AnyWindowHandle,
        observation: Subscription,
        cx: &mut App,
    ) -> ChatSlot {
        thread.update(cx, |thread, _| thread.bind_window(window));
        ChatSlot::new(thread.clone(), observation)
    }
    pub fn append(&mut self, slot: ChatSlot) {
        self.contexts.current_mut().chats_mut().append(slot);
    }
    pub fn prepend(&mut self, slots: Vec<ChatSlot>) {
        self.contexts.current_mut().chats_mut().prepend(slots);
    }
    /// Creates the chat `start` describes, or none for an unknown agent.
    pub fn start(
        &self,
        start: ChatStart,
        cx: &mut App,
    ) -> Result<Option<Entity<AgentThread>>, nocterm_agent_runtime::ChatOpenError> {
        let agent = match &start {
            ChatStart::New { agent, .. } => agent.as_str(),
            ChatStart::Restart { data, .. } => data.agent_id(),
        };
        if self.host().agents().get(agent, cx).is_none() {
            return Ok(None);
        }
        let agent = agent.to_owned();
        let thread = match start {
            ChatStart::New { agent, terminal } => {
                let thread = self.host().create_chat(agent, cx)?;
                if let Some(id) = terminal {
                    thread.update(cx, |thread, cx| {
                        thread.add_composer_attachment(Attachment::Terminal(id), cx)
                    });
                }
                thread
            }
            ChatStart::Restart {
                data,
                expected_owner,
            } => self.host().restart_chat(*data, expected_owner, cx)?,
        };
        self.host().runtime().set_last_agent(&agent, cx);
        Ok(Some(thread))
    }
    /// Copies loaded chat `source` up to and including entry `last` (all of
    /// it, with `None`) into a new, saved chat.
    pub fn fork(
        &self,
        source: &Entity<AgentThread>,
        last: Option<usize>,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, nocterm_agent_runtime::ChatOpenError> {
        let host = self
            .contexts
            .origin(source.entity_id())
            .ok_or(nocterm_agent_runtime::ChatOpenError::Closed)?
            .host();
        host.require_scope(source.read(cx).document().key().scope())?;
        let fork = match last {
            Some(last) => source.read(cx).fork_at(last + 1),
            None => source.read(cx).fork(),
        };
        let thread = host.fork_chat(fork, cx)?;
        thread.update(cx, |thread, cx| thread.persist(cx));
        Ok(thread)
    }
    /// The chats saved in earlier runs and not listed yet, newest first:
    /// loaded chats, then archived summaries.
    pub fn saved(&self, cx: &mut App) -> Vec<Entity<AgentThread>> {
        let Some(catalog) = self.host().runtime().catalog_snapshot(cx) else {
            return Vec::new();
        };
        let mut restored = Vec::new();
        for index in (0..catalog.rows().len()).rev() {
            let Some(row) = catalog.row(index) else {
                continue;
            };
            if self
                .contexts
                .current()
                .chats()
                .iter()
                .chain(&restored)
                .any(|thread| thread.read(cx).chat_id() == row.summary().id)
            {
                continue;
            }
            match self.host().restore_archive(row, cx) {
                Ok(thread) => restored.push(thread),
                Err(error) => tracing::warn!(%error, "could not admit archived chat"),
            }
        }
        restored
    }
    /// Captures chat `id`'s document so it survives removal.
    fn admit_before_removal(&self, id: EntityId, cx: &mut App) -> Result<(), RetainError> {
        if self.host().runtime().capture_and_detach_document(id, cx)?
            && let Some(thread) = self.contexts.current().chats().find(id)
        {
            thread.update(cx, |thread, cx| {
                thread.document_admitted(thread.document().desired_revision(), cx)
            });
        }
        Ok(())
    }
    /// Admits the documents of the chats `keep` rejects, before they are
    /// removed with [`Self::remove`]; chats whose documents could not be
    /// retained stay.
    pub fn admit_removals(
        &mut self,
        keep: impl Fn(&Entity<AgentThread>, &App) -> bool,
        cx: &mut App,
    ) -> Retained {
        let proposed: Vec<_> = self
            .contexts
            .current()
            .chats()
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
        Retained { removed, error }
    }
    /// Global AI disable captures every retained context before releasing slots.
    pub fn clear_all(&mut self, cx: &mut App) -> Retained {
        let clients: Vec<_> = self
            .contexts
            .all()
            .flat_map(|scope| {
                scope
                    .chats()
                    .iter()
                    .map(|thread| (scope.host().clone(), thread.clone()))
            })
            .collect();
        let mut removed = Vec::new();
        let mut error = None;
        for (host, thread) in clients {
            match host
                .runtime()
                .capture_and_detach_document(thread.entity_id(), cx)
            {
                Ok(admitted) => {
                    if admitted {
                        thread.update(cx, |thread, cx| {
                            thread.document_admitted(thread.document().desired_revision(), cx)
                        });
                    }
                    removed.push(thread.entity_id());
                }
                Err(failure) => error = Some(RetainError::from(failure)),
            }
        }
        self.contexts.remove_all(&removed);
        Retained { removed, error }
    }
    /// Removes admitted chats `ids` and their observations.
    pub fn remove(&mut self, ids: &[EntityId]) {
        self.contexts.current_mut().chats_mut().remove_admitted(ids);
    }
    /// Releases chat `id`'s agent and deletes its saved copy; the chat stays
    /// listed until it is removed.
    pub fn delete(&self, id: EntityId, cx: &mut App) -> bool {
        let Some(thread) = self.contexts.current().chats().find(id).cloned() else {
            return false;
        };
        let chat = thread.read(cx).document().chat_id().to_owned();
        thread.update(cx, |thread, cx| thread.release_resources(cx));
        self.host().runtime().delete_chat(chat, cx);
        true
    }
    /// Evicts the oldest resident transcripts other than the open chat's
    /// until history fits the budget. Returns the evicted chats whose
    /// transcripts mention `query`.
    pub fn trim_resident_history(&self, query: &str, cx: &mut App) -> Vec<String> {
        let matches: std::collections::HashSet<_> = self
            .contexts
            .current()
            .chats()
            .iter()
            .filter(|thread| !query.is_empty() && thread.read(cx).transcript().mentions(query))
            .map(|thread| thread.read(cx).document().chat_id().to_owned())
            .collect();
        match self.host().runtime().trim_history(cx) {
            Ok(evicted) => evicted
                .into_iter()
                .filter(|key| key.scope() == self.host().scope() && matches.contains(key.id()))
                .map(|key| key.id().to_owned())
                .collect(),
            Err(error) => {
                tracing::warn!(%error, "resident history cannot be trimmed");
                Vec::new()
            }
        }
    }
}
