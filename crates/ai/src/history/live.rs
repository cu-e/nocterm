//! Live document owner: the sole mutation entry over the actual Live role.
//!
//! Owns: the Live and PromptDispatch retained roles, the Foreground
//! construction role, the accepted session epoch and the live revision.
//! Does not own: session/connection lookup, persistence scheduling,
//! plan/metadata/usage controls or any native I/O. Callers supply the original
//! authority they already validated; this owner only re-checks it.
//!
//! Every transition checks scope, epoch and revision before allocating, then
//! publishes exactly one replacement snapshot. A failed or unchanged transition
//! leaves the previous graph and revision untouched.
use super::native::entries;
use crate::{
    history::physical::{
        ChangedRows, ComposerSlot, ManagedInstallError, ManagedParts, PhysicalError, RowSlot,
        SnapshotBuilder,
    },
    model::{
        content::Content,
        ids::SessionId,
        native::NativeOwned,
        notice::SessionUpdate,
        retention::{
            RetentionError,
            document::{ConstructionRole, RetainedRole},
        },
        tools::ClientDisplay,
    },
    session::Ticket,
    thread::{Entry, ThreadChange},
};

mod application;
mod control_application;
mod copies;
pub(crate) mod creation;
mod dispatch;
mod error;
mod fork;
mod input_mutation;
pub mod inputs;
pub use copies::LiveCopies;
pub use creation::{AttachmentEdit, LiveAttachmentRef, LocalAttachmentFact};
pub use dispatch::PreparedDispatch;
pub use error::LiveError;
pub use fork::{ForkFacts, LiveFork};
pub(crate) mod search;

/// Original session authority captured by the caller; never manufactured here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveEpoch {
    ticket: Ticket,
    serial: u64,
    session: SessionId,
}
impl LiveEpoch {
    pub fn new(ticket: Ticket, serial: u64, session: SessionId) -> Self {
        Self {
            ticket,
            serial,
            session,
        }
    }
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }
    pub fn serial(&self) -> u64 {
        self.serial
    }
    pub fn session(&self) -> &SessionId {
        &self.session
    }
}

