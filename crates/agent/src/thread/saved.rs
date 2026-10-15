//! A chat as saved in the history: restoring, forking, naming and saving.
use gpui_kit::{App, Context};
use nocterm_ai::{acp, thread::ThreadState};
use std::{sync::Arc, time::Duration};

use super::{AgentThread, Attachment, Composer, Fork, RestartData, Restore, archive};

impl AgentThread {
    pub(crate) fn send_now(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.composer.move_to_front(id) {
            return;
        }
        if self.session.generating() {
            self.stop(cx);
        }
        self.composer.resume();
        self.accept_document_inputs(cx, false);
        self.persist(cx);
        // When generating, only the old prompt's completion may dispatch.
        if !self.session.generating() {
            self.dispatch_next(cx);
        }
        cx.notify();
    }

    /// Drops the queued prompt `id` without sending it. A prompt being
    /// edited stays until its edit ends.
    pub(crate) fn remove_queued(&mut self, id: u64, editing: Option<u64>, cx: &mut Context<Self>) {
        if editing == Some(id) || !self.composer.remove(id) {
            return;
        }
        // A chat left with nothing else has no snapshot of its own; its file
        // must still lose the prompt, or it comes back on the next start.
        self.accept_document_inputs(cx, false);
        self.persist(cx);
        cx.notify();
    }

    pub(crate) fn end_composer_edit(&mut self, resume: bool, cx: &mut Context<Self>) {
        self.composer.end_edit();
        if resume {
            self.dispatch_next(cx);
        }
        cx.notify();
    }

    pub(crate) fn attachment_scope(&self) -> &[Attachment] {
        self.tools
            .attachment_scope(self.composer.default_attachments())
    }

    #[cfg(test)]
    pub(crate) fn pause_queue(&mut self, cx: &mut Context<Self>) {
        self.composer.pause();
        cx.notify();
    }

    pub(crate) fn begin_composer_edit(
        &mut self,
        images: Vec<nocterm_ai::images::PromptImage>,
        attachments: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) {
        self.composer.begin_edit(images, attachments);
        cx.notify();
    }

    pub(crate) fn remove_composer_image(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.composer.remove_image(index) {
            cx.notify();
        }
    }

    pub(crate) fn accept_composer_images(
        &mut self,
        images: Vec<nocterm_ai::images::PromptImage>,
        cx: &mut Context<Self>,
    ) -> Result<(), nocterm_ai::images::ImageCollectionError> {
        self.composer.replace_images(images)?;
        cx.notify();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn clear_composer_context(&mut self, cx: &mut Context<Self>) {
        self.replace_composer_context(Vec::new(), cx);
    }

    #[cfg(test)]
    pub(crate) fn replace_composer_context(
        &mut self,
        values: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) {
        let old = self.composer.attachments().to_vec();
        for value in old {
            if !values.contains(&value) {
                self.attach(value, cx);
            }
        }
        for value in values {
            if !self.composer.attachments().contains(&value) {
                self.attach(value, cx);
            }
        }
    }

    pub(crate) fn add_composer_attachment(
        &mut self,
        attachment: Attachment,
        cx: &mut Context<Self>,
    ) {
        self.composer.add_attachment_if_absent(attachment);
        cx.notify();
    }

    pub(super) fn configured(&mut self, cx: &mut Context<Self>) {
        self.session.opened();
        self.sync_config(true, cx);
        self.remember_options(cx);
        self.dispatch_next(cx);
        cx.notify();
    }

    pub(super) fn remember_options(&self, cx: &mut Context<Self>) {
        let (agent, options) = (
            self.state.agent_id().to_owned(),
            self.config.options().to_vec(),
        );
        let runtime = self.host.runtime().clone();
        cx.defer(move |cx| {
            runtime.remember_options(&agent, &options, cx);
        });
    }

    pub(super) fn sync_config(&mut self, save: bool, cx: &mut Context<Self>) {
        if self.config.sync_ready(self.session.phase()) && save {
            self.accept_document_inputs(cx, false);
            self.save(cx);
        }
    }

    pub(crate) fn set_mode(&mut self, id: acp::SessionModeId, cx: &mut Context<Self>) {
        if self.session.generating() || !self.host.enabled(cx) {
            return;
        }
        if self.config.choose_mode(id.clone()) {
            self.accept_document_inputs(cx, false);
            self.save(cx);
        }
        cx.notify();
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        if self.session.opening() {
            return;
        }
        let ticket = self.session.ticket();
        self.config.bind_request(ticket);
        self.start_configuration(
            ticket,
            false,
            commands,
            session,
            super::config::operation::ConfigurationRequest::Mode(id),
            cx,
        );
    }

    pub(crate) fn set_config(
        &mut self,
        id: acp::SessionConfigId,
        value: acp::SessionConfigOptionValue,
        cx: &mut Context<Self>,
    ) {
        if self.session.generating() || !self.host.enabled(cx) {
            return;
        }
        let choice = self.config.choose_option(&id, &value);
        if !choice.accepted() {
            return;
        }
        if choice.durable_changed() {
            self.accept_document_inputs(cx, false);
            self.save(cx);
        }
        if let Some((option, choice)) = choice.remember() {
            let agent = self.state.agent_id().to_owned();
            let runtime = self.host.runtime().clone();
            cx.defer(move |cx| {
                runtime.remember_choice(&agent, &option, &choice, cx);
            });
        }
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return cx.notify();
        };
        if self.session.opening() {
            return cx.notify();
        }
        let ticket = self.session.ticket();
        self.config.bind_request(ticket);
        self.start_configuration(
            ticket,
            false,
            commands,
            session,
            super::config::operation::ConfigurationRequest::Option(id, value),
            cx,
        );
        cx.notify();
    }

