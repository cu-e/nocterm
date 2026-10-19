//! The actual composer and volatile graph edges form one construction owner.
use super::{
    bundle::{BundleOwner, BundleSlot},
    composer::ComposerSlot,
    extras::LiveExtras,
    inputs::LiveInputs,
    presentation::LivePresentation,
    slots::*,
};
use crate::{
    history::ComposerData,
    model::{
        native::NativeOwned,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
};

pub(super) struct ComposerGraph {
    persisted: BundleSlot<ComposerSlot>,
    extras: Option<BundleSlot<LiveExtras>>,
}
/// Closed input transitions preserve original prompt/default allocation identities.
pub enum ComposerInputEdit {
    BeginOriginal(u64),
    End,
    Reset,
    Restart,
    RemoveImage(usize),
}
impl ComposerGraph {
    pub(super) fn replace_input_scope(
        &mut self,
        role: &ConstructionRole,
        live: NativeOwned<Vec<crate::history::live::inputs::LiveAttachment>>,
        saved: Option<NativeOwned<Vec<crate::history::SavedAttachment>>>,
    ) -> Result<bool, PhysicalError> {
        if !live.belongs_to_document(role.scope())
            || saved
                .as_ref()
                .is_some_and(|value| !value.belongs_to_document(role.scope()))
        {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let persisted = if let Some(saved) =
            saved.filter(|saved| !self.composer().attachments_equal(saved.get()))
        {
            let (body, original) = self.composer().replacing_attachments(role, saved)?;
            Some(BundleSlot::new(BundleOwner::build(
                body,
                SlabBuilder::new(role)?,
                &original,
            )?))
        } else {
            None
        };
        let inputs = match self.inputs() {
            Some(current) => current.replacing_attachments(role, live)?,
            None => LiveInputs::prepare_attachments(role, live)?,
        };
        let extras =
            LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)?;
        let saved_changed = persisted.is_some();
        if let Some(persisted) = persisted {
            drop(std::mem::replace(&mut self.persisted, persisted));
        }
        drop(self.extras.replace(extras));
        Ok(saved_changed)
    }
    pub(super) fn add_prepared_image(
        &mut self,
        role: &ConstructionRole,
        image: NativeOwned<crate::images::managed::PreparedPromptImage>,
    ) -> Result<(), PhysicalError> {
        let inputs = LiveInputs::add_image(self.inputs(), role, image)?;
        let extras =
            LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)?;
        drop(self.extras.replace(extras));
        Ok(())
    }
    pub(super) fn edit_inputs(
        &mut self,
        role: &ConstructionRole,
        edit: ComposerInputEdit,
    ) -> Result<bool, PhysicalError> {
        let next = match edit {
            ComposerInputEdit::BeginOriginal(id) => {
                let prompt = self
                    .composer()
                    .queue()
                    .iter()
                    .find(|prompt| prompt.id() == id)
                    .ok_or(PhysicalError::QueuedPromptMissing)?;
                let retained = match self.inputs() {
                    Some(current) => current.begin_edit(role, prompt)?,
                    None => None,
                };
                match retained {
                    Some(next) => Some(next),
                    None => Some(self.begin_imported_edit(role, prompt)?),
                }
            }
            ComposerInputEdit::End => self
                .inputs()
                .map(|inputs| inputs.end_edit(role))
                .transpose()?
                .flatten(),
            ComposerInputEdit::Reset => {
                self.inputs().map(|inputs| inputs.reset(role)).transpose()?
            }
            ComposerInputEdit::Restart => self
                .inputs()
                .map(|inputs| inputs.restarted(role))
                .transpose()?,
            ComposerInputEdit::RemoveImage(index) => self
                .inputs()
                .map(|inputs| inputs.removing_image(role, index))
                .transpose()?
                .flatten(),
        };
        let Some(next) = next else {
            return Ok(false);
        };
        let extras =
            LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), next, role)?;
        drop(self.extras.replace(extras));
        Ok(true)
    }
    /// A restored or imported text prompt has no retained edit inputs; its own
    /// saved attachments become the edit, the composer's become the defaults.
    /// Images need the managed image operation first and are refused here.
    fn begin_imported_edit(
        &self,
        role: &ConstructionRole,
        prompt: &super::PromptSlot,
    ) -> Result<BundleSlot<LiveInputs>, PhysicalError> {
        if prompt.images().len() != 0 {
            return Err(PhysicalError::QueuedPromptNeedsPreparation);
        }
        let ledger = crate::model::json::deferred::DecodeLedger::for_operation(
            role,
            crate::history::MAX_RESIDENT_BYTES,
        )?;
        let attachments = live_attachments(&ledger, prompt.attachments())?;
        let images = crate::model::native::NativeList::new(&ledger, 0)?.finish();
        let restored;
        let current = match self.inputs() {
            Some(current) => current,
            None => {
                let saved = live_attachments(&ledger, self.composer().attachments())?;
                restored = LiveInputs::prepare_attachments(role, saved)?;
                restored.body()
            }
        };
        current.begin_edit_prepared(role, prompt, images, attachments)
    }
    pub(super) fn new(persisted: BundleSlot<ComposerSlot>) -> Self {
        Self {
            persisted,
            extras: None,
        }
    }
    pub(super) fn alias(&self) -> Self {
        Self {
            persisted: self.persisted.alias(),
            extras: self.extras.as_ref().map(BundleSlot::alias),
        }
    }
    pub(super) fn composer(&self) -> &ComposerSlot {
        self.persisted.body()
    }
    pub(super) fn inputs(&self) -> Option<&LiveInputs> {
        self.extras
            .as_ref()
            .and_then(|extras| extras.body().inputs())
    }
    pub(super) fn presentation(&self) -> Option<&LivePresentation> {
        self.extras
            .as_ref()
            .and_then(|extras| extras.body().presentation())
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.persisted.count(count)?;
        if let Some(extras) = &self.extras {
            extras.count(count)?;
        }
        Ok(())
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.persisted.cleanup(permit);
        if let Some(extras) = &self.extras {
            extras.cleanup(permit);
        }
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.persisted.adopt(publication);
        if let Some(extras) = &self.extras {
            extras.adopt(publication);
        }
    }
    pub(super) fn retire(&self) {
        self.persisted.retire();
        if let Some(extras) = &self.extras {
            extras.retire();
        }
    }
    /// Replaces actual volatile attachments, comparing borrowed payloads before allocation.
    /// A foreign document is rejected even when its attachment values are equal.
    pub(super) fn replace_live_attachments(
        &mut self,
        role: &ConstructionRole,
        value: NativeOwned<Vec<crate::history::live::inputs::LiveAttachment>>,
    ) -> Result<bool, PhysicalError> {
        if !value.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let equal = self.extras.as_ref().map_or_else(
            || value.get().is_empty(),
            |extras| {
                extras.body().inputs().map_or_else(
                    || value.get().is_empty(),
                    |inputs| inputs.equals_attachments(value.get()),
                )
            },
        );
        if equal {
            return Ok(false);
        }
        let inputs = match self
            .extras
            .as_ref()
            .and_then(|extras| extras.body().inputs())
        {
            Some(current) => current.replacing_attachments(role, value)?,
            None => LiveInputs::prepare_attachments(role, value)?,
        };
        let next =
            LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)?;
        let old = self.extras.replace(next);
        drop(old);
        Ok(true)
    }
    pub(super) fn replace_images(
        &mut self,
        role: &ConstructionRole,
        value: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
    ) -> Result<(), PhysicalError> {
        let inputs = match self
            .extras
            .as_ref()
            .and_then(|extras| extras.body().inputs())
        {
            Some(current) => current.replacing_images(role, value)?,
            None => LiveInputs::prepare_images(role, value)?,
        };
        let next =
            LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)?;
        drop(self.extras.replace(next));
        Ok(())
    }
    /// Drops only volatile extras. All persisted component bundles remain identical.
    pub(super) fn prepare_saved_candidate(&mut self) {
        drop(self.extras.take());
    }
    pub(super) fn update_presentation(
        &mut self,
        role: &ConstructionRole,
        update: super::presentation::PresentationUpdate,
    ) -> Result<bool, PhysicalError> {
        let previous = self
            .extras
            .as_ref()
            .and_then(|extras| extras.body().presentation());
        let Some(presentation) =
            LivePresentation::updated(role, previous, update, self.persisted.original_credit())?
        else {
            return Ok(false);
        };
        let original = presentation.original_credit().clone();
        let next = LiveExtras::replace_presentation(
            self.extras.as_ref().map(BundleSlot::body),
            presentation,
            role,
            &original,
        )?;
        drop(self.extras.replace(next));
        Ok(true)
    }
    pub(super) fn replace_usage(
        &mut self,
        role: &ConstructionRole,
        value: NativeOwned<crate::model::notice::UsageNotice>,
    ) -> Result<bool, PhysicalError> {
        self.update_presentation(role, super::presentation::PresentationUpdate::Usage(value))
    }
    pub(super) fn replace_plan(
        &mut self,
        role: &ConstructionRole,
        value: NativeOwned<Vec<crate::model::notice::PlanStep>>,
    ) -> Result<bool, PhysicalError> {
        self.update_presentation(role, super::presentation::PresentationUpdate::Plan(value))
    }
    pub(super) fn replace_composer(
        &mut self,
        role: &ConstructionRole,
        composer: NativeOwned<ComposerData>,
    ) -> Result<(), PhysicalError> {
        if !composer.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let header = NativeCredit::reserve_header(role)?;
        let (value, allocation) = composer.into_parts();
        let mut source = ControlledComponent {
            value: Some(value),
            pool: NativeSourcePool::new(role, allocation, header),
        };
        let original = source.pool.original().clone();
        let value = source.value.take().expect("fresh controlled composer");
        let mut builder = SlabBuilder::new(role)?;
        let body = ComposerSlot::new(value, &mut builder, &original, &mut source.pool)?;
        let next = BundleSlot::new(BundleOwner::build(body, builder, &original)?);
        let old = std::mem::replace(&mut self.persisted, next);
        drop(old);
        Ok(())
    }
    pub(super) fn validate_submission(
        &self,
        replace: Option<u64>,
        attachments: usize,
    ) -> Result<(), PhysicalError> {
        let replaced = replace.is_some_and(|id| {
            self.composer()
                .queue()
                .iter()
                .any(|prompt| prompt.id() == id)
        });
        if replace.is_some() && !replaced {
            return Err(PhysicalError::QueuedPromptMissing);
        }
        if self.composer().queue().len() + usize::from(!replaced) > 1024
            || self.inputs().map_or(0, |inputs| inputs.images().len()) > 128
            || attachments > 1024
        {
            return Err(PhysicalError::QueueLimit);
        }
        Ok(())
    }
    /// Construct a queued prompt from the current actual inputs, then publish all
    /// composer and auxiliary edges together after every admission succeeds.
    pub(super) fn submit_prompt(
        &mut self,
        role: &ConstructionRole,
        id: u64,
        text: NativeOwned<String>,
        attachments: NativeOwned<Vec<crate::history::SavedAttachment>>,
        replace: Option<u64>,
    ) -> Result<bool, PhysicalError> {
        if !text.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let previous = replace.and_then(|id| {
            self.composer()
                .queue()
                .iter()
                .find(|prompt| prompt.id() == id)
        });
        self.validate_submission(replace, attachments.get().len())?;
        let replaced = previous.is_some();
        let images = self.inputs().map_or(&[][..], LiveInputs::images);
        let empty_images = NativeOwned::from_parts(Vec::new(), text.ledger().reserve(0)?);
        let clear_draft = NativeOwned::from_parts(None, text.ledger().reserve(0)?);
        let prompt =
            super::PromptSlot::from_live(role, replace.unwrap_or(id), text, images, attachments)?;
        let body = if let Some(previous) = previous {
            self.composer()
                .replacing_prompt(role, previous, &prompt)?
                .ok_or(crate::model::json::deferred::JsonDecodeError::ConstructionOrder)?
        } else {
            self.composer().adding_prompt(role, &prompt)?
        };
        let original = body.original_credit().clone();
        let mut persisted = BundleSlot::new(BundleOwner::build(
            body,
            SlabBuilder::new(role)?,
            &original,
        )?);
        if !replaced {
            let (body, original) = persisted.body().replacing_draft(role, clear_draft)?;
            persisted = BundleSlot::new(BundleOwner::build(
                body,
                SlabBuilder::new(role)?,
                &original,
            )?);
        }
        let empty;
        let current = if let Some(current) = self.inputs() {
            current
        } else {
            empty = LiveInputs::empty(role, prompt.original_credit())?;
            empty.body()
        };
        let removed = previous
            .map(|previous| current.remove_prompt(role, previous))
            .transpose()?
            .flatten();
        let current = removed.as_ref().map_or(current, BundleSlot::body);
        let recorded = current.record_prompt(role, &prompt)?;
        let inputs = recorded.body().replacing_images(role, empty_images)?;
        let extras =
            LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)?;
        drop(std::mem::replace(&mut self.persisted, persisted));
        drop(self.extras.replace(extras));
        Ok(replaced)
    }
    pub(super) fn replace_draft(
        &mut self,
        role: &ConstructionRole,
        value: NativeOwned<Option<String>>,
    ) -> Result<(), PhysicalError> {
        let (body, original) = self.persisted.body().replacing_draft(role, value)?;
        let builder = SlabBuilder::new(role)?;
        let next = BundleSlot::new(BundleOwner::build(body, builder, &original)?);
        let old = std::mem::replace(&mut self.persisted, next);
        drop(old);
        Ok(())
    }
    pub(super) fn select_queue(
        &mut self,
        role: &ConstructionRole,
        selection: super::composer::QueueSelection,
    ) -> Result<(), PhysicalError> {
        let body = self.persisted.body().selecting_queue(role, selection)?;
        let mut inputs = None;
        for (index, prompt) in self.composer().queue().iter().enumerate() {
            let removed = match selection {
                super::QueueSelection::All => false,
                super::QueueSelection::WithoutFirst => index == 0,
                super::QueueSelection::OnlyFirst => index > 0,
            };
            if removed {
                let current = inputs
                    .as_ref()
                    .map(BundleSlot::body)
                    .or_else(|| self.inputs());
                if let Some(current) = current
                    && let Some(next) = current.remove_prompt(role, prompt)?
                {
                    inputs = Some(next);
                }
            }
        }
        let extras = inputs
            .map(|inputs| {
                LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)
            })
            .transpose()?;
        self.install_composer_edit(role, body)?;
        if let Some(extras) = extras {
            drop(self.extras.replace(extras));
        }
        Ok(())
    }
    /// Removes all queued prompts carrying the original id, preserving survivor order.
    /// Unknown ids return false without allocating. A refusal preserves this builder.
    pub(super) fn remove_prompt(
        &mut self,
        role: &ConstructionRole,
        id: u64,
    ) -> Result<bool, PhysicalError> {
        let next = self.persisted.body().removing_prompt(role, id)?;
        let Some(next) = next else {
            return Ok(false);
        };
        // Prepare every auxiliary removal from the original physical slots before
        // publishing either edge. Duplicate numeric ids keep their own identities.
        let mut inputs = None;
        for prompt in self
            .persisted
            .body()
            .queue()
            .iter()
            .filter(|prompt| prompt.id() == id)
        {
            let current = inputs
                .as_ref()
                .map(BundleSlot::body)
                .or_else(|| self.inputs());
            if let Some(current) = current
                && let Some(next) = current.remove_prompt(role, prompt)?
            {
                inputs = Some(next);
            }
        }
        let extras = inputs
            .map(|inputs| {
                LiveExtras::replace_inputs(self.extras.as_ref().map(BundleSlot::body), inputs, role)
            })
            .transpose()?;
        self.install_composer_edit(role, next)?;
        if let Some(extras) = extras {
            drop(self.extras.replace(extras));
        }
        Ok(true)
    }
    /// Moves the first queued prompt with the original id to its final zero-based index.
    /// Target zero matches feature Composer's move-to-front rule. Unknown ids, unchanged
    /// positions and targets outside the queue return false without allocating.
    /// Only the queue vector is rebuilt; prompt payloads retain their genuine owners.
    pub(super) fn move_prompt(
        &mut self,
        role: &ConstructionRole,
        id: u64,
        target: usize,
    ) -> Result<bool, PhysicalError> {
        let next = self.persisted.body().moving_prompt(role, id, target)?;
        let Some(next) = next else {
            return Ok(false);
        };
        self.install_composer_edit(role, next)?;
        Ok(true)
    }
    fn install_composer_edit(
        &mut self,
        role: &ConstructionRole,
        body: ComposerSlot,
    ) -> Result<(), PhysicalError> {
        let original = body.original_credit().clone();
        let next = BundleSlot::new(BundleOwner::build(
            body,
            SlabBuilder::new(role)?,
            &original,
        )?);
        let old = std::mem::replace(&mut self.persisted, next);
        drop(old);
        Ok(())
    }
}
struct ControlledComponent<T> {
    value: Option<T>,
    pool: NativeSourcePool,
}
/// Saved attachment identities as live ones; a local terminal is unavailable until re-bound.
fn live_attachments(
    ledger: &crate::model::json::deferred::DecodeLedger,
    values: &[super::composer::AttachmentSlot],
) -> Result<NativeOwned<Vec<crate::history::live::inputs::LiveAttachment>>, PhysicalError> {
    use super::composer::AttachmentSlot;
    use crate::history::live::inputs;
    let copies = crate::model::native::NativeCopies::new(ledger);
    let mut list = crate::model::native::NativeList::new(ledger, values.len())?;
    for value in values {
        let live = match value {
            AttachmentSlot::Connection(text) => inputs::connection(copies.text(&text.read())?),
            AttachmentSlot::Group(text) => inputs::group(copies.text(&text.read())?),
            AttachmentSlot::LocalTerminal(text) => {
                inputs::unavailable_local(copies.text(&text.read())?)
            }
        };
        list.push(live)?;
    }
    Ok(list.finish())
}