/// A checked epoch plus the Live revision the caller observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAdmission {
    epoch: Option<LiveEpoch>,
    revision: u64,
}
impl LiveAdmission {
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Result of one admitted transcript transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppliedUpdate {
    change: ThreadChange,
    saved_changed: bool,
    revision: u64,
}
impl AppliedUpdate {
    pub fn change(&self) -> ThreadChange {
        self.change
    }
    /// Whether the persisted transcript window changed. A presentation-only
    /// update still advances the live revision and invalidates old admissions.
    pub fn saved_changed(&self) -> bool {
        self.saved_changed
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Transcript updates are applied; other notices go back to their owners.
#[must_use = "forwarded notices still own their payload"]
#[expect(
    clippy::large_enum_variant,
    reason = "a Box would be an unpaid allocation"
)]
pub enum LiveRouted {
    Applied(AppliedUpdate),
    Forward(NativeOwned<SessionUpdate>),
}

pub struct LiveOwner {
    live: RetainedRole,
    foreground: ConstructionRole,
    dispatch: crate::history::dispatch::DispatchProducer,
    epoch: Option<LiveEpoch>,
    revision: u64,
}
struct ForegroundMutation<'a>(&'a mut ConstructionRole);
impl Drop for ForegroundMutation<'_> {
    fn drop(&mut self) {
        self.0.clear_source();
    }
}
impl LiveOwner {
    pub fn mentions(&self, query: &str) -> Result<bool, LiveError> {
        let Some(snapshot) = self.live.snapshot() else {
            return Ok(false);
        };
        search::matches(&self.foreground, snapshot, query)
            .map_err(|error| PhysicalError::from(error).into())
    }
    /// Install framing once from original supplied native identity/time facts.
    /// Callers complete this fallible work before creating a visible entity.
    pub fn initialize(&mut self, agent: &str, id: &str, updated: u64) -> Result<(), LiveError> {
        if self.live.snapshot().is_some() {
            return Err(LiveError::Role);
        }
        self.foreground.ensure_live_phase()?;
        let document = creation::DocumentSeed::new(agent, id, updated).prepare(&self.foreground)?;
        let staged = SnapshotBuilder::from_document(&self.foreground, document)?.finish(None)?;
        crate::history::decision_managed(staged.snapshot())?;
        self.live.replace(staged)?;
        Ok(())
    }
    pub fn new(
        live: RetainedRole,
        foreground: ConstructionRole,
        dispatch: RetainedRole,
    ) -> Result<Self, LiveError> {
        if !live.is_live()
            || !dispatch.is_prompt_dispatch()
            || !foreground.is_foreground()
            || !live.scope().same_document(foreground.scope())
            || !live.scope().same_document(dispatch.scope())
        {
            return Err(LiveError::Role);
        }
        Ok(Self {
            live,
            foreground,
            dispatch: crate::history::dispatch::DispatchProducer::new(dispatch)?,
            epoch: None,
            revision: 0,
        })
    }
    pub fn role(&self) -> &RetainedRole {
        &self.live
    }
    pub fn dispatch(&self) -> &crate::history::dispatch::DispatchProducer {
        &self.dispatch
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn rows(&self) -> &[RowSlot] {
        self.live.snapshot().map_or(&[], |owner| owner.rows())
    }
    pub fn times(&self) -> &[u64] {
        self.live.snapshot().map_or(&[], |owner| owner.times())
    }
    pub fn composer(&self) -> Option<&ComposerSlot> {
        self.live.snapshot().map(|owner| owner.composer())
    }
    pub fn inputs(&self) -> Option<&crate::history::physical::LiveInputs> {
        self.live.snapshot().and_then(|owner| owner.live_inputs())
    }
    pub fn prepare_reader_id(&mut self) -> Result<crate::history::ReaderDocumentId, LiveError> {
        self.foreground.ensure_live_phase()?;
        let snapshot = self.live.snapshot().ok_or(LiveError::MissingPayload)?;
        creation::ReaderIdentity::new(&snapshot.id())
            .prepare(&self.foreground)
            .map_err(crate::history::HistoryCodecError::from)
            .map_err(LiveError::from)
    }
    pub fn prepare_image_request(
        &mut self,
        bytes: &[u8],
    ) -> Result<
        crate::images::managed::ManagedImageRequest,
        crate::images::managed::ImageOperationError,
    > {
        self.foreground
            .ensure_live_phase()
            .map_err(crate::model::json::deferred::JsonDecodeError::Retention)?;
        crate::images::managed::ManagedImageRequest::prepare(&self.foreground, bytes)
    }
    pub fn controls(&self) -> Option<&crate::history::physical::LiveControls> {
        self.live
            .snapshot()
            .and_then(|owner| owner.presentation())
            .and_then(|presentation| presentation.controls())
    }
    pub fn install_opened(
        &mut self,
        admission: LiveAdmission,
        opened: crate::history::source::ManagedOpened,
    ) -> Result<AppliedUpdate, LiveError> {
        control_application::ControlApplication::new(self, admission)?.opened(opened)
    }
    pub fn install_info(
        &mut self,
        admission: LiveAdmission,
        info: crate::history::source::ManagedAgentInfo,
    ) -> Result<AppliedUpdate, LiveError> {
        control_application::ControlApplication::new(self, admission)?.info(info)
    }
    pub fn finish_turn(
        &mut self,
        tokens: Option<crate::model::usage::TokenTotals>,
    ) -> Result<(), LiveError> {
        self.transition(|builder| {
            builder
                .update_presentation(crate::history::physical::PresentationUpdate::Tokens(tokens))
                .map(|changed| ((), changed))
        })?;
        Ok(())
    }
    pub fn usage(&self) -> Option<&crate::history::physical::LiveUsage> {
        self.live
            .snapshot()
            .and_then(|owner| owner.presentation())
            .and_then(|presentation| presentation.usage())
    }
    pub fn plan(
        &self,
    ) -> Option<crate::history::physical::PayloadGuard<'_, Vec<crate::model::notice::PlanStep>>>
    {
        self.live
            .snapshot()
            .and_then(|owner| owner.presentation())
            .and_then(|presentation| presentation.plan())
    }
    /// Replaces the accepted epoch; admissions of the previous one go stale.
    pub fn accept_epoch(&mut self, original: LiveEpoch) {
        self.epoch = Some(original);
    }
    pub fn admission(&self, original: &LiveEpoch) -> Result<LiveAdmission, LiveError> {
        match &self.epoch {
            Some(epoch) if epoch == original => Ok(LiveAdmission {
                epoch: Some(original.clone()),
                revision: self.revision,
            }),
            _ => Err(LiveError::Stale),
        }
    }
    fn check(&self, admission: &LiveAdmission) -> Result<(), LiveError> {
        if self.epoch != admission.epoch || self.revision != admission.revision {
            return Err(LiveError::Stale);
        }
        if self.live.snapshot().is_none() {
            return Err(LiveError::MissingPayload);
        }
        Ok(())
    }
    /// Local document edits also exist before a native session has opened.
    /// This captures only this owner's current epoch/revision, never a Ticket.
    pub fn local_admission(&self) -> LiveAdmission {
        LiveAdmission {
            epoch: self.epoch.clone(),
            revision: self.revision,
        }
    }
    /// Runs one builder transition and publishes it only when it changed.
    fn transition<R>(
        &mut self,
        edit: impl FnOnce(&mut SnapshotBuilder<'_>) -> Result<(R, bool), PhysicalError>,
    ) -> Result<(R, bool), LiveError> {
        self.transition_certified(None, edit)
    }
    fn transition_certified<R>(
        &mut self,
        policy: Option<&mut crate::model::retention::document::PolicyLiability>,
        edit: impl FnOnce(&mut SnapshotBuilder<'_>) -> Result<(R, bool), PhysicalError>,
    ) -> Result<(R, bool), LiveError> {
        let foreground = ForegroundMutation(&mut self.foreground);
        let result = (|| {
            let mut builder = SnapshotBuilder::from_role(foreground.0, &self.live)?;
            let (value, changed) = edit(&mut builder)?;
            if changed {
                let staged = builder.finish(None)?;
                crate::history::decision_managed(staged.snapshot())?;
                if let Some(policy) = policy {
                    self.live.replace_certified(staged, policy)?;
                } else {
                    self.live.replace(staged)?;
                }
            }
            Ok::<_, LiveError>((value, changed))
        })();
        drop(foreground);
        let (value, changed) = result?;
        if changed {
            self.revision += 1;
        }
        Ok((value, changed))
    }
    fn applied(&self, saved_changed: bool) -> AppliedUpdate {
        AppliedUpdate {
            change: ThreadChange::Transcript,
            saved_changed,
            revision: self.revision,
        }
    }
    /// Consume the original source certificate in the same atomic publication
    /// which installs the complete new live policy and physical graph.
    pub fn apply_certified(
        &mut self,
        admission: LiveAdmission,
        mut notice: crate::history::source::ManagedNotice,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let epoch = admission.epoch.as_ref().ok_or(LiveError::Stale)?;
        if !notice.belongs_to(self.live.scope()) || notice.notice().session != epoch.session {
            return Err(LiveError::Role);
        }
        let (update, policy) = notice.take_update();
        match application::NoticeApplication::new(self, update, Some(policy), now).run()? {
            LiveRouted::Applied(value) => Ok(value),
            LiveRouted::Forward(value) => {
                drop(value);
                Err(LiveError::Role)
            }
        }
    }
    pub fn apply_owned(
        &mut self,
        admission: LiveAdmission,
        update: NativeOwned<SessionUpdate>,
        now: u64,
    ) -> Result<LiveRouted, LiveError> {
        self.check(&admission)?;
        if admission.epoch.is_none() {
            return Err(LiveError::Stale);
        }
        application::NoticeApplication::new(self, update, None, now).run()
    }
    pub fn append_user(
        &mut self,
        admission: LiveAdmission,
        content: NativeOwned<Vec<Content>>,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let row = entries::user(content);
        self.transition(|builder| builder.append_row(row, now).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    pub fn truncate(
        &mut self,
        admission: LiveAdmission,
        len: usize,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let (_, changed) =
            self.transition(|b| b.truncate_rows(len).map(|changed| ((), changed)))?;
        Ok(self.applied(changed))
    }
    /// Replaces only the draft bundle; queued media remain aliased in place.
    pub fn replace_draft(
        &mut self,
        admission: LiveAdmission,
        draft: NativeOwned<Option<String>>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        if !draft.ledger().belongs_to_document(self.live.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let current = self.composer().and_then(ComposerSlot::draft);
        if current.as_deref() == draft.get().as_deref() {
            return Ok(self.applied(false));
        }
        drop(current);
        self.transition(|builder| builder.replace_draft(draft).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    /// Copies editor text into the original Foreground phase before publishing.
    /// Empty text clears the draft, matching the composer's existing rule.
    /// No caller-created String or unscoped ledger is adopted.
    pub fn set_draft(
        &mut self,
        admission: LiveAdmission,
        text: Option<&str>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(
            admission,
            text.filter(|text| !text.is_empty()),
            creation::TextTarget::Draft,
            None,
        )
    }
    pub fn set_draft_at(
        &mut self,
        admission: LiveAdmission,
        text: Option<&str>,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(
            admission,
            text.filter(|text| !text.is_empty()),
            creation::TextTarget::Draft,
            Some(now),
        )
    }
    pub fn set_title(
        &mut self,
        admission: LiveAdmission,
        title: Option<&str>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(admission, title, creation::TextTarget::Title, None)
    }
    pub fn rename(
        &mut self,
        admission: LiveAdmission,
        name: Option<&str>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(admission, name, creation::TextTarget::Name, None)
    }
    pub fn rename_at(
        &mut self,
        admission: LiveAdmission,
        name: Option<&str>,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(
            admission,
            name.map(str::trim).filter(|name| !name.is_empty()),
            creation::TextTarget::Name,
            Some(now),
        )
    }
    pub fn set_resume_session(
        &mut self,
        admission: LiveAdmission,
        session: Option<&str>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(admission, session, creation::TextTarget::Session, None)
    }
    pub fn set_resume_mode(
        &mut self,
        admission: LiveAdmission,
        mode: Option<&str>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.edit_text(admission, mode, creation::TextTarget::Mode, None)
    }
    pub fn replace_resume_workdir(
        &mut self,
        admission: LiveAdmission,
        value: NativeOwned<Option<std::path::PathBuf>>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        if !value.ledger().belongs_to_document(self.live.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        if self
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .resume()
            .workdir()
            .as_deref()
            == value.get().as_deref()
        {
            return Ok(self.applied(false));
        }
        self.foreground.ensure_live_phase()?;
        self.transition(|builder| builder.replace_workdir(value).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    pub fn replace_resume_choice(
        &mut self,
        admission: LiveAdmission,
        key: NativeOwned<String>,
        value: NativeOwned<crate::session_config::ConfigValue>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        if !key.ledger().belongs_to_document(self.live.scope())
            || !value.ledger().belongs_to_document(self.live.scope())
        {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let choices = self
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .resume()
            .config();
        if choices
            .binary_search_by(|choice| (*choice.key()).cmp(key.as_str()))
            .is_ok_and(|index| choices[index].value().matches(value.get()))
        {
            return Ok(self.applied(false));
        }
        self.foreground.ensure_live_phase()?;
        self.transition(|builder| builder.replace_choice(key, value).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    pub fn set_pending_history(
        &mut self,
        admission: LiveAdmission,
        pending: bool,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        if self
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .resume()
            .pending_history()
            == pending
        {
            return Ok(self.applied(false));
        }
        self.foreground.ensure_live_phase()?;
        self.transition(|builder| builder.set_pending_history(pending).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    fn edit_text(
        &mut self,
        admission: LiveAdmission,
        text: Option<&str>,
        target: creation::TextTarget,
        updated: Option<u64>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let snapshot = self.live.snapshot().ok_or(LiveError::MissingPayload)?;
        let current = target.current(snapshot);
        if current.as_deref() == text {
            return Ok(self.applied(false));
        }
        let pinned = snapshot.metadata().pinned();
        drop(current);
        self.foreground.ensure_live_phase()?;
        let edit =
            creation::LocalTextEdit::prepare(&self.foreground, text, target, updated, pinned)?;
        self.transition(|builder| edit.apply(builder).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    pub fn pin_at(
        &mut self,
        admission: LiveAdmission,
        pinned: bool,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        if self
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .metadata()
            .pinned()
            == pinned
        {
            return Ok(self.applied(false));
        }
        self.stamp_metadata(admission, now, pinned)
    }
    pub fn stamp_metadata(
        &mut self,
        admission: LiveAdmission,
        updated: u64,
        pinned: bool,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let metadata = self
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .metadata();
        if metadata.updated() == updated && metadata.pinned() == pinned {
            return Ok(self.applied(false));
        }
        self.foreground.ensure_live_phase()?;
        self.transition(|builder| builder.stamp_metadata(updated, pinned).map(|()| ((), true)))?;
        Ok(self.applied(true))
    }
    /// Publishes aliases of the real persisted components into the supplied
    /// original Candidate role. Volatile inputs stay owned only by Live.
    pub fn prepare_saved_candidate(
        &mut self,
        admission: LiveAdmission,
        candidate: &mut RetainedRole,
    ) -> Result<(), LiveError> {
        self.check(&admission)?;
        if !candidate.is_candidate() || !self.live.scope().same_document(candidate.scope()) {
            return Err(LiveError::Role);
        }
        self.foreground.ensure_live_phase()?;
        let foreground = ForegroundMutation(&mut self.foreground);
        let result = (|| {
            let mut builder = SnapshotBuilder::from_role(foreground.0, &self.live)?;
            builder.prepare_saved_candidate();
            let staged = builder.finish(None)?;
            crate::history::decision_managed(staged.snapshot())?;
            candidate.replace(staged)?;
            Ok::<_, LiveError>(())
        })();
        drop(foreground);
        result
    }
    pub fn patch_display(
        &mut self,
        admission: LiveAdmission,
        row: usize,
        display: NativeOwned<ClientDisplay>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let in_saved_window = row >= self.saved_start();
        let (_, changed) =
            self.transition(|b| b.patch_display(row, display).map(|changed| ((), changed)))?;
        Ok(self.applied(changed && in_saved_window))
    }
    /// Replaces volatile context without marking persisted fields dirty.
    pub fn edit_inputs(
        &mut self,
        admission: LiveAdmission,
        edit: crate::history::physical::ComposerInputEdit,
    ) -> Result<AppliedUpdate, LiveError> {
        input_mutation::InputMutation::new(self, admission)?.edit(edit)
    }
    pub fn replace_attachments(
        &mut self,
        admission: LiveAdmission,
        attachments: NativeOwned<Vec<inputs::LiveAttachment>>,
    ) -> Result<AppliedUpdate, LiveError> {
        input_mutation::InputMutation::new(self, admission)?.replace_attachments(attachments)
    }
    pub fn edit_attachments(
        &mut self,
        admission: LiveAdmission,
        edit: AttachmentEdit<'_>,
        locals: &[LocalAttachmentFact<'_>],
        updated: u64,
    ) -> Result<(AppliedUpdate, bool), LiveError> {
        input_mutation::InputMutation::new(self, admission)?.attachments(edit, locals, updated)
    }
    pub fn replace_images(
        &mut self,
        admission: LiveAdmission,
        images: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
    ) -> Result<AppliedUpdate, LiveError> {
        input_mutation::InputMutation::new(self, admission)?.replace_images(images)
    }
    pub fn add_prepared_image(
        &mut self,
        admission: LiveAdmission,
        image: NativeOwned<crate::images::managed::PreparedPromptImage>,
    ) -> Result<AppliedUpdate, LiveError> {
        input_mutation::InputMutation::new(self, admission)?.add_image(image)
    }
    /// Removes all occurrences of the original prompt id, preserving survivors.
    /// Commit visible User aliases and queue pop with the original dispatch root.
    pub fn stage_dispatch(
        &mut self,
        admission: LiveAdmission,
        framing: crate::history::dispatch::DispatchFraming,
        now: u64,
    ) -> Result<PreparedDispatch<'_>, LiveError> {
        PreparedDispatch::prepare(self, admission, framing, now)
    }
    pub fn prepare_dispatch(
        &mut self,
        admission: LiveAdmission,
        framing: crate::history::dispatch::DispatchFraming,
        now: u64,
    ) -> Result<crate::history::dispatch::ActiveDispatch, LiveError> {
        self.stage_dispatch(admission, framing, now)?.commit()
    }
    pub fn queue_text(
        &mut self,
        admission: LiveAdmission,
        next_id: u64,
        text: &str,
        attachments: &[inputs::SavedAttachmentRef<'_>],
        replace: Option<u64>,
    ) -> Result<(AppliedUpdate, bool), LiveError> {
        self.check(&admission)?;
        let (replaced, _) = self.transition(|builder| {
            builder
                .queue_text(next_id, text, attachments, replace)
                .map(|replaced| (replaced, true))
        })?;
        Ok((self.applied(true), replaced))
    }
    /// Queue a prompt using the original current input owners. Refusal leaves
    /// draft, images, queue identities and revision unchanged.
    pub fn submit_prompt(
        &mut self,
        admission: LiveAdmission,
        next_id: u64,
        text: NativeOwned<String>,
        attachments: NativeOwned<Vec<crate::history::SavedAttachment>>,
        replace: Option<u64>,
    ) -> Result<(AppliedUpdate, bool), LiveError> {
        self.check(&admission)?;
        let (replaced, _) = self.transition(|builder| {
            builder
                .submit_prompt(next_id, text, attachments, replace)
                .map(|replaced| (replaced, true))
        })?;
        Ok((self.applied(true), replaced))
    }
    pub fn remove_prompt(
        &mut self,
        admission: LiveAdmission,
        id: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let (_, changed) = self.transition(|b| b.remove_prompt(id).map(|changed| ((), changed)))?;
        Ok(self.applied(changed))
    }
    /// Changes queue order without copying any prompt payload.
    pub fn move_prompt(
        &mut self,
        admission: LiveAdmission,
        id: u64,
        target: usize,
    ) -> Result<AppliedUpdate, LiveError> {
        self.check(&admission)?;
        let (_, changed) =
            self.transition(|b| b.move_prompt(id, target).map(|changed| ((), changed)))?;
        Ok(self.applied(changed))
    }
    fn saved_start(&self) -> usize {
        self.rows()
            .len()
            .saturating_sub(crate::history::MAX_SAVED_ENTRIES)
    }
    pub fn finish_pending_tools(
        &mut self,
        admission: LiveAdmission,
    ) -> Result<ChangedRows, LiveError> {
        self.check(&admission)?;
        let (rows, _) = self.transition(|b| {
            let rows = b.finish_pending_tools()?;
            let changed = !rows.as_slice().is_empty();
            Ok((rows, changed))
        })?;
        Ok(rows)
    }
    /// Installs history and recycles the original reader/delivery capabilities.
    #[expect(
        clippy::result_large_err,
        reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
    )]
    pub fn install_delivery(
        &mut self,
        delivery: crate::history::ReaderDelivery,
    ) -> Result<(), crate::history::ReaderInstallError> {
        delivery.install_into(&mut self.live)?;
        self.revision += 1;
        Ok(())
    }
    /// Installs managed parts when the caller owns capability recycling.
    pub fn install(&mut self, parts: ManagedParts) -> Result<RetainedRole, ManagedInstallError> {
        let delivery = parts.install_into(&mut self.live)?;
        self.revision += 1;
        Ok(delivery)
    }
}

#[cfg(test)]
mod tests;
