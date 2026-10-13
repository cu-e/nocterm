//! A chat as saved in the history: restoring, forking, naming and saving.
use gpui_kit::{Context, WeakEntity};
use nocterm_ai::{acp, thread::ThreadState};
use nocterm_workspace::Workspace;

use super::{AgentThread, Composer, Fork, Restore};
use crate::runtime::Runtime;

impl AgentThread {
    /// A restored document connects when shown with warm start enabled.
    pub(crate) fn restored(
        chat: nocterm_ai::history::SavedChat,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut thread = Self::from_document(
            super::document::ChatDocument::new(
                chat.agent_id.clone(),
                chat.id.clone(),
                chat.updated,
            ),
            workspace,
            cx,
        );
        thread.restore_content(chat);
        thread.preview_config(cx);
        thread
    }
    pub(crate) fn restore_content(&mut self, chat: nocterm_ai::history::SavedChat) {
        self.restore_content_with_overlay(chat, None);
    }
    pub(super) fn restore_content_with_overlay(
        &mut self,
        mut chat: nocterm_ai::history::SavedChat,
        overlay: Option<super::archive::DraftOverlay>,
    ) {
        self.archive.hydration_committed(chat.resident_bytes());
        let pending_history =
            chat.pending_history || (chat.session_id.is_none() && !chat.entries.is_empty());
        let prepared_sizes = chat
            .prepared_budget()
            .map(|prepared| prepared.prompt_sizes().to_vec())
            .unwrap_or_default();
        if let Some(overlay) = &overlay {
            self.state
                .imported_with_overlay(&mut chat, pending_history, overlay.updated());
        } else {
            self.state.imported_clean(&mut chat, pending_history);
        }
        self.status = "Saved chat".into();
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
        if let Some(overlay) = overlay {
            self.composer.set_draft(overlay.text().to_owned());
        }
        self.restore = chat.session_id.map(|session| Restore {
            session: acp::SessionId::new(session),
            workdir: chat.workdir.unwrap_or_default(),
            fork: false,
        });
        self.config.restore(
            chat.config,
            chat.mode.map(acp::SessionModeId::new),
            chat.model,
        );
    }
    /// A copy of this chat, to go on with separately. When opened, the copy
    /// continues in a copy of this chat's agent session.
    pub(crate) fn fork(&self) -> Fork {
        self.fork_at(self.state.entries().len())
    }
    /// A copy of the chat's first `len` entries. Only a copy of the whole
    /// chat continues in a copy of the agent's session; an earlier cut
    /// starts a new session, since the agent cannot forget the rest.
    pub(crate) fn fork_at(&self, len: usize) -> Fork {
        let whole = len >= self.state.entries().len();
        let mut chat = nocterm_ai::history::SavedChat::new(self.state.agent_id().to_owned());
        let mut state = self.state.transcript().clone();
        state.truncate(len);
        chat.entries = state.entries;
        chat.times = state.times;
        chat.title = self.state.transcript().title.clone();
        chat.name = Some(format!("{} (fork)", self.title()));
        chat.model = self.model();
        chat.config = self.config.choices().clone();
        chat.mode = self.config.mode_choice().map(ToString::to_string);
        chat.pending_history = self.state.pending_history() || !whole;
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
        thread
            .state
            .set_pending_history(pending_history, nocterm_ai::time::now());
        thread.restore = fork.restore;
        thread.composer.replace_context(fork.attachments);
        thread.establish_clean_document_inputs(cx);
        thread.state.fork_created();
        thread.status = "Forked chat".into();
        thread
    }
    /// The chat's name: the user's, else the agent's title, else the start of
    /// the first message.
    pub(crate) fn title(&self) -> String {
        self.archive.summary().map_or_else(
            || chat_title(self.state.name(), self.state.transcript()),
            |summary| summary.title().to_owned(),
        )
    }
    /// The model of the last prompt, in this run or a saved one.
    pub(crate) fn model(&self) -> Option<String> {
        self.config.model()
    }
    pub(crate) fn rename(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.is_archived() {
            self.load_archive(super::archive::ArchiveIntent::Rename { name }, cx);
            return;
        }
        if self.state.rename(name, nocterm_ai::time::now()) {
            self.save(cx);
        }
        cx.notify();
    }
    pub(crate) fn set_pinned(&mut self, pinned: bool, cx: &mut Context<Self>) {
        if self.is_archived() {
            self.load_archive(super::archive::ArchiveIntent::Pin { pinned }, cx);
            return;
        }
        if self.state.pin(pinned, nocterm_ai::time::now()) {
            self.save(cx);
        }
        cx.notify();
    }
    pub(super) fn persistent_resume(&self) -> super::document::Resume {
        if self.state.entries().is_empty() {
            return super::document::Resume::default();
        }
        match (self.session(), &self.restore) {
            (Some(session), _) => super::document::Resume::new(
                Some(session.to_string()),
                self.session_workdir().clone(),
            ),
            (None, Some(restore)) if !restore.fork => super::document::Resume::new(
                Some(restore.session.to_string()),
                Some(restore.workdir.clone()),
            ),
            _ => super::document::Resume::default(),
        }
    }
    pub(super) fn history_metadata(&self, cx: &gpui_kit::App) -> nocterm_ai::history::SavedChat {
        let input = super::document::Inputs::new(
            &self.composer,
            &self.config,
            self.stable_attachments(self.composer.default_attachments(), cx),
            self.persistent_resume(),
        );
        self.state.metadata(&input)
    }
    pub(crate) fn shared_snapshot(
        &self,
        cx: &gpui_kit::App,
    ) -> Option<std::sync::Arc<nocterm_ai::history::SharedChat>> {
        if self.is_archived() {
            return None;
        }
        let input = super::document::Inputs::new(
            &self.composer,
            &self.config,
            self.stable_attachments(self.composer.default_attachments(), cx),
            self.persistent_resume(),
        );
        self.state.snapshot(&input)
    }
    pub(super) fn accept_document_inputs(&mut self, cx: &gpui_kit::App, draft: bool) -> bool {
        let attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        let resume = self.persistent_resume();
        let input = super::document::Inputs::new(&self.composer, &self.config, attachments, resume);
        self.state
            .accept_inputs(&input, nocterm_ai::time::now(), draft)
    }
    pub(super) fn establish_clean_document_inputs(&mut self, cx: &gpui_kit::App) {
        let attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        let resume = self.persistent_resume();
        let input = super::document::Inputs::new(&self.composer, &self.config, attachments, resume);
        self.state.establish_clean_inputs(&input);
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
    pub(crate) fn persist(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
    }
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        let Some((chat, revision)) = self.capture_pending(cx) else {
            return;
        };
        if !self.state.mark_normal_offer(revision) {
            return;
        }
        let owner = cx.entity_id();
        let thread = cx.weak_entity();
        cx.defer(move |cx| {
            let result = Runtime::global(cx).update(cx, |runtime, cx| {
                runtime.save_chat(chat, owner, revision, cx)
            });
            match result {
                Ok(true) => {
                    let _ = thread.update(cx, |thread, cx| thread.document_admitted(revision, cx));
                }
                Ok(false) => {}
                Err(error) => {
                    let _ = thread.update(cx, |thread, cx| {
                        thread.document_save_failed(revision, error, cx)
                    });
                }
            }
        });
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
