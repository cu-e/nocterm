//! Actual prompt ownership is independent of queue-vector and draft lifetime.
//! Edits alias genuine prompt tokens, preserving full queued media until dispatch.
use super::{
    bundle::{self, Body, BundleOwner, BundleSlot},
    content::ContentSlot,
    slots::*,
};
use crate::{
    history::{ComposerData, SavedAttachment, SavedPrompt},
    model::{
        json::lifetime::Shared,
        native::NativeOwned,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
};

#[cfg(test)]
mod image_bundles;
#[cfg(test)]
mod queue_tests;

pub struct ComposerSlot {
    draft: BundleSlot<DraftSlot>,
    queue: BundleSlot<QueueSlot>,
    attachments: BundleSlot<AttachmentList>,
}
struct DraftSlot {
    text: Option<TextSlot>,
}
struct QueueSlot {
    prompts: Vec<PromptSlot>,
}
struct AttachmentList {
    values: Vec<AttachmentSlot>,
}
pub struct PromptSlot {
    payload: BundleSlot<PromptBody>,
    policy: Option<super::policy::ComponentPolicy>,
}
struct PromptBody {
    id: u64,
    text: BundleSlot<TextSlot>,
    images: Vec<BundleSlot<ContentSlot>>,
    attachments: Vec<AttachmentSlot>,
}
pub enum AttachmentSlot {
    Connection(TextSlot),
    Group(TextSlot),
    LocalTerminal(TextSlot),
}
#[derive(Clone, Copy)]
pub enum QueueSelection {
    All,
    WithoutFirst,
    OnlyFirst,
}
impl AttachmentSlot {
    fn new(value: SavedAttachment, pool: &mut NativeSourcePool) -> Result<Self, PhysicalError> {
        Ok(match value {
            SavedAttachment::Connection(id) => Self::Connection(TextSlot::new(id, pool)?),
            SavedAttachment::Group(id) => Self::Group(TextSlot::new(id, pool)?),
            SavedAttachment::LocalTerminal(id) => Self::LocalTerminal(TextSlot::new(id, pool)?),
        })
    }
    fn text(&self) -> &TextSlot {
        match self {
            Self::Connection(text) | Self::Group(text) | Self::LocalTerminal(text) => text,
        }
    }
    pub fn id(&self) -> TextGuard<'_> {
        self.text().read()
    }
}
impl PromptSlot {
    fn certified(payload: BundleSlot<PromptBody>) -> Result<Self, PhysicalError> {
        let mut prompt = Self {
            payload,
            policy: None,
        };
        prompt.policy = Some(super::policy::ComponentPolicy::prompt(&prompt)?);
        Ok(prompt)
    }
    pub(crate) fn cached_policy(&self) -> Result<usize, crate::history::MeasurementError> {
        self.policy
            .map(super::policy::ComponentPolicy::bytes)
            .ok_or(crate::history::MeasurementError::Retention(
                RetentionError::UncertifiedPolicy,
            ))
    }
    fn new(
        value: SavedPrompt,
        queue_builder: &mut SlabBuilder<'_>,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let mut builder = SlabBuilder::new(queue_builder.role())?;
        let images = builder.map_in(value.images, pool, |value, builder, pool| {
            let content = ContentSlot::new(value, builder.role(), original, pool)?;
            bundle::part(builder.role(), original, content)
        })?;
        let attachments = builder.map_in(value.attachments, pool, |value, _, pool| {
            AttachmentSlot::new(value, pool)
        })?;
        let body = PromptBody {
            id: value.id,
            text: bundle::part(
                queue_builder.role(),
                original,
                TextSlot::new(value.text, pool)?,
            )?,
            images,
            attachments,
        };
        Self::certified(BundleSlot::new(BundleOwner::build(
            body, builder, original,
        )?))
    }
    /// Construct the actual queued payload once from original managed inputs.
    pub(super) fn from_live(
        role: &ConstructionRole,
        id: u64,
        text: NativeOwned<String>,
        images: &[super::ManagedImage],
        attachments: NativeOwned<Vec<SavedAttachment>>,
    ) -> Result<Self, PhysicalError> {
        let mut source = NativeSource::new(text.join(attachments)?, role)?;
        let original = source.original().clone();
        let (text, attachments) = source.take();
        let mut builder = SlabBuilder::new(role)?;
        let text = bundle::part(role, &original, TextSlot::new(text, source.pool())?)?;
        let mut content = builder.vector(images.len())?;
        content.extend(images.iter().map(super::ManagedImage::content_alias));
        let attachments = builder.map_in(attachments, source.pool(), |value, _, pool| {
            AttachmentSlot::new(value, pool)
        })?;
        Self::certified(BundleSlot::new(BundleOwner::build(
            PromptBody {
                id,
                text,
                images: content,
                attachments,
            },
            builder,
            &original,
        )?))
    }
    /// A submitted User row shares the original prompt text and every image.
    pub(super) fn user_items(
        &self,
        builder: &mut SlabBuilder<'_>,
    ) -> Result<Vec<BundleSlot<ContentSlot>>, PhysicalError> {
        let body = self.payload.body();
        let capacity = body
            .images
            .len()
            .checked_add(1)
            .ok_or(RetentionError::Arithmetic)?;
        let mut items = builder.vector(capacity)?;
        items.push(bundle::part(
            builder.role(),
            self.payload.original_credit(),
            ContentSlot::from_prompt_text(body.text.alias()),
        )?);
        items.extend(body.images.iter().map(BundleSlot::alias));
        Ok(items)
    }
    pub(super) fn original_credit(&self) -> &Shared<NativeCredit> {
        self.payload.original_credit()
    }
    pub(super) fn alias(&self) -> Self {
        Self {
            payload: self.payload.alias(),
            policy: self.policy,
        }
    }
    pub fn id(&self) -> u64 {
        self.payload.body().id
    }
    pub(super) fn same_payload(&self, other: &Self) -> bool {
        self.payload.same_payload(&other.payload)
    }
    pub fn text(&self) -> TextGuard<'_> {
        self.payload.body().text.body().read()
    }
    /// Borrows the actual independently owned content bundles, without DTO copies.
    pub(super) fn image_capacity(&self) -> usize {
        self.payload.body().images.capacity()
    }
    pub(super) fn attachment_capacity(&self) -> usize {
        self.payload.body().attachments.capacity()
    }
    pub fn images(&self) -> impl ExactSizeIterator<Item = &ContentSlot> + Clone {
        self.payload.body().images.iter().map(BundleSlot::body)
    }
    pub fn attachments(&self) -> &[AttachmentSlot] {
        &self.payload.body().attachments
    }
}
impl Body for PromptBody {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.text.count(count)?;
        count.add_exclusive(vector_bytes(&self.images)?)?;
        count.add_exclusive(vector_bytes(&self.attachments)?)?;
        for value in &self.images {
            value.count(count)?;
        }
        for value in &self.attachments {
            value.text().count(count)?;
        }
        Ok(())
    }
    fn retire(&self) {
        self.text.retire();
        for value in &self.images {
            value.retire();
        }
        for value in &self.attachments {
            value.text().retire();
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.text.cleanup(permit);
        for value in &self.attachments {
            value.text().cleanup(permit);
        }
        for value in &self.images {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.text.adopt(publication);
        for value in &self.attachments {
            value.text().adopt(publication);
        }
        for value in &self.images {
            value.adopt(publication);
        }
    }
}
impl Body for PromptSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.payload.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.payload.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.payload.adopt(publication);
    }
    fn retire(&self) {
        self.payload.retire();
    }
}
impl ComposerSlot {
    pub(super) fn new(
        value: ComposerData,
        parent: &mut SlabBuilder<'_>,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let role = parent.role();
        let draft = bundle::part(
            role,
            original,
            DraftSlot {
                text: optional_text(value.draft, pool)?,
            },
        )?;
        let mut builder = SlabBuilder::new(role)?;
        let prompts = builder.map_in(value.queue, pool, |value, builder, pool| {
            PromptSlot::new(value, builder, original, pool)
        })?;
        let queue = BundleSlot::new(BundleOwner::build(
            QueueSlot { prompts },
            builder,
            original,
        )?);
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(value.attachments, pool, |value, _, pool| {
            AttachmentSlot::new(value, pool)
        })?;
        let attachments = BundleSlot::new(BundleOwner::build(
            AttachmentList { values },
            builder,
            original,
        )?);
        Ok(Self {
            draft,
            queue,
            attachments,
        })
    }
    pub(super) fn queue_capacity(&self) -> usize {
        self.queue.body().prompts.capacity()
    }
    pub(super) fn attachment_capacity(&self) -> usize {
        self.attachments.body().values.capacity()
    }
    pub fn draft(&self) -> Option<TextGuard<'_>> {
        self.draft.body().text.as_ref().map(TextSlot::read)
    }
    pub fn queue(&self) -> &[PromptSlot] {
        &self.queue.body().prompts
    }
    pub fn attachments(&self) -> &[AttachmentSlot] {
        &self.attachments.body().values
    }
    pub(super) fn attachments_equal(&self, values: &[SavedAttachment]) -> bool {
        self.attachments().len() == values.len()
            && self
                .attachments()
                .iter()
                .zip(values)
                .all(|(left, right)| match (left, right) {
                    (AttachmentSlot::Connection(left), SavedAttachment::Connection(right))
                    | (AttachmentSlot::Group(left), SavedAttachment::Group(right))
                    | (
                        AttachmentSlot::LocalTerminal(left),
                        SavedAttachment::LocalTerminal(right),
                    ) => &*left.read() == right,
                    _ => false,
                })
    }
    pub(super) fn replacing_attachments(
        &self,
        role: &ConstructionRole,
        value: NativeOwned<Vec<SavedAttachment>>,
    ) -> Result<(Self, Shared<NativeCredit>), PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(source.take(), source.pool(), |value, _, pool| {
            AttachmentSlot::new(value, pool)
        })?;
        let attachments = BundleSlot::new(BundleOwner::build(
            AttachmentList { values },
            builder,
            &original,
        )?);
        Ok((
            Self {
                draft: self.draft.alias(),
                queue: self.queue.alias(),
                attachments,
            },
            original,
        ))
    }
    pub(super) fn replacing_draft(
        &self,
        role: &ConstructionRole,
        value: NativeOwned<Option<String>>,
    ) -> Result<(Self, Shared<NativeCredit>), PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let draft = bundle::part(
            role,
            &original,
            DraftSlot {
                text: optional_text(source.take(), source.pool())?,
            },
        )?;
        Ok((
            Self {
                draft,
                queue: self.queue.alias(),
                attachments: self.attachments.alias(),
            },
            original,
        ))
    }
    pub(super) fn adding_prompt(
        &self,
        role: &ConstructionRole,
        prompt: &PromptSlot,
    ) -> Result<Self, PhysicalError> {
        let source = self.queue();
        let len = source
            .len()
            .checked_add(1)
            .ok_or(RetentionError::Arithmetic)?;
        self.rebuilding_queue(
            role,
            len,
            source.iter().chain(std::iter::once(prompt)),
            prompt.original_credit(),
        )
    }
    pub(super) fn replacing_prompt(
        &self,
        role: &ConstructionRole,
        original: &PromptSlot,
        replacement: &PromptSlot,
    ) -> Result<Option<Self>, PhysicalError> {
        let source = self.queue();
        let Some(index) = source
            .iter()
            .position(|prompt| prompt.same_payload(original))
        else {
            return Ok(None);
        };
        self.rebuilding_queue(
            role,
            source.len(),
            source.iter().enumerate().map(|(position, prompt)| {
                if position == index {
                    replacement
                } else {
                    prompt
                }
            }),
            replacement.original_credit(),
        )
        .map(Some)
    }
    pub(super) fn selecting_queue(
        &self,
        role: &ConstructionRole,
        selection: QueueSelection,
    ) -> Result<Self, PhysicalError> {
        let source = &self.queue.body().prompts;
        let range = match selection {
            QueueSelection::All => 0..source.len(),
            QueueSelection::WithoutFirst => source.len().min(1)..source.len(),
            QueueSelection::OnlyFirst => 0..source.len().min(1),
        };
        self.rebuilding_queue(
            role,
            range.len(),
            source[range].iter(),
            self.queue.original_credit(),
        )
    }
    pub(super) fn removing_prompt(
        &self,
        role: &ConstructionRole,
        id: u64,
    ) -> Result<Option<Self>, PhysicalError> {
        let source = self.queue();
        let removed = source.iter().filter(|prompt| prompt.id() == id).count();
        if removed == 0 {
            return Ok(None);
        }
        // Feature Composer::remove removes every occurrence of the original id.
        self.rebuilding_queue(
            role,
            source.len() - removed,
            source.iter().filter(|prompt| prompt.id() != id),
            self.queue.original_credit(),
        )
        .map(Some)
    }
    pub(super) fn moving_prompt(
        &self,
        role: &ConstructionRole,
        id: u64,
        target: usize,
    ) -> Result<Option<Self>, PhysicalError> {
        let source = self.queue();
        let Some(index) = source.iter().position(|prompt| prompt.id() == id) else {
            return Ok(None);
        };
        if index == target || target >= source.len() {
            return Ok(None);
        }
        // Map final positions to original slots without a temporary payload/vector copy.
        let prompts = (0..source.len()).map(|position| {
            let original = if position == target {
                index
            } else if index < target && (index..target).contains(&position) {
                position + 1
            } else if target < index && (target + 1..=index).contains(&position) {
                position - 1
            } else {
                position
            };
            &source[original]
        });
        self.rebuilding_queue(role, source.len(), prompts, self.queue.original_credit())
            .map(Some)
    }
    fn rebuilding_queue<'a>(
        &self,
        role: &ConstructionRole,
        len: usize,
        source: impl Iterator<Item = &'a PromptSlot>,
        original: &Shared<NativeCredit>,
    ) -> Result<Self, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let mut prompts = builder.vector(len)?;
        for prompt in source {
            prompts.push(prompt.alias());
        }
        // New real queue metadata is prepaid by this builder. Its source is the
        // actual unchanged queue or the original newly installed prompt.
        let queue = BundleSlot::new(BundleOwner::build(
            QueueSlot { prompts },
            builder,
            original,
        )?);
        Ok(Self {
            draft: self.draft.alias(),
            queue,
            attachments: self.attachments.alias(),
        })
    }
    pub(super) fn original_credit(&self) -> &Shared<NativeCredit> {
        self.draft.original_credit()
    }
}
impl Body for DraftSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.text
            .as_ref()
            .map_or(Ok(()), |value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(value) = &self.text {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(value) = &self.text {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        retire_text(&self.text);
    }
}
impl Body for QueueSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.prompts)?)?;
        for value in &self.prompts {
            value.payload.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for value in &self.prompts {
            value.payload.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for value in &self.prompts {
            value.payload.adopt(publication);
        }
    }
    fn retire(&self) {
        for value in &self.prompts {
            value.payload.retire();
        }
    }
}
impl Body for AttachmentList {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.values)?)?;
        for value in &self.values {
            value.text().count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for value in &self.values {
            value.text().cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for value in &self.values {
            value.text().adopt(publication);
        }
    }
    fn retire(&self) {
        for value in &self.values {
            value.text().retire();
        }
    }
}
impl Body for ComposerSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.draft.count(count)?;
        self.queue.count(count)?;
        self.attachments.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.draft.cleanup(permit);
        self.queue.cleanup(permit);
        self.attachments.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.draft.adopt(publication);
        self.queue.adopt(publication);
        self.attachments.adopt(publication);
    }
    fn retire(&self) {
        self.draft.retire();
        self.queue.retire();
        self.attachments.retire();
    }
}