    pub(crate) fn preview_config(&mut self, cx: &mut Context<Self>) {
        let state = self.host.runtime().favorites(cx);
        self.config.preview(
            state.choices(self.state.agent_id()),
            state
                .options
                .get(self.state.agent_id())
                .map_or(&[], Vec::as_slice),
        );
        self.establish_clean_document_inputs(cx);
    }

    pub(crate) fn restore_descriptor(&self) -> Option<Restore> {
        self.session.restore_descriptor()
    }

    pub(crate) fn restarted(
        data: RestartData,
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let (document, config, composer, restore) = data.into_parts();
        let state = super::document::ChatDocument::restarted(document);
        let mut thread = Self::from_document(state, host, cx);
        thread.config.hydrate_restart(config);
        thread.composer = composer;
        thread.session.import_restore(restore);
        thread
    }

    pub(crate) fn restart_data(&self) -> RestartData {
        RestartData::new(
            self.state.restart_snapshot(),
            self.config.restart_snapshot(),
            self.composer.restarted(),
            self.restore_descriptor(),
        )
    }

    pub(crate) fn capture_pending(
        &mut self,
        cx: &App,
    ) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)> {
        self.state.cancel_debounce();
        if !self.state.needs_candidate() {
            return self.state.existing_offer();
        }
        if self.is_archived() {
            let snapshot = self
                .archive
                .draft_patch(self.state.chat_id(), self.state.agent_id())
                .map(|metadata| {
                    Arc::new(nocterm_ai::history::SharedChat {
                        metadata,
                        queue: Vec::new(),
                    })
                })?;
            return self.state.capture_patch(snapshot);
        }
        let attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        let resume = self.persistent_resume();
        let input = super::document::Inputs::new(&self.composer, &self.config, attachments, resume);
        self.state.propose_candidate(&input)
    }

    pub(crate) fn flush_snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        if self.state.clean() {
            return None;
        }
        if let Some(snapshot) = self.state.current_snapshot() {
            return Some(snapshot);
        }
        if self.is_archived() {
            return self
                .archive
                .draft_patch(self.state.chat_id(), self.state.agent_id())
                .map(|metadata| {
                    Arc::new(nocterm_ai::history::SharedChat {
                        metadata,
                        queue: Vec::new(),
                    })
                });
        }
        self.shared_snapshot(cx).or_else(|| {
            self.state.history_present().then(|| {
                Arc::new(nocterm_ai::history::SharedChat {
                    metadata: self.history_metadata(cx),
                    queue: Vec::new(),
                })
            })
        })
    }

    pub(crate) fn flush_draft(&mut self, cx: &mut Context<Self>) {
        if self.state.draft_changed() {
            self.save(cx);
        }
    }

    pub(crate) fn set_draft(&mut self, text: String, cx: &mut Context<Self>) {
        if self.is_archived() {
            let now = self.host.timestamp();
            if self.archive.edit_draft(text, now) {
                self.state.edited_archived_draft(now);
                cx.notify();
            }
            return;
        }
        if !self.composer.set_draft(text) {
            return;
        }
        self.accept_document_inputs(cx, true);
        if !self.state.has_debounce() {
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let _ = this.update(cx, |this, cx| this.save(cx));
            });
            self.state.replace_debounce(task);
        }
        cx.notify();
    }

    pub(crate) fn evict_history(&mut self, cx: &Context<Self>) -> bool {
        let gate = archive::EvictionGate::new(
            !self.session.leased(),
            !self.session_busy(),
            self.state.desired_revision() == self.state.published_revision(),
            !self.state.draft_changed(),
            self.state.persistence_error().is_none(),
            self.state.history_present(),
        );
        let mut summary = nocterm_ai::history::ChatSummary::from_chat(&self.history_metadata(cx));
        summary.title = Some(self.title());
        summary.has_content = !self.state.entries().is_empty() || !self.composer.is_empty();
        if !self.archive.request_evict(gate, summary) {
            return false;
        }
        self.state.evict_clean();
        self.composer = Default::default();
        self.tools.clear_displays();
        true
    }

    pub(crate) fn resident_history_bytes(&self) -> usize {
        if self.is_archived() {
            return 0;
        }
        self.archive
            .resident_bytes()
            .max(self.state.resident_bytes())
            + self
                .composer
                .queue()
                .iter()
                .map(|prompt| prompt.memory_len)
                .sum::<usize>()
            + self.composer.draft().map_or(0, String::len)
    }

    fn apply_archive_intent(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<Self>) {
        match intent {
            archive::ArchiveIntent::Rename { name } => self.rename(name, cx),
            archive::ArchiveIntent::Pin { pinned } => self.set_pinned(pinned, cx),
            archive::ArchiveIntent::Open {
                panel,
                source,
                revision,
            } => cx.defer(move |cx| {
                let _ = panel.update(cx, |panel, cx| panel.archive_opened(source, revision, cx));
            }),
            archive::ArchiveIntent::Fork {
                panel,
                source,
                window,
                last,
            } => cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.fork_loaded_archive(source, last, window, cx)
                    });
                });
            }),
        }
    }

    fn on_archive_loaded(
        &mut self,
        completion: archive::load_operation::LoadCompletion,
        cx: &mut Context<Self>,
    ) {
        let (load_id, ticket, result, permit) = completion.into_parts();

        let current = self.session.session_current(ticket)
            && self.host.enabled(cx)
            && self
                .host
                .runtime()
                .document_is_current(self.state.chat_id(), cx.entity_id(), cx);
        match self.archive.accept(load_id, current, result, permit) {
            archive::LoadOutcome::Stale => return,
            archive::LoadOutcome::Failed(error) => {
                self.notice
                    .present(format!("Could not load saved chat: {error}"));
                self.notice.mark_error();
            }
            archive::LoadOutcome::Hydrate {
                chat,
                overlay,
                intents,
            } => {
                self.restore_content_with_overlay(*chat, overlay);
                self.preview_config(cx);
                if self.state.draft_changed() {
                    self.save(cx);
                }
                for intent in intents {
                    self.apply_archive_intent(intent, cx);
                }
                self.warm(cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn load_archive(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<Self>) {
        let request = self.archive.request(intent, self.session.ticket());
        let archive::RequestOutcome::Start {
            load_id,
            ticket,
            chat_id,
        } = request
        else {
            if let archive::RequestOutcome::RunNow(intent) = request {
                self.apply_archive_intent(intent, cx);
            }
            return;
        };
        self.notice.present("Loading saved chat…".into());
        let reader = self.host.runtime().history_reader(cx);
        let operation =
            archive::load_operation::LoadOperation::new(load_id, ticket, chat_id, reader);
        let loading = cx.background_executor().spawn(operation.run());
        cx.spawn(async move |this, cx| {
            let completion = loading.await;
            let _ = this.update(cx, |this, cx| this.on_archive_loaded(completion, cx));
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn release_history_delivery(&mut self) {
        self.archive.release_delivery_permit();
    }

    /// A restored document connects when shown with warm start enabled.
    pub(crate) fn restored(
        chat: nocterm_ai::history::SavedChat,
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut thread = Self::from_document(
            super::document::ChatDocument::new(
                chat.agent_id.clone(),
                chat.id.clone(),
                chat.updated,
            ),
            host,
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
        self.notice.present("Saved chat".into());
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
        self.session
            .import_restore(chat.session_id.map(|session| Restore {
                session: acp::SessionId::new(session),
                workdir: chat.workdir.unwrap_or_default(),
                fork: false,
            }));
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
        let mut chat = self.host.new_chat(self.state.agent_id().to_owned());
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
        let restore = match (self.session(), self.session.restore()) {
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
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let pending_history =
            fork.chat.pending_history || (fork.restore.is_none() && !fork.chat.entries.is_empty());
        let mut thread = Self::restored(fork.chat, host, cx);
        thread
            .state
            .set_pending_history(pending_history, thread.host.timestamp());
        thread.session.import_restore(fork.restore);
        thread.composer.replace_context(fork.attachments);
        thread.establish_clean_document_inputs(cx);
        thread.state.fork_created();
        thread.notice.present("Forked chat".into());
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
        if self.state.rename(name, self.host.timestamp()) {
            self.save(cx);
        }
        cx.notify();
    }
    pub(crate) fn set_pinned(&mut self, pinned: bool, cx: &mut Context<Self>) {
        if self.is_archived() {
            self.load_archive(super::archive::ArchiveIntent::Pin { pinned }, cx);
            return;
        }
        if self.state.pin(pinned, self.host.timestamp()) {
            self.save(cx);
        }
        cx.notify();
    }
    pub(super) fn persistent_resume(&self) -> super::document::Resume {
        if self.state.entries().is_empty() {
            return super::document::Resume::default();
        }
        match (self.session(), self.session.restore()) {
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
            .accept_inputs(&input, self.host.timestamp(), draft)
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
        let runtime = self.host.runtime().clone();
        cx.defer(move |cx| {
            let result = runtime.save_chat(chat, owner, revision, cx);
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
