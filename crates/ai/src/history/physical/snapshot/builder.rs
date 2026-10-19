//! Checked construction owns the actual mutable component aliases until publication.
use super::*;

pub struct SnapshotBuilder<'a> {
    parts: SnapshotParts,
    inherited: Option<PreparedHistory>,
    slab: SlabBuilder<'a>,
    scope: DocumentRetention,
}
impl<'a> SnapshotBuilder<'a> {
    pub fn replace_input_scope(
        &mut self,
        live: NativeOwned<Vec<crate::history::live::inputs::LiveAttachment>>,
        saved: Option<NativeOwned<Vec<crate::history::SavedAttachment>>>,
    ) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .replace_input_scope(self.slab.role(), live, saved)
    }
    pub fn add_prepared_image(
        &mut self,
        image: NativeOwned<crate::images::managed::PreparedPromptImage>,
    ) -> Result<(), PhysicalError> {
        self.parts
            .composition
            .add_prepared_image(self.slab.role(), image)
    }
    pub fn from_native(
        role: &'a ConstructionRole,
        id: NativeOwned<String>,
        metadata: NativeOwned<ChatMetadata>,
        resume: NativeOwned<ResumeDescriptor>,
        transcript: NativeOwned<TranscriptData>,
        composer: NativeOwned<ComposerData>,
    ) -> Result<Self, PhysicalError> {
        if !id.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let joined = id
            .join(metadata)?
            .join(resume)?
            .join(transcript)?
            .join(composer)?;
        // Header preallocation happens while the complete native owner still
        // guards payload-before-credit cancellation. Extraction afterward is
        // infallible and immediately installs the authentic shared credit owner.
        let header = NativeCredit::reserve_header(role)?;
        let (value, allocation) = joined.into_parts();
        let source = ControlledSource {
            value: Some(value),
            pool: NativeSourcePool::new(role, allocation, header),
        };
        Self::convert(role, source)
    }
    pub fn from_document(
        role: &'a ConstructionRole,
        document: NativeOwned<crate::history::SavedDocument>,
    ) -> Result<Self, PhysicalError> {
        if !document.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let header = NativeCredit::reserve_header(role)?;
        let (document, allocation) = document.into_parts();
        let crate::history::SavedDocument {
            id,
            metadata,
            resume,
            transcript,
            composer,
        } = document;
        Self::convert(
            role,
            ControlledSource {
                value: Some(((((id, metadata), resume), transcript), composer)),
                pool: NativeSourcePool::new(role, allocation, header),
            },
        )
    }
    fn convert(
        role: &'a ConstructionRole,
        mut source: ControlledSource,
    ) -> Result<Self, PhysicalError> {
        let original = source.pool.original().clone();
        let ((((id, metadata), resume), transcript), composer) =
            source.value.take().expect("fresh native source");
        let mut rows_builder = SlabBuilder::new(role)?;
        let rows = rows_builder.map_in(
            transcript.entries,
            &mut source.pool,
            |value, builder, pool| RowSlot::new(value, builder, &original, pool),
        )?;
        let transcript = BundleSlot::new(BundleOwner::build(
            TranscriptSlot::new(rows, transcript.times, &mut source.pool)?,
            rows_builder,
            &original,
        )?);
        let mut resume_builder = SlabBuilder::new(role)?;
        let resume = ResumeSlot::new(resume, &mut resume_builder, &mut source.pool)?;
        let resume = BundleSlot::new(BundleOwner::build(resume, resume_builder, &original)?);
        let mut composer_builder = SlabBuilder::new(role)?;
        let composer =
            ComposerSlot::new(composer, &mut composer_builder, &original, &mut source.pool)?;
        let composer = BundleSlot::new(BundleOwner::build(composer, composer_builder, &original)?);
        let id = bundle::part(role, &original, TextSlot::new(id, &mut source.pool)?)?;
        let metadata = bundle::part(
            role,
            &original,
            MetadataSlot::new(metadata, role, &original, &mut source.pool)?,
        )?;
        Ok(Self {
            parts: SnapshotParts {
                id,
                metadata,
                resume,
                transcript,
                composition: ComposerGraph::new(composer),
            },
            inherited: None,
            slab: SlabBuilder::new(role)?,
            scope: role.scope().clone(),
        })
    }
    /// Retain the genuine old source in the operation before preparing aliases.
    /// No transcript/media copy is needed for a metadata/composer-only edit.
    pub fn from_role(
        role: &'a mut ConstructionRole,
        source: &RetainedRole,
    ) -> Result<Self, PhysicalError> {
        role.retain_source(source)?;
        let owner = source.snapshot().ok_or(RetentionError::FrozenOwner)?;
        Ok(Self {
            parts: owner.life.slab.parts.alias(),
            inherited: owner.prepared().clone(),
            slab: SlabBuilder::new(role)?,
            scope: role.scope().clone(),
        })
    }
    pub fn from_fork_role(
        role: &'a mut ConstructionRole,
        source: &RetainedRole,
    ) -> Result<Self, PhysicalError> {
        role.retain_fork_source(source)?;
        let owner = source.snapshot().ok_or(RetentionError::FrozenOwner)?;
        Ok(Self {
            parts: owner.life.slab.parts.alias(),
            inherited: owner.prepared().clone(),
            slab: SlabBuilder::new(role)?,
            scope: role.scope().clone(),
        })
    }
    /// Full live rows are retained. The2000-record boundary belongs only to the
    /// encoder's borrowed saved window, never this mutation or physical slab.
    pub fn append_row(
        &mut self,
        row: NativeOwned<crate::thread::Entry>,
        now: u64,
    ) -> Result<(), PhysicalError> {
        let next = self.parts.transcript.body().prepare_append(
            self.slab.role(),
            row,
            now,
            self.parts.transcript.original_credit(),
        )?;
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(())
    }
    /// Shares one authentic current queued payload into its transcript row.
    pub fn prepare_dispatch(
        &self,
        framing: crate::history::dispatch::DispatchFraming,
    ) -> Result<super::super::DispatchStaging<'a>, PhysicalError> {
        let prompt = self
            .parts
            .composition
            .composer()
            .queue()
            .first()
            .ok_or(PhysicalError::QueuedPromptMissing)?;
        super::super::DispatchPayload::prepare(
            self.slab.role(),
            prompt,
            self.parts.composition.inputs(),
            framing,
        )
    }
    pub fn append_prompt_user(&mut self, index: usize, now: u64) -> Result<bool, PhysicalError> {
        let Some(prompt) = self.parts.composition.composer().queue().get(index) else {
            return Ok(false);
        };
        let next = self.parts.transcript.body().prepare_append_prompt(
            self.slab.role(),
            prompt,
            now,
            self.parts.transcript.original_credit(),
        )?;
        drop(std::mem::replace(&mut self.parts.transcript, next));
        Ok(true)
    }
    pub fn replace_row(
        &mut self,
        index: usize,
        row: NativeOwned<crate::thread::Entry>,
    ) -> Result<bool, PhysicalError> {
        let next = self.parts.transcript.body().prepare_replace(
            self.slab.role(),
            index,
            row,
            self.parts.transcript.original_credit(),
        )?;
        let Some(next) = next else {
            return Ok(false);
        };
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(true)
    }
    /// Applies a live tool patch to the first row with its id, as the reducer does. A status-only
    /// patch shares every payload bundle with the previous row version.
    pub fn patch_tool(
        &mut self,
        patch: NativeOwned<crate::model::tools::ToolPatch>,
    ) -> Result<bool, PhysicalError> {
        let next = self.parts.transcript.body().prepare_patch_tool(
            self.slab.role(),
            patch,
            self.parts.transcript.original_credit(),
        )?;
        let Some(next) = next else {
            return Ok(false);
        };
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(true)
    }
    /// Installs a client display on the tool row at `index`.
    pub fn patch_display(
        &mut self,
        index: usize,
        display: NativeOwned<crate::model::tools::ClientDisplay>,
    ) -> Result<bool, PhysicalError> {
        let next = self.parts.transcript.body().prepare_patch_display(
            self.slab.role(),
            index,
            display,
            self.parts.transcript.original_credit(),
        )?;
        let Some(next) = next else {
            return Ok(false);
        };
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(true)
    }
    /// Pending/Running tools become Failed in one transcript rebuild.
    pub fn finish_pending_tools(&mut self) -> Result<super::super::ChangedRows, PhysicalError> {
        let next = self.parts.transcript.body().prepare_finish_pending_tools(
            self.slab.role(),
            self.parts.transcript.original_credit(),
        )?;
        let Some((next, changed)) = next else {
            return Ok(super::super::ChangedRows::default());
        };
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(changed)
    }
    /// Appends an agent (`thought == false`) or thought text chunk; only the
    /// changed final row is copied, once, into prepaid exact capacity.
    pub fn append_agent_text(
        &mut self,
        thought: bool,
        chunk: NativeOwned<String>,
        now: u64,
    ) -> Result<bool, PhysicalError> {
        let next = self.parts.transcript.body().prepare_append_text(
            self.slab.role(),
            thought,
            chunk,
            now,
            self.parts.transcript.original_credit(),
        )?;
        let Some(next) = next else {
            return Ok(false);
        };
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(true)
    }
    /// Appends a user content chunk; earlier items of a final User row alias.
    pub fn append_user_content(
        &mut self,
        content: NativeOwned<crate::model::content::Content>,
        now: u64,
    ) -> Result<(), PhysicalError> {
        let next = self.parts.transcript.body().prepare_append_user(
            self.slab.role(),
            content,
            now,
            self.parts.transcript.original_credit(),
        )?;
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(())
    }
    pub fn truncate_rows(&mut self, len: usize) -> Result<bool, PhysicalError> {
        let next = self.parts.transcript.body().prepare_truncate(
            self.slab.role(),
            len,
            self.parts.transcript.original_credit(),
        )?;
        let Some(next) = next else {
            return Ok(false);
        };
        let old = std::mem::replace(&mut self.parts.transcript, next);
        drop(old);
        Ok(true)
    }
    pub fn replace_live_attachments(
        &mut self,
        value: NativeOwned<Vec<crate::history::live::inputs::LiveAttachment>>,
    ) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .replace_live_attachments(self.slab.role(), value)
    }
    pub fn edit_inputs(
        &mut self,
        edit: super::super::ComposerInputEdit,
    ) -> Result<bool, PhysicalError> {
        self.parts.composition.edit_inputs(self.slab.role(), edit)
    }
    pub fn replace_images(
        &mut self,
        value: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
    ) -> Result<(), PhysicalError> {
        self.parts
            .composition
            .replace_images(self.slab.role(), value)
    }
    pub fn prepare_saved_candidate(&mut self) {
        self.parts.composition.prepare_saved_candidate();
    }
    pub fn update_presentation(
        &mut self,
        update: super::super::presentation::PresentationUpdate,
    ) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .update_presentation(self.slab.role(), update)
    }
    pub fn replace_usage(
        &mut self,
        value: NativeOwned<crate::model::notice::UsageNotice>,
    ) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .replace_usage(self.slab.role(), value)
    }
    pub fn replace_plan(
        &mut self,
        value: NativeOwned<Vec<crate::model::notice::PlanStep>>,
    ) -> Result<bool, PhysicalError> {
        self.parts.composition.replace_plan(self.slab.role(), value)
    }
    pub fn replace_composer(
        &mut self,
        value: NativeOwned<ComposerData>,
    ) -> Result<(), PhysicalError> {
        self.parts
            .composition
            .replace_composer(self.slab.role(), value)
    }
    pub fn queue_text(
        &mut self,
        id: u64,
        text: &str,
        attachments: &[crate::history::live::inputs::SavedAttachmentRef<'_>],
        replace: Option<u64>,
    ) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .validate_submission(replace, attachments.len())?;
        let (text, attachments) =
            crate::history::live::creation::QueuedText::new(text, attachments)
                .prepare(self.slab.role())?;
        self.submit_prompt(id, text, attachments, replace)
    }
    pub fn submit_prompt(
        &mut self,
        id: u64,
        text: NativeOwned<String>,
        attachments: NativeOwned<Vec<crate::history::SavedAttachment>>,
        replace: Option<u64>,
    ) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .submit_prompt(self.slab.role(), id, text, attachments, replace)
    }
    pub fn replace_draft(
        &mut self,
        value: NativeOwned<Option<String>>,
    ) -> Result<(), PhysicalError> {
        self.parts
            .composition
            .replace_draft(self.slab.role(), value)
    }
    pub fn replace_title(
        &mut self,
        value: NativeOwned<Option<String>>,
    ) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .metadata
            .body()
            .replacing_title(value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.metadata, next));
        Ok(())
    }
    /// A fork keeps its aliased rows but is a different document identity.
    pub fn replace_id(&mut self, value: NativeOwned<String>) -> Result<(), PhysicalError> {
        let mut source = NativeSource::new(value, self.slab.role())?;
        let original = source.original().clone();
        let text = source.take();
        let slot = TextSlot::new(text, source.pool())?;
        let next = bundle::part(self.slab.role(), &original, slot)?;
        drop(std::mem::replace(&mut self.parts.id, next));
        Ok(())
    }
    pub fn rename(&mut self, value: NativeOwned<Option<String>>) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .metadata
            .body()
            .renaming(value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.metadata, next));
        Ok(())
    }
    /// The model of the latest accepted prompt; other metadata text stays shared.
    pub fn replace_model(
        &mut self,
        value: NativeOwned<Option<String>>,
    ) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .metadata
            .body()
            .replacing_model(value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.metadata, next));
        Ok(())
    }
    /// Scalar metadata changes retain every original text bundle.
    pub fn stamp_metadata(&mut self, updated: u64, pinned: bool) -> Result<(), PhysicalError> {
        let next = self.parts.metadata.body().stamped(
            updated,
            pinned,
            self.slab.role(),
            self.parts.metadata.original_credit(),
        )?;
        drop(std::mem::replace(&mut self.parts.metadata, next));
        Ok(())
    }
    pub fn replace_session(
        &mut self,
        value: NativeOwned<Option<String>>,
    ) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .resume
            .body()
            .replacing_session(value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.resume, next));
        Ok(())
    }
    pub fn replace_mode(
        &mut self,
        value: NativeOwned<Option<String>>,
    ) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .resume
            .body()
            .replacing_mode(value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.resume, next));
        Ok(())
    }
    pub fn replace_workdir(
        &mut self,
        value: NativeOwned<Option<std::path::PathBuf>>,
    ) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .resume
            .body()
            .replacing_workdir(value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.resume, next));
        Ok(())
    }
    pub fn replace_choice(
        &mut self,
        key: NativeOwned<String>,
        value: NativeOwned<crate::session_config::ConfigValue>,
    ) -> Result<(), PhysicalError> {
        let next = self
            .parts
            .resume
            .body()
            .replacing_choice(key, value, self.slab.role())?;
        drop(std::mem::replace(&mut self.parts.resume, next));
        Ok(())
    }
    pub fn set_pending_history(&mut self, pending: bool) -> Result<(), PhysicalError> {
        let next = self.parts.resume.body().with_pending_history(
            pending,
            self.slab.role(),
            self.parts.resume.original_credit(),
        )?;
        drop(std::mem::replace(&mut self.parts.resume, next));
        Ok(())
    }
    pub fn select_queue(
        &mut self,
        selection: super::super::composer::QueueSelection,
    ) -> Result<(), PhysicalError> {
        self.parts
            .composition
            .select_queue(self.slab.role(), selection)
    }
    pub fn remove_prompt(&mut self, id: u64) -> Result<bool, PhysicalError> {
        self.parts.composition.remove_prompt(self.slab.role(), id)
    }
    pub fn move_prompt(&mut self, id: u64, target: usize) -> Result<bool, PhysicalError> {
        self.parts
            .composition
            .move_prompt(self.slab.role(), id, target)
    }
    pub fn finish(
        mut self,
        prepared: Option<PreparedHistory>,
    ) -> Result<SnapshotStaging<'a>, PhysicalError> {
        let prepared = prepared.or_else(|| self.inherited.take());
        self.slab.shared::<SnapshotAllocation>()?;
        self.slab.shared::<PayloadLife>()?;
        let prepared_projection = prepared.as_ref().map(PreparedHistory::retention_projection);
        let cell = RetentionCell::new(self.scope.authority());
        let operation = self.slab.role();
        let slab = Shared::new(SnapshotAllocation {
            parts: self.parts,
            prepared: Mutex::new(prepared),
            prepared_projection,
            cell,
            payload_retired: AtomicBool::new(false),
            policy: OnceLock::new(),
            credit: self.slab.finish(),
        });
        let owner = SnapshotOwner {
            life: Shared::new(PayloadLife { slab }),
        };
        let policy = super::super::policy::SnapshotPolicy::prepare(&owner)?;
        if owner.life.slab.policy.set(policy).is_err() {
            return Err(RetentionError::UncertifiedPolicy.into());
        }
        Ok(SnapshotStaging {
            owner,
            scope: self.scope,
            _operation: operation,
        })
    }
}
type SourceParts = (
    (((String, ChatMetadata), ResumeDescriptor), TranscriptData),
    ComposerData,
);
struct ControlledSource {
    value: Option<SourceParts>,
    pool: NativeSourcePool,
}
