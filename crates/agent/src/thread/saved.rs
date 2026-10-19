//! A chat as saved in the history: restoring, forking, naming and saving.
use crate::thread::document::{ChatDocument, DocumentError, ResumeFacts};
use gpui_kit::{App, Context};
use nocterm_ai::{
    history::{ChatMetadata, ChatSummary, ReaderDelivery},
    model::ids::{ConfigId, ModeId, SessionId},
    session_config::ConfigValue,
    thread::{ForkStaging, ThreadState},
};
use std::path::PathBuf;
use std::time::Duration;

use super::{AgentThread, Attachment, Fork, RestartData, Restore, archive};
mod archive_driver;
mod title;

impl AgentThread {
    pub(crate) fn send_now(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.queued_prompt_ids().any(|queued| queued == id) {
            return;
        }
        if let Err(error) = self.state.move_prompt(id, 0) {
            return self.composer_refused(error, cx);
        }
        if self.session.generating() {
            self.stop(cx);
        }
        self.composer.resume();
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
        if editing == Some(id) {
            return;
        }
        match self.state.remove_prompt(id) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => return self.composer_refused(error, cx),
        }
        // A chat left with nothing else has no snapshot of its own; its file
        // must still lose the prompt, or it comes back on the next start.
        self.persist(cx);
        cx.notify();
    }

    pub(crate) fn end_composer_edit(&mut self, resume: bool, cx: &mut Context<Self>) {
        if let Err(error) = self
            .state
            .edit_composer(nocterm_ai::history::physical::ComposerInputEdit::End)
        {
            return self.composer_refused(error, cx);
        }
        if resume {
            self.dispatch_next(cx);
        }
        cx.notify();
    }

    pub(in crate::thread) fn attachment_scope(&self) -> super::tool_runtime::TurnAttachments<'_> {
        self.tools
            .attachment_scope(self.state.default_attachments())
    }

    #[cfg(test)]
    pub(crate) fn pause_queue(&mut self, cx: &mut Context<Self>) {
        self.composer.pause();
        cx.notify();
    }

    /// Shows queued prompt `id`'s images and context for editing; false when it cannot be edited.
    pub(crate) fn begin_composer_edit(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        match self
            .state
            .edit_composer(nocterm_ai::history::physical::ComposerInputEdit::BeginOriginal(id))
        {
            Ok(()) => {
                cx.notify();
                true
            }
            Err(error) => {
                self.composer_refused(error, cx);
                false
            }
        }
    }

    /// An admitted copy of `bytes`, ready for inspection off the UI thread.
    pub(crate) fn prepare_image(
        &mut self,
        bytes: &[u8],
    ) -> Result<nocterm_ai::images::managed::ManagedImageRequest, super::document::DocumentError>
    {
        self.state.prepare_image(bytes)
    }

    pub(crate) fn remove_composer_image(&mut self, index: usize, cx: &mut Context<Self>) {
        match self
            .state
            .edit_composer(nocterm_ai::history::physical::ComposerInputEdit::RemoveImage(index))
        {
            Ok(()) => cx.notify(),
            Err(error) => self.composer_refused(error, cx),
        }
    }

    /// Each prepared image joins the composer; the first refusal stops the rest.
    pub(crate) fn accept_composer_image(
        &mut self,
        image: nocterm_ai::model::native::NativeOwned<
            nocterm_ai::images::managed::PreparedPromptImage,
        >,
        cx: &mut Context<Self>,
    ) -> Result<(), super::document::DocumentError> {
        self.state.add_prepared_image(image)?;
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
        let old = self
            .composer()
            .attachments()
            .iter()
            .map(|value| value.to_attachment())
            .collect::<Vec<_>>();
        for value in old {
            if !values.contains(&value) {
                self.attach(value, cx);
            }
        }
        for value in values {
            if !self.composer().attachments().contains(&value) {
                self.attach(value, cx);
            }
        }
    }

    pub(crate) fn add_composer_attachment(
        &mut self,
        attachment: Attachment,
        cx: &mut Context<Self>,
    ) {
        match self.edit_attachment(
            nocterm_ai::history::live::AttachmentEdit::AddIfAbsent(attachment.live_ref()),
            cx,
        ) {
            Ok(_) => cx.notify(),
            Err(error) => self.composer_refused(error, cx),
        }
    }

    pub(super) fn configured(&mut self, cx: &mut Context<Self>) {
        self.session.opened();
        self.sync_config(true, cx);
        self.remember_options(cx);
        self.dispatch_next(cx);
        cx.notify();
    }

    pub(super) fn remember_options(&self, cx: &mut Context<Self>) {
        let (agent, options) = (self.agent_name(), self.configuration().options());
        let runtime = self.host.runtime().clone();
        cx.defer(move |cx| {
            runtime.remember_options(&agent, &options, cx);
        });
    }

    /// The chat's agent identity, copied for a deferred runtime call.
    pub(super) fn agent_name(&self) -> String {
        self.state
            .agent_id()
            .map(|agent| agent.to_string())
            .unwrap_or_default()
    }

    pub(super) fn sync_config(&mut self, save: bool, cx: &mut Context<Self>) {
        let controls = super::controls::Controls::read(self.live_controls());
        if self.config.sync_ready(self.session.phase(), &controls) && save {
            self.record_resume();
            self.save(cx);
        }
    }

    pub(crate) fn set_mode(&mut self, id: ModeId, cx: &mut Context<Self>) {
        if self.session.generating() || !self.host.enabled(cx) {
            return;
        }
        if self.config.choose_mode(id.clone()) {
            self.record_resume();
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

    pub(crate) fn set_config(&mut self, id: ConfigId, value: ConfigValue, cx: &mut Context<Self>) {
        if self.session.generating() || !self.host.enabled(cx) {
            return;
        }
        let controls = super::controls::Controls::read(self.live_controls());
        let choice = self.config.choose_option(&id, &value, &controls);
        if !choice.accepted() {
            return;
        }
        if choice.durable_changed() {
            self.record_resume();
            self.save(cx);
        }
        if let Some((option, choice)) = choice.remember() {
            let agent = self.agent_name();
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
        let agent = self.agent_name();
        self.config.preview(
            state.choices(&agent),
            state.options.get(&agent).map_or(&[], Vec::as_slice),
        );
        self.record_resume();
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

    pub(crate) fn restart_data(&mut self) -> Result<RestartData, super::document::DocumentError> {
        let config = self.config.restart_snapshot();
        let composer = self.composer.restarted();
        let restore = self.restore_descriptor();
        let document = self.state.take_restart()?;
        Ok(RestartData::new(document, config, composer, restore))
    }

    pub(crate) fn capture_pending(
        &mut self,
        _cx: &App,
    ) -> Result<Option<u64>, super::document::DocumentError> {
        self.state.cancel_debounce();
        if self.state.is_retired() {
            return Ok(None);
        }
        if !self.is_archived() {
            self.record_resume();
        }
        self.state.prepare_candidate()
    }
    fn write_intent(&self) -> nocterm_ai::history::writing::ManagedWriteIntent {
        if self.is_archived() {
            nocterm_ai::history::writing::ManagedWriteIntent::ArchiveDraft {
                updated: self.state.updated(),
            }
        } else {
            nocterm_ai::history::writing::ManagedWriteIntent::Snapshot
        }
    }
    pub(crate) fn capture_into(
        &mut self,
        admission: &crate::runtime::CaptureAdmission,
        cx: &mut App,
    ) -> Result<Option<u64>, crate::runtime::CaptureError> {
        let revision = self.state.desired_revision();
        let captured = self.capture_pending(cx).map_err(|error| {
            crate::runtime::CaptureError::persistence(
                revision,
                nocterm_store::StoreError::backend(error).into(),
            )
        })?;
        let Some(revision) = captured else {
            return Ok(None);
        };
        let offered = self.state.offer(self.write_intent()).map_err(|error| {
            crate::runtime::CaptureError::persistence(
                revision,
                nocterm_store::StoreError::backend(error).into(),
            )
        })?;
        let accepted = admission
            .save(offered, revision, cx)
            .map_err(|error| crate::runtime::CaptureError::persistence(revision, error))?;
        if accepted {
            self.state.acknowledge_admission(revision);
        }
        Ok(accepted.then_some(revision))
    }
    pub(crate) fn flush_draft(&mut self, cx: &mut Context<Self>) {
        if self.state.draft_changed() {
            self.save(cx);
        }
    }

    pub(crate) fn set_draft(&mut self, text: String, cx: &mut Context<Self>) {
        if self.is_archived() {
            let now = self.host.timestamp();
            if self.archive.edit_draft(text.clone(), now) {
                // The empty archived document carries the draft the patch writes.
                if let Err(error) = self.state.set_draft(&text, now) {
                    self.notice.present(error.to_string());
                    self.notice.mark_error();
                }
                self.state.edited_archived_draft();
                cx.notify();
            }
            return;
        }
        match self.state.set_draft(&text, self.host.timestamp()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                self.notice.present(error.to_string());
                self.notice.mark_error();
                cx.notify();
                return;
            }
        }
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
        let Some(summary) = self.eviction_summary() else {
            return false;
        };
        if !self.archive.request_evict(gate, summary) {
            return false;
        }
        if !self.state.evict_clean(self.host.retention(cx)) {
            return false;
        }
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
    }

    pub(crate) fn load_archive(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<Self>) {
        archive_driver::ArchiveDriver::new(self).load(intent, cx);
    }
    pub(super) fn composer_refused(
        &mut self,
        error: super::document::DocumentError,
        cx: &mut Context<Self>,
    ) {
        self.notice.present(error.to_string());
        self.notice.mark_error();
        cx.notify();
    }
    pub(crate) fn release_history_delivery(&mut self) {
        self.archive.release_delivery_permit();
    }

    /// The chat's name: the user's, else the agent's title, else the start of
    /// the first message.
    pub(crate) fn title(&self) -> String {
        self.archive.summary().map_or_else(
            || {
                self.state.transcript().map_or_else(
                    || "New Agent".to_owned(),
                    |state| chat_title(self.state.name().as_deref(), state),
                )
            },
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
        match self.state.rename(name.as_deref(), self.host.timestamp()) {
            Ok(true) => self.save(cx),
            Ok(false) => {}
            Err(error) => self
                .notice
                .present(format!("Could not rename chat: {error}")),
        }
        cx.notify();
    }
    pub(crate) fn set_pinned(&mut self, pinned: bool, cx: &mut Context<Self>) {
        if self.is_archived() {
            self.load_archive(super::archive::ArchiveIntent::Pin { pinned }, cx);
            return;
        }
        match self.state.pin(pinned, self.host.timestamp()) {
            Ok(true) => self.save(cx),
            Ok(false) => {}
            Err(error) => self.notice.present(format!("Could not pin chat: {error}")),
        }
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn shared_snapshot(
        &self,
        _cx: &App,
    ) -> Option<&nocterm_ai::history::physical::SnapshotOwner> {
        if self.is_archived() {
            None
        } else {
            self.state.snapshot()
        }
    }
    #[cfg(test)]
    pub(crate) fn flush_snapshot(
        &self,
        _cx: &App,
    ) -> Option<&nocterm_ai::history::physical::SnapshotOwner> {
        if self.state.clean() {
            None
        } else {
            self.state.snapshot()
        }
    }
    #[cfg(test)]
    pub(crate) fn snapshot(&self, cx: &App) -> Option<nocterm_ai::history::SavedChat> {
        let owner = self.shared_snapshot(cx)?;
        let bytes =
            serde_json::to_vec(&nocterm_ai::history::physical::SnapshotWire::new(owner)).ok()?;
        let id = self.chat_id()?.to_string();
        nocterm_ai::history::decode(self.host.retention(cx), &bytes, &id).ok()
    }
    pub(crate) fn persist(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
    }
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        if self.state.is_retired() {
            return;
        }
        let captured = match self.capture_pending(cx) {
            Ok(captured) => captured,
            Err(error) => {
                self.document_save_failed(
                    self.state.desired_revision(),
                    nocterm_store::StoreError::backend(error).into(),
                    cx,
                );
                return;
            }
        };
        let Some(revision) = captured else {
            return;
        };
        if !self.state.mark_normal_offer(revision) {
            return;
        }
        let owner = cx.entity_id();
        let runtime = self.host.runtime().clone();
        let result = match self.state.offer(self.write_intent()) {
            Ok(offer) => runtime.save_chat(offer, owner, revision, cx),
            Err(error) => Err(nocterm_store::StoreError::backend(error).into()),
        };
        match result {
            Ok(true) => self.document_admitted(revision, cx),
            Ok(false) => {}
            Err(error) => self.document_save_failed(revision, error, cx),
        }
    }
    /// A detached chat's final capture is owned by the runtime, so a disabled
    /// agent's rows can go at once instead of waiting for a publication event.
    pub(crate) fn release_captured_disabled_document(&mut self, cx: &gpui_kit::App) {
        if !self.host.enabled(cx) && self.state.release_captured(self.host.retention(cx)) {
            self.config.disabled();
            self.composer.reset_after_disable();
        }
    }

    pub(super) fn clear_settled_disabled_document(&mut self, cx: &gpui_kit::App) {
        if !self.host.enabled(cx) && self.state.release_disabled(self.host.retention(cx)) {
            self.config.disabled();
            self.composer.reset_after_disable();
        }
    }
    pub(crate) fn document_admitted(&mut self, revision: u64, cx: &mut Context<Self>) {
        self.state.acknowledge_admission(revision);
        self.clear_settled_disabled_document(cx);
        cx.notify();
    }
    pub(super) fn document_save_failed(
        &mut self,
        revision: u64,
        error: crate::runtime::AgentPersistenceError,
        cx: &mut Context<Self>,
    ) {
        let message = format!(
            "Could not save chat: {error}. Queued messages are retained; check disk space before continuing."
        );
        if !self.state.failed(revision, error) {
            return;
        }
        self.notice.present(message);
        self.composer.pause();
        self.notice.mark_error();
        cx.notify();
    }
    /// A saved chat's delivered rows replace the empty document; its resume facts,
    /// draft overlay and paused queue follow.
    pub(super) fn restore_delivery(
        &mut self,
        delivery: ReaderDelivery,
        overlay: Option<archive::DraftOverlay>,
    ) -> Result<(), DocumentError> {
        self.state.install_delivery(delivery)?;
        self.archive
            .hydration_committed(self.state.resident_bytes());
        if let Some(overlay) = overlay {
            self.state.set_draft(overlay.text(), overlay.updated())?;
        }
        let recorded = self.state.recorded_resume()?;
        self.notice.present("Saved chat".into());
        self.session
            .import_restore(recorded.session.map(|session| Restore {
                session: SessionId::new(session),
                workdir: recorded.workdir.unwrap_or_default(),
                fork: false,
            }));
        self.config.restore(
            recorded.choices,
            recorded.mode.map(ModeId::new),
            recorded.model,
        );
        self.composer = super::Composer::restored(recorded.highest_queued);
        Ok(())
    }

    /// The agent session this chat would resume, with its working directory.
    fn persistent_resume(&self) -> Option<(String, Option<PathBuf>)> {
        if self.state.rows().is_empty() {
            return None;
        }
        match (self.session(), self.session.restore()) {
            (Some(session), _) => Some((session.to_string(), self.session_workdir().clone())),
            (None, Some(restore)) if !restore.fork => {
                Some((restore.session.to_string(), Some(restore.workdir.clone())))
            }
            _ => None,
        }
    }

    /// Copies the session, working directory, mode and chosen options into the document.
    pub(in crate::thread) fn record_resume(&mut self) {
        let resume = self.persistent_resume();
        let mode = self.config.mode_choice().map(ToString::to_string);
        let model = self.config.model();
        let facts = ResumeFacts {
            session: resume.as_ref().map(|(session, _)| session.as_str()),
            workdir: resume.as_ref().and_then(|(_, workdir)| workdir.as_deref()),
            mode: mode.as_deref(),
            model: model.as_deref(),
            choices: self.config.choices(),
        };
        if let Err(error) = self.state.sync_resume(facts) {
            tracing::warn!(%error, "resume facts were not retained");
        }
    }

    /// The catalog entry the chat keeps once its rows are released.
    pub(super) fn eviction_summary(&self) -> Option<ChatSummary> {
        let snapshot = self.state.snapshot()?;
        let metadata = ChatMetadata {
            agent_id: snapshot.metadata().agent_id().to_string(),
            title: self
                .state
                .transcript()
                .and_then(|state| state.title())
                .map(|title| title.to_string()),
            name: snapshot.metadata().name().map(|name| name.to_string()),
            pinned: snapshot.metadata().pinned(),
            updated: snapshot.metadata().updated(),
            model: self.config.model(),
        };
        let has_content = !self.state.rows().is_empty() || !self.composer().is_empty();
        let mut summary =
            ChatSummary::from_projection(&snapshot.id(), &metadata, has_content, None);
        summary.title = Some(self.title());
        Some(summary)
    }

    /// A copy of this chat, to go on with separately. When opened, the copy
    /// continues in a copy of this chat's agent session.
    pub(crate) fn fork(&self, cx: &App) -> Result<Fork, DocumentError> {
        self.fork_at(self.state.rows().len(), cx)
    }

    /// A copy of the chat's first `len` rows. Only a copy of the whole
    /// chat continues in a copy of the agent's session; an earlier cut
    /// starts a new session, since the agent cannot forget the rest.
    pub(crate) fn fork_at(&self, len: usize, cx: &App) -> Result<Fork, DocumentError> {
        let source = self.state.transcript().ok_or(DocumentError::Retired)?;
        let whole = len >= self.state.rows().len();
        let identity = self.host.new_identity();
        let name = format!("{} (fork)", self.title());
        let pending = self.state.pending_history() || !whole;
        let state = ForkStaging::new(source, len).finish(
            self.host.retention(cx),
            &identity.id,
            &name,
            pending,
            identity.created,
        )?;
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
        Ok(Fork {
            state,
            restore,
            attachments: self
                .state
                .default_attachments()
                .iter()
                .map(|attachment| attachment.to_attachment())
                .collect(),
            choices: self.config.choices().clone(),
            mode: self.config.mode_choice().cloned(),
            model: self.config.model(),
        })
    }

    /// The chat `fork` made, connected only when work is submitted.
    pub(crate) fn prepare_forked(
        fork: Fork,
        host: crate::chat_host::ChatHost,
    ) -> Result<impl FnOnce(&mut Context<Self>) -> Self, DocumentError> {
        let Fork {
            state,
            restore,
            attachments,
            choices,
            mode,
            model,
        } = fork;
        let document = ChatDocument::from_state(state);
        Ok(move |cx: &mut Context<Self>| {
            let mut thread = Self::from_document(document, host, cx);
            thread.config.restore(choices, mode, model);
            thread.session.import_restore(restore);
            for attachment in &attachments {
                if let Err(error) = thread.edit_attachment(
                    nocterm_ai::history::live::AttachmentEdit::AddIfAbsent(attachment.live_ref()),
                    cx,
                ) {
                    tracing::warn!(%error, "forked context was not retained");
                }
            }
            thread.record_resume();
            thread.state.fork_created();
            thread.notice.present("Forked chat".into());
            thread
        })
    }
}

/// A chat's title reads its actual physical transcript without copying entries.
pub(crate) fn chat_title(name: Option<&str>, state: &ThreadState) -> String {
    title::ChatTitle::new(name, state).display()
}
