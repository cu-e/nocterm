//! UI revision receipts surround the original managed document; payload ownership
//! stays in its Live/Candidate/Reader/Writer/Source capabilities.
use gpui_kit::Task;
use nocterm_ai::{
    history::{
        self,
        live::{AppliedUpdate, LiveCopies, LiveEpoch, LiveError},
        physical::{RowSlot, SnapshotOwner, TextGuard},
        source::{ManagedAgentInfo, ManagedNotice, ManagedOpened},
        writing::{CandidateOffer, ManagedWriteIntent},
    },
    model::{native::NativeOwned, retention::RetentionAuthority},
    thread::{ThreadState, ThreadStateError},
};
use std::collections::HashSet;
mod revision;
use revision::PersistenceState;

/// The resumable facts a restored document recorded.
pub(super) struct RecordedResume {
    pub(super) session: Option<String>,
    pub(super) workdir: Option<std::path::PathBuf>,
    pub(super) mode: Option<String>,
    pub(super) choices: nocterm_ai::session_config::Choices,
    pub(super) model: Option<String>,
    pub(super) highest_queued: Option<u64>,
}

/// One archived chat's id, reader identity and transcript source.
pub(crate) type SearchJob = (String, history::ReaderDocumentId, history::ReaderSource);
pub(crate) struct ChatDocument {
    physical: Option<ThreadState>,
    persistence: PersistenceState,
    changed_rows: HashSet<usize>,
    draft_debounce: Option<Task<()>>,
}
pub(super) struct DocumentRestart {
    physical: ThreadState,
    persistence: PersistenceState,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum DocumentError {
    #[error("document ownership was transferred")]
    Retired,
    #[error(transparent)]
    State(#[from] ThreadStateError),
    #[error(transparent)]
    Live(#[from] LiveError),
    #[error(transparent)]
    History(#[from] history::HistoryCodecError),
    #[error(transparent)]
    Image(#[from] nocterm_ai::images::managed::ImageOperationError),
    #[error(transparent)]
    Install(#[from] history::ReaderSourceError),
    #[error(transparent)]
    Prepare(#[from] nocterm_ai::history::source::ManagedCommandError),
}
/// Request framing the feature chose for the next prompt; copied natively at dispatch.
pub(super) struct DispatchRequest<'a> {
    pub(super) session: &'a nocterm_ai::model::ids::SessionId,
    pub(super) context: Option<&'a str>,
    pub(super) history: Option<&'a str>,
    pub(super) command: bool,
}
impl<'a> DispatchRequest<'a> {
    pub(super) fn new(
        session: &'a nocterm_ai::model::ids::SessionId,
        context: Option<&'a str>,
        history: Option<&'a str>,
        command: bool,
    ) -> Self {
        Self {
            session,
            context,
            history,
            command,
        }
    }
}
/// The resumable facts of a connected chat, copied from the session at one moment.
pub(super) struct ResumeFacts<'a> {
    pub(super) session: Option<&'a str>,
    pub(super) workdir: Option<&'a std::path::Path>,
    pub(super) mode: Option<&'a str>,
    pub(super) model: Option<&'a str>,
    pub(super) choices: &'a nocterm_ai::session_config::Choices,
}
impl ChatDocument {
    /// The resumable facts the document recorded, read back after a restore.
    pub(super) fn recorded_resume(&self) -> Result<RecordedResume, DocumentError> {
        let snapshot = self.snapshot().ok_or(DocumentError::Retired)?;
        let resume = snapshot.resume();
        let choices = resume
            .config()
            .iter()
            .map(|choice| {
                let value = match choice.value() {
                    nocterm_ai::history::physical::ChoiceValueSlot::Flag(flag) => {
                        nocterm_ai::session_config::ConfigValue::Flag(*flag)
                    }
                    nocterm_ai::history::physical::ChoiceValueSlot::Value(text) => {
                        nocterm_ai::session_config::ConfigValue::Value(text.read().to_string())
                    }
                };
                (choice.key().to_string(), value)
            })
            .collect();
        Ok(RecordedResume {
            session: resume.session_id().map(|value| value.to_string()),
            workdir: resume.workdir().map(|value| value.to_path_buf()),
            mode: resume.mode().map(|value| value.to_string()),
            choices,
            model: snapshot.metadata().model().map(|value| value.to_string()),
            highest_queued: snapshot
                .composer()
                .queue()
                .iter()
                .map(|prompt| prompt.id())
                .max(),
        })
    }

    pub(super) fn new(
        retention: &RetentionAuthority,
        agent: &str,
        id: &str,
        updated: u64,
    ) -> Result<Self, DocumentError> {
        let physical = ThreadState::new(retention, agent, id, updated)?;
        Ok(Self {
            physical: Some(physical),
            persistence: PersistenceState::default(),
            changed_rows: HashSet::new(),
            draft_debounce: None,
        })
    }
    pub(super) fn from_state(physical: ThreadState) -> Self {
        Self {
            physical: Some(physical),
            persistence: PersistenceState::default(),
            changed_rows: HashSet::new(),
            draft_debounce: None,
        }
    }
    /// The archived chat's draft was edited without its document resident.
    pub(super) fn edited_archived_draft(&mut self) {
        self.changed(true);
    }
    pub(super) fn archived_clean(
        &mut self,
        summary: &history::ChatSummary,
    ) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        live.rename(live.local_admission(), summary.name.as_deref())?;
        live.stamp_metadata(live.local_admission(), summary.updated, summary.pinned)?;
        self.imported_clean();
        Ok(())
    }
    pub(super) fn restarted(data: DocumentRestart) -> Self {
        let mut persistence = data.persistence;
        persistence.restarted();
        Self {
            physical: Some(data.physical),
            persistence,
            changed_rows: HashSet::new(),
            draft_debounce: None,
        }
    }
    pub(crate) fn is_retired(&self) -> bool {
        self.physical.is_none()
    }
    pub(crate) fn transcript(&self) -> Option<&ThreadState> {
        self.physical.as_ref()
    }
    pub(crate) fn physical_mut(&mut self) -> Result<&mut ThreadState, DocumentError> {
        self.physical.as_mut().ok_or(DocumentError::Retired)
    }
    pub(crate) fn snapshot(&self) -> Option<&SnapshotOwner> {
        self.physical
            .as_ref()
            .and_then(|state| state.live().role().snapshot())
    }
    pub(crate) fn rows(&self) -> &[RowSlot] {
        self.physical.as_ref().map_or(&[], ThreadState::rows)
    }
    pub(super) fn composer(&self) -> Option<&history::physical::ComposerSlot> {
        self.snapshot().map(SnapshotOwner::composer)
    }
    pub(super) fn controls(&self) -> Option<&history::physical::LiveControls> {
        self.snapshot()?.presentation()?.controls()
    }
    pub(super) fn inputs(&self) -> Option<&history::physical::LiveInputs> {
        self.snapshot().and_then(SnapshotOwner::live_inputs)
    }
    pub(super) fn default_attachments(&self) -> super::composer::AttachmentSelection<'_> {
        use super::composer::AttachmentSelection;
        self.inputs().map_or_else(
            || {
                AttachmentSelection::Saved(
                    self.composer()
                        .map_or(&[], |composer| composer.attachments()),
                )
            },
            |inputs| AttachmentSelection::Live(inputs.default_attachments()),
        )
    }
    pub(super) fn edit_attachments(
        &mut self,
        edit: history::live::AttachmentEdit<'_>,
        locals: &[history::live::LocalAttachmentFact<'_>],
        updated: u64,
    ) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let (applied, removed) =
            live.edit_attachments(live.local_admission(), edit, locals, updated)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(removed)
    }
    pub(super) fn prepare_reader_id(&mut self) -> Result<history::ReaderDocumentId, DocumentError> {
        self.physical_mut()?
            .live_mut()
            .prepare_reader_id()
            .map_err(DocumentError::from)
    }
    pub(super) fn prepare_search(&mut self) -> Option<SearchJob> {
        let id = self.chat_id()?.to_string();
        let source = self.transcript()?.reader().clone();
        let document = self.prepare_reader_id().ok()?;
        Some((id, document, source))
    }
    pub(super) fn prepare_image(
        &mut self,
        bytes: &[u8],
    ) -> Result<nocterm_ai::images::managed::ManagedImageRequest, DocumentError> {
        self.physical_mut()?
            .live_mut()
            .prepare_image_request(bytes)
            .map_err(DocumentError::from)
    }
    pub(super) fn add_prepared_image(
        &mut self,
        image: NativeOwned<nocterm_ai::images::managed::PreparedPromptImage>,
    ) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        live.add_prepared_image(live.local_admission(), image)?;
        Ok(())
    }
    pub(super) fn edit_composer(
        &mut self,
        edit: history::physical::ComposerInputEdit,
    ) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        live.edit_inputs(live.local_admission(), edit)?;
        Ok(())
    }
    pub(super) fn remove_prompt(&mut self, id: u64) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let changed = live
            .remove_prompt(live.local_admission(), id)?
            .saved_changed();
        if changed {
            self.changed(false);
        }
        Ok(changed)
    }
    pub(super) fn move_prompt(&mut self, id: u64, target: usize) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let changed = live
            .move_prompt(live.local_admission(), id, target)?
            .saved_changed();
        if changed {
            self.changed(false);
        }
        Ok(changed)
    }
    pub(crate) fn agent_id(&self) -> Option<TextGuard<'_>> {
        self.snapshot()
            .map(|snapshot| snapshot.metadata().agent_id())
    }
    pub(crate) fn chat_id(&self) -> Option<TextGuard<'_>> {
        self.snapshot().map(SnapshotOwner::id)
    }
    pub(crate) fn name(&self) -> Option<TextGuard<'_>> {
        self.snapshot()
            .and_then(|snapshot| snapshot.metadata().name())
    }
    pub(crate) fn pinned(&self) -> bool {
        self.snapshot()
            .is_some_and(|snapshot| snapshot.metadata().pinned())
    }
    pub(crate) fn updated(&self) -> u64 {
        self.snapshot()
            .map_or(0, |snapshot| snapshot.metadata().updated())
    }
    pub(crate) fn pending_history(&self) -> bool {
        self.snapshot()
            .is_some_and(|snapshot| snapshot.resume().pending_history())
    }
    pub(crate) fn resident_bytes(&self) -> usize {
        self.physical
            .as_ref()
            .and_then(|state| {
                state
                    .live()
                    .role()
                    .scope()
                    .retained_and_pending_bytes()
                    .ok()
            })
            .unwrap_or(usize::MAX)
    }
    pub(crate) fn desired_revision(&self) -> u64 {
        self.persistence.desired()
    }
    pub(crate) fn published_revision(&self) -> u64 {
        self.persistence.published_revision()
    }
    pub(crate) fn persistence_error(&self) -> Option<&crate::runtime::AgentPersistenceError> {
        self.persistence.failure()
    }
    pub(crate) fn history_present(&self) -> bool {
        self.persistence.history_present()
    }
    pub(crate) fn draft_changed(&self) -> bool {
        self.persistence.draft_changed()
    }
    pub(crate) fn clean(&self) -> bool {
        self.persistence.clean()
    }
    pub(super) fn changed(&mut self, draft: bool) {
        self.persistence.changed(draft);
    }
    pub(super) fn fork_created(&mut self) {
        self.persistence.fork_created();
    }
    pub(super) fn imported_clean(&mut self) {
        self.persistence.imported_clean();
        self.changed_rows.clear();
    }
    pub(super) fn apply_notice(
        &mut self,
        epoch: &LiveEpoch,
        notice: ManagedNotice,
        now: u64,
    ) -> Result<AppliedUpdate, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let applied = live.apply_certified(live.admission(epoch)?, notice, now)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(applied)
    }
    pub(super) fn open_epoch(
        &mut self,
        epoch: LiveEpoch,
        opened: ManagedOpened,
    ) -> Result<AppliedUpdate, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        live.accept_epoch(epoch.clone());
        let applied = live.install_opened(live.admission(&epoch)?, opened)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(applied)
    }
    pub(super) fn install_info(&mut self, info: ManagedAgentInfo) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        live.install_info(live.local_admission(), info)?;
        Ok(())
    }
    pub(super) fn apply_options(
        &mut self,
        epoch: &LiveEpoch,
        options: NativeOwned<Vec<nocterm_ai::model::controls::ConfigOption>>,
        now: u64,
    ) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let admission = live.admission(epoch)?;
        let applied = LiveCopies::new(live)?.configuration(admission, options, now)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(())
    }
    pub(super) fn apply_mode(
        &mut self,
        epoch: &LiveEpoch,
        mode: &nocterm_ai::model::ids::ModeId,
        now: u64,
    ) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let admission = live.admission(epoch)?;
        let applied = LiveCopies::new(live)?.mode(admission, mode, now)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(())
    }
    pub(super) fn finish_turn(
        &mut self,
        tokens: Option<nocterm_ai::model::usage::TokenTotals>,
    ) -> Result<(), DocumentError> {
        self.physical_mut()?.live_mut().finish_turn(tokens)?;
        Ok(())
    }
    pub(super) fn set_pending_history(
        &mut self,
        pending: bool,
        now: u64,
    ) -> Result<bool, DocumentError> {
        let pinned = self.pinned();
        let live = self.physical_mut()?.live_mut();
        let applied = live.set_pending_history(live.local_admission(), pending)?;
        if applied.saved_changed() {
            live.stamp_metadata(live.local_admission(), now, pinned)?;
            self.changed(false);
        }
        Ok(applied.saved_changed())
    }
    pub(super) fn patch_display(
        &mut self,
        row: usize,
        display: &nocterm_ai::model::tools::ClientDisplay,
    ) -> Result<(), DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let admission = live.local_admission();
        let applied = LiveCopies::new(live)?.patch_display(admission, row, display)?;
        self.mark_row(row);
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(())
    }
    pub(super) fn finish_pending_tools(&mut self, now: u64) -> Result<(), DocumentError> {
        let pinned = self.pinned();
        let live = self.physical_mut()?.live_mut();
        let rows = live.finish_pending_tools(live.local_admission())?;
        if rows.as_slice().is_empty() {
            return Ok(());
        }
        live.stamp_metadata(live.local_admission(), now, pinned)?;
        for row in rows {
            self.mark_row(row);
        }
        self.changed(false);
        Ok(())
    }
    /// Copies the resumable facts into the document; each unchanged one leaves it clean.
    pub(super) fn sync_resume(&mut self, facts: ResumeFacts<'_>) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let mut changed = live
            .set_resume_session(live.local_admission(), facts.session)?
            .saved_changed();
        changed |= live
            .set_resume_mode(live.local_admission(), facts.mode)?
            .saved_changed();
        let admission = live.local_admission();
        changed |= LiveCopies::new(live)?
            .workdir(admission, facts.workdir)?
            .saved_changed();
        let admission = live.local_admission();
        changed |= LiveCopies::new(live)?
            .model(admission, facts.model)?
            .saved_changed();
        for (key, value) in facts.choices.iter() {
            let admission = live.local_admission();
            changed |= LiveCopies::new(live)?
                .choice(admission, key, value)?
                .saved_changed();
        }
        if changed {
            self.changed(false);
        }
        Ok(changed)
    }
    pub(super) fn install_delivery(
        &mut self,
        delivery: history::ReaderDelivery,
    ) -> Result<(), DocumentError> {
        // The refused delivery recycles its capabilities when dropped here.
        self.physical_mut()?
            .live_mut()
            .install_delivery(delivery)
            .map_err(|error| DocumentError::Install(error.into_parts().1))
    }
    /// A fresh empty document keeps the chat's identity and metadata, so a
    /// clean chat releases its rows until it is shown again.
    fn replace_with_empty(&mut self, retention: &RetentionAuthority) -> Result<(), DocumentError> {
        let (agent, id, name, pinned, updated, pending) = {
            let snapshot = self.snapshot().ok_or(DocumentError::Retired)?;
            (
                snapshot.metadata().agent_id().to_string(),
                snapshot.id().to_string(),
                snapshot.metadata().name().map(|name| name.to_string()),
                snapshot.metadata().pinned(),
                snapshot.metadata().updated(),
                snapshot.resume().pending_history(),
            )
        };
        let mut fresh = ThreadState::new(retention, &agent, &id, updated)?;
        let live = fresh.live_mut();
        live.rename(live.local_admission(), name.as_deref())?;
        live.stamp_metadata(live.local_admission(), updated, pinned)?;
        live.set_pending_history(live.local_admission(), pending)?;
        self.physical = Some(fresh);
        self.changed_rows.clear();
        self.draft_debounce = None;
        Ok(())
    }
    /// Releases the rows of a published chat; false when anything is unwritten.
    pub(super) fn evict_clean(&mut self, retention: &RetentionAuthority) -> bool {
        self.clean() && !self.draft_changed() && self.replace_with_empty(retention).is_ok()
    }
    /// Releases a disabled agent's chat unless its latest input is still unwritten.
    pub(super) fn release_disabled(&mut self, retention: &RetentionAuthority) -> bool {
        (self.clean() || self.published_revision() == self.desired_revision())
            && self.replace_with_empty(retention).is_ok()
    }
    /// Releases rows whose final capture the runtime already owns.
    pub(super) fn release_captured(&mut self, retention: &RetentionAuthority) -> bool {
        self.replace_with_empty(retention).is_ok()
    }
    pub(super) fn set_draft(&mut self, text: &str, now: u64) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let applied = live.set_draft_at(live.local_admission(), Some(text), now)?;
        if applied.saved_changed() {
            self.changed(true);
        }
        Ok(applied.saved_changed())
    }
    pub(super) fn rename(&mut self, name: Option<&str>, now: u64) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let applied = live.rename_at(live.local_admission(), name, now)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(applied.saved_changed())
    }
    pub(super) fn pin(&mut self, pinned: bool, now: u64) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let applied = live.pin_at(live.local_admission(), pinned, now)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(applied.saved_changed())
    }
    /// Stages, natively serializes and only then commits the first queued prompt.
    /// A refusal before the commit leaves the queue, rows and revision untouched.
    pub(super) fn dispatch(
        &mut self,
        request: DispatchRequest<'_>,
        now: u64,
        commands: &nocterm_ai::connection::ManagedCommandsHandle,
    ) -> Result<
        (
            nocterm_ai::connection::ManagedPreparedPrompt,
            history::dispatch::ActiveDispatch,
        ),
        DocumentError,
    > {
        let live = self.physical_mut()?.live_mut();
        let framing = LiveCopies::new(live)?.framing(
            request.session,
            request.context,
            request.history,
            request.command,
        )?;
        let admission = live.local_admission();
        let staged = live.stage_dispatch(admission, framing, now)?;
        let native = commands.prepare_prompt(staged.payload())?;
        let active = staged.commit()?;
        self.changed(false);
        Ok((native, active))
    }
    pub(super) fn queue_text(
        &mut self,
        next_id: u64,
        text: &str,
        attachments: &[history::live::inputs::SavedAttachmentRef<'_>],
        replace: Option<u64>,
    ) -> Result<bool, DocumentError> {
        let live = self.physical_mut()?.live_mut();
        let (applied, replaced) =
            live.queue_text(live.local_admission(), next_id, text, attachments, replace)?;
        if applied.saved_changed() {
            self.changed(false);
        }
        Ok(replaced)
    }
    pub(super) fn prepare_candidate(&mut self) -> Result<Option<u64>, DocumentError> {
        if self.clean() {
            return Ok(None);
        }
        if !self.history_present()
            && self.snapshot().is_some_and(|snapshot| {
                let composer = snapshot.composer();
                !snapshot.metadata().pinned()
                    && snapshot.metadata().name().is_none()
                    && snapshot.rows().is_empty()
                    && composer.draft().is_none_or(|draft| draft.is_empty())
                    && composer.queue().is_empty()
                    && composer.attachments().is_empty()
                    && snapshot
                        .live_inputs()
                        .is_none_or(|inputs| inputs.default_attachments().is_empty())
            })
        {
            return Ok(None);
        }
        if self.persistence.has_current_candidate() {
            return Ok(self.persistence.capture_pending());
        }
        self.physical_mut()?.prepare_candidate()?;
        self.persistence.prepare();
        Ok(self.persistence.capture_pending())
    }
    pub(super) fn offer(
        &self,
        intent: ManagedWriteIntent,
    ) -> Result<CandidateOffer<'_>, DocumentError> {
        if !self.persistence.has_current_candidate() {
            return Err(DocumentError::Retired);
        }
        self.physical
            .as_ref()
            .ok_or(DocumentError::Retired)?
            .candidate(intent)
            .map_err(DocumentError::from)
    }
    #[cfg(test)]
    pub(super) fn existing_offer(&self) -> Option<u64> {
        self.persistence.capture_pending()
    }
    #[cfg(test)]
    pub(super) fn needs_candidate(&self) -> bool {
        !self.clean() && !self.persistence.has_current_candidate()
    }
    pub(super) fn mark_normal_offer(&mut self, revision: u64) -> bool {
        self.persistence.mark_normal_offer(revision)
    }
    pub(super) fn acknowledge_admission(&mut self, revision: u64) {
        self.persistence.acknowledge_admission(revision);
        if revision == self.desired_revision() {
            self.draft_debounce = None;
        }
    }
    pub(super) fn published(&mut self, revision: u64) -> bool {
        self.persistence.published(revision)
    }
    pub(super) fn failed(
        &mut self,
        revision: u64,
        error: crate::runtime::AgentPersistenceError,
    ) -> bool {
        self.persistence.failed(revision, error)
    }
    pub(crate) fn take_changed_rows(&mut self) -> HashSet<usize> {
        std::mem::take(&mut self.changed_rows)
    }
    pub(super) fn mark_row(&mut self, row: usize) {
        self.changed_rows.insert(row);
    }
    pub(super) fn has_debounce(&self) -> bool {
        self.draft_debounce.is_some()
    }
    pub(super) fn replace_debounce(&mut self, task: Task<()>) {
        self.draft_debounce = Some(task);
    }
    pub(super) fn cancel_debounce(&mut self) {
        self.draft_debounce = None;
    }
    /// The caller preflights replacement creation before moving these originals.
    pub(super) fn take_restart(&mut self) -> Result<DocumentRestart, DocumentError> {
        let physical = self.physical.take().ok_or(DocumentError::Retired)?;
        self.draft_debounce = None;
        self.changed_rows.clear();
        Ok(DocumentRestart {
            physical,
            persistence: std::mem::take(&mut self.persistence),
        })
    }
}

#[cfg(test)]
mod tests;
