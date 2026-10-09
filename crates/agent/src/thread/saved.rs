//! A chat as saved in the history: restoring, forking, naming and saving.
use gpui_kit::{Context, WeakEntity};
use nocterm_ai::{acp, thread::ThreadState};
use nocterm_workspace::Workspace;

use super::{AgentThread, Composer, Fork, Restore};
use crate::runtime::Runtime;

impl AgentThread {
    /// A thread showing `chat` from history, connected only when work is submitted.
    pub(crate) fn restored(
        chat: nocterm_ai::history::SavedChat,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut thread = Self::new(chat.agent_id.clone(), workspace, cx);
        thread.restore_content(chat);
        thread
    }
    pub(crate) fn restore_content(&mut self, chat: nocterm_ai::history::SavedChat) {
        self.resident_archive_bytes = chat.resident_bytes();
        self.archive = None;
        self.loading_archive = false;
        let pending_history =
            chat.pending_history || (chat.session_id.is_none() && !chat.entries.is_empty());
        let prepared_sizes = chat
            .prepared_budget()
            .map(|prepared| prepared.prompt_sizes().to_vec())
            .unwrap_or_default();
        self.storage_budget = nocterm_ai::history::HistoryBudget::restored(chat.prepared_budget());
        self.chat_id = chat.id;
        self.history_queued = true;
        if self.pending_archive_draft.is_none() {
            self.updated = chat.updated;
        }
        self.status = "Saved chat".into();
        self.state.entries = chat.entries;
        self.state.times = chat.times;
        self.state.title = chat.title;
        self.name = chat.name;
        self.pinned = chat.pinned;
        let queue = chat
            .queue
            .into_iter()
            .enumerate()
            .map(|(index, saved)| {
                let attachments = super::queue::restore_attachments(&saved.attachments);
                super::queue::QueuedPrompt::prepared(
                    saved,
                    attachments,
                    prepared_sizes.get(index).copied(),
                )
            })
            .collect();
        self.composer = Composer::restored(
            chat.draft,
            super::queue::restore_attachments(&chat.attachments),
            queue,
        );
        if let Some(text) = self.pending_archive_draft.take() {
            self.composer.set_draft(text);
            self.draft_changed = true;
        }
        self.fallback_history = pending_history;
        self.restore = chat.session_id.map(|session| Restore {
            session: acp::SessionId::new(session),
            workdir: chat.workdir.unwrap_or_default(),
            fork: false,
        });
        self.last_model = chat.model;
    }
    /// A copy of this chat, to go on with separately. When opened, the copy
    /// continues in a copy of this chat's agent session.
    pub(crate) fn fork(&self) -> Fork {
        self.fork_at(self.state.entries.len())
    }
    /// A copy of the chat's first `len` entries. Only a copy of the whole
    /// chat continues in a copy of the agent's session; an earlier cut
    /// starts a new session, since the agent cannot forget the rest.
    pub(crate) fn fork_at(&self, len: usize) -> Fork {
        let whole = len >= self.state.entries.len();
        let mut chat = nocterm_ai::history::SavedChat::new(self.agent_id.clone());
        let mut state = self.state.clone();
        state.truncate(len);
        chat.entries = state.entries;
        chat.times = state.times;
        chat.title = self.state.title.clone();
        chat.name = Some(format!("{} (fork)", self.title()));
        chat.model = self.model();
        chat.pending_history = self.fallback_history || !whole;
        let restore = match (&self.session(), &self.restore) {
            (Some(session), _) => self.session_workdir().clone().map(|workdir| Restore {
                session: session.clone(),
                workdir,
                fork: true,
            }),
            (None, Some(restore)) => Some(Restore {
                fork: true,
                ..restore.clone()
            }),
            (None, None) => None,
        }
        .filter(|_| whole);
        Fork {
            chat,
            restore,
            attachments: self.composer.default_attachments().to_vec(),
        }
    }
    /// The chat `fork` made, connected only when work is submitted.
    pub(crate) fn forked(
        fork: Fork,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let pending_history =
            fork.chat.pending_history || (fork.restore.is_none() && !fork.chat.entries.is_empty());
        let mut thread = Self::restored(fork.chat, workspace, cx);
        thread.fallback_history = pending_history;
        thread.restore = fork.restore;
        thread.composer.attachments = fork.attachments;
        thread.status = "Forked chat".into();
        thread
    }
    /// The chat's name: the user's, else the agent's title, else the start of
    /// the first message.
    pub(crate) fn title(&self) -> String {
        self.archive.as_ref().map_or_else(
            || chat_title(self.name.as_deref(), &self.state),
            |summary| summary.title().to_owned(),
        )
    }
    /// The model of the last prompt, in this run or a saved one.
    pub(crate) fn model(&self) -> Option<String> {
        self.last_prompt
            .as_ref()
            .and_then(|prompt| prompt.model.clone())
            .or_else(|| self.last_model.clone())
    }
    pub(crate) fn rename(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.archive.is_some() {
            self.load_archive(cx, move |thread, cx| thread.rename(name, cx));
            return;
        }
        self.name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        self.save(cx);
        cx.notify();
    }
    pub(crate) fn set_pinned(&mut self, pinned: bool, cx: &mut Context<Self>) {
        if self.archive.is_some() {
            self.load_archive(cx, move |thread, cx| thread.set_pinned(pinned, cx));
            return;
        }
        self.pinned = pinned;
        self.save(cx);
        cx.notify();
    }
    /// Metadata shared by transcript, queue and unsent composer snapshots.
    pub(super) fn history_metadata(&self, cx: &gpui_kit::App) -> nocterm_ai::history::SavedChat {
        let mut chat = nocterm_ai::history::SavedChat::new(self.agent_id.clone());
        chat.id = self.chat_id.clone();
        chat.updated = self.updated;
        chat.title = self.state.title.clone();
        chat.name = self.name.clone();
        chat.pinned = self.pinned;
        chat.draft = self.composer.draft.clone();
        chat.attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        chat.times = nocterm_ai::history::SavedChat::bounded_times(
            self.state.entries.len(),
            &self.state.times,
        );
        chat.model = self.model();
        chat.pending_history = self.fallback_history;
        // Providers may not persist an empty session until its first prompt.
        if self.state.entries.is_empty() {
            return chat;
        }
        match (&self.session(), &self.restore) {
            (Some(session), _) => {
                chat.session_id = Some(session.0.to_string());
                chat.workdir = self.session_workdir().clone();
            }
            // A fork not yet opened has no session of its own: reopening the
            // original would continue it rather than a copy.
            (None, Some(restore)) if !restore.fork => {
                chat.session_id = Some(restore.session.0.to_string());
                chat.workdir = Some(restore.workdir.clone());
            }
            (None, _) => {}
        }
        chat
    }
    pub(crate) fn shared_snapshot(
        &self,
        cx: &gpui_kit::App,
    ) -> Option<std::sync::Arc<nocterm_ai::history::SharedChat>> {
        if self.archive.is_some() {
            return None;
        }
        if !self.pinned
            && self.name.is_none()
            && self.state.entries.is_empty()
            && self.composer.queue.is_empty()
            && self.composer.draft.is_none()
        {
            return None;
        }
        let mut metadata = self.history_metadata(cx);
        metadata.entries = nocterm_ai::history::SavedChat::bounded_entries(&self.state.entries);
        Some(std::sync::Arc::new(nocterm_ai::history::SharedChat {
            metadata,
            queue: self
                .composer
                .queue
                .iter()
                .map(|prompt| prompt.saved.clone())
                .collect(),
        }))
    }
    #[cfg(test)]
    pub(crate) fn snapshot(&self, cx: &gpui_kit::App) -> Option<nocterm_ai::history::SavedChat> {
        let shared = self.shared_snapshot(cx)?;
        let mut chat = shared.metadata.clone();
        chat.queue = shared
            .queue
            .iter()
            .map(|prompt| (**prompt).clone())
            .collect();
        Some(chat)
    }
    /// Marks the chat as changed now and saves it.
    pub(crate) fn persist(&mut self, cx: &mut Context<Self>) {
        self.updated = nocterm_ai::time::now();
        self.save(cx);
    }
    /// Saves transcript, queued prompts or an unsent draft to history.
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        if let Some(chat) = self.pending_snapshot(cx) {
            self.history_queued = true;
            self.draft_save = None;
            self.draft_changed = false;
            self.document_revision += 1;
            let revision = self.document_revision;
            // Deferred: this may run while the runtime itself is updating.
            let owner = cx.entity_id();
            cx.defer(move |cx| {
                Runtime::global(cx).update(cx, |runtime, cx| {
                    runtime.save_chat(chat, owner, revision, cx)
                })
            });
        }
    }
}

/// A chat's title: `name`, else the agent's title, else the start of the
/// first message. A chat with nothing in it is a new one.
pub(crate) fn chat_title(name: Option<&str>, state: &ThreadState) -> String {
    if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
        return name.to_owned();
    }
    if state.entries.is_empty() {
        return "New Agent".into();
    }
    state
        .title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .or_else(|| {
            state.entries.iter().find_map(|entry| match entry {
                nocterm_ai::thread::Entry::User(parts) => {
                    parts.iter().find_map(|part| match part {
                        acp::ContentBlock::Text(text) if !text.text.trim().is_empty() => {
                            Some(text.text.trim().chars().take(60).collect())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
        })
        .unwrap_or_else(|| "New Agent".into())
}
