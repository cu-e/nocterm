//! Real volatile input storage, independently shared from persisted components.
use super::{
    bundle::{Body, BundleOwner, BundleSlot},
    slots::{
        NativeCredit, NativeSource, NativeSourcePool, PhysicalError, SlabBuilder, TextSlot,
        vector_bytes,
    },
};
use crate::{
    history::live::inputs::{LiveAttachment, LocalItemKey},
    model::{
        native::NativeOwned,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
};

mod auxiliary;
mod defaults;
mod images;
use auxiliary::PromptAuxiliary;
use defaults::Defaults;
use images::LiveImages;
pub use images::{ImageBinaryGuard, ManagedImage};
#[cfg(test)]
mod attachment_lifetimes;
#[cfg(test)]
mod image_lifetimes;

pub struct LiveInputs {
    attachments: BundleSlot<LiveAttachments>,
    images: Option<BundleSlot<LiveImages>>,
    defaults: Option<BundleSlot<Defaults>>,
    auxiliary: Option<BundleSlot<PromptAuxiliary>>,
}
pub(in crate::history::physical) struct LiveAttachments {
    values: Vec<LiveAttachmentSlot>,
}
/// Borrowed live attachment payloads; local keys are never persisted or authority.
pub enum LiveAttachmentSlot {
    Connection(TextSlot),
    Group(TextSlot),
    UnavailableLocal(TextSlot),
    LocalItem(LocalItemKey),
}
impl LiveAttachmentSlot {
    fn new(value: LiveAttachment, pool: &mut NativeSourcePool) -> Result<Self, PhysicalError> {
        Ok(match value {
            LiveAttachment::Connection(text) => Self::Connection(TextSlot::new(text, pool)?),
            LiveAttachment::Group(text) => Self::Group(TextSlot::new(text, pool)?),
            LiveAttachment::UnavailableLocal(text) => {
                Self::UnavailableLocal(TextSlot::new(text, pool)?)
            }
            LiveAttachment::LocalItem(key) => Self::LocalItem(key),
        })
    }
    fn text(&self) -> Option<&TextSlot> {
        match self {
            Self::Connection(text) | Self::Group(text) | Self::UnavailableLocal(text) => Some(text),
            Self::LocalItem(_) => None,
        }
    }
    fn equals(&self, value: &LiveAttachment) -> bool {
        match (self, value) {
            (Self::Connection(left), LiveAttachment::Connection(right))
            | (Self::Group(left), LiveAttachment::Group(right))
            | (Self::UnavailableLocal(left), LiveAttachment::UnavailableLocal(right)) => {
                &*left.read() == right.as_str()
            }
            (Self::LocalItem(left), LiveAttachment::LocalItem(right)) => left == right,
            _ => false,
        }
    }
}
impl LiveInputs {
    pub(in crate::history::physical) fn prompt_attachments(
        &self,
        prompt: &super::PromptSlot,
    ) -> Option<BundleSlot<LiveAttachments>> {
        self.auxiliary
            .as_ref()
            .and_then(|auxiliary| auxiliary.body().attachments(prompt))
    }
    pub(super) fn add_image(
        previous: Option<&Self>,
        role: &ConstructionRole,
        value: NativeOwned<crate::images::managed::PreparedPromptImage>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let images = LiveImages::append(
            previous.and_then(|inputs| inputs.images.as_ref().map(BundleSlot::body)),
            role,
            value,
        )?;
        let original = images.original_credit().clone();
        let attachments = if let Some(previous) = previous {
            previous.attachments.alias()
        } else {
            BundleSlot::new(BundleOwner::build(
                LiveAttachments { values: Vec::new() },
                SlabBuilder::new(role)?,
                &original,
            )?)
        };
        let body = Self {
            attachments,
            images: Some(images),
            defaults: previous.and_then(|inputs| inputs.defaults.as_ref().map(BundleSlot::alias)),
            auxiliary: previous.and_then(|inputs| inputs.auxiliary.as_ref().map(BundleSlot::alias)),
        };
        bundle_inputs(role, body, &original)
    }
    pub(super) fn removing_image(
        &self,
        role: &ConstructionRole,
        index: usize,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        let Some(images) = self
            .images
            .as_ref()
            .filter(|images| index < images.body().values().len())
        else {
            return Ok(None);
        };
        let images = images
            .body()
            .without(role, index, images.original_credit())?;
        let body = Self {
            attachments: self.attachments.alias(),
            images: Some(images),
            defaults: self.defaults.as_ref().map(BundleSlot::alias),
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        bundle_inputs(role, body, self.attachments.original_credit()).map(Some)
    }
    pub(super) fn empty(
        role: &ConstructionRole,
        original: &crate::model::json::lifetime::Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let attachments = BundleSlot::new(BundleOwner::build(
            LiveAttachments { values: Vec::new() },
            SlabBuilder::new(role)?,
            original,
        )?);
        bundle_inputs(
            role,
            Self {
                attachments,
                images: None,
                defaults: None,
                auxiliary: None,
            },
            original,
        )
    }
    pub fn attachments(&self) -> &[LiveAttachmentSlot] {
        &self.attachments.body().values
    }
    pub fn images(&self) -> &[ManagedImage] {
        self.images
            .as_ref()
            .map_or(&[], |images| images.body().values())
    }
    pub fn default_attachments(&self) -> &[LiveAttachmentSlot] {
        self.defaults.as_ref().map_or_else(
            || self.attachments(),
            |defaults| defaults.body().attachments(),
        )
    }
    pub fn editing(&self) -> bool {
        self.defaults.is_some()
    }
    pub(super) fn replacing_attachments(
        &self,
        role: &ConstructionRole,
        value: NativeOwned<Vec<LiveAttachment>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let prepared = Self::prepare_attachments(role, value)?;
        let body = Self {
            attachments: prepared.body().attachments.alias(),
            images: self.images.as_ref().map(BundleSlot::alias),
            defaults: self.defaults.as_ref().map(BundleSlot::alias),
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        bundle_inputs(role, body, prepared.original_credit())
    }
    pub(super) fn prepare_images(
        role: &ConstructionRole,
        value: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        if !value.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        validate_images(value.get())?;
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let images = LiveImages::build(role, source.take(), &original, source.pool())?;
        let attachments = BundleSlot::new(BundleOwner::build(
            LiveAttachments { values: Vec::new() },
            SlabBuilder::new(role)?,
            &original,
        )?);
        bundle_inputs(
            role,
            Self {
                attachments,
                images: Some(images),
                defaults: None,
                auxiliary: None,
            },
            &original,
        )
    }
    pub(super) fn replacing_images(
        &self,
        role: &ConstructionRole,
        value: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let prepared = Self::prepare_images(role, value)?;
        let body = Self {
            attachments: self.attachments.alias(),
            images: prepared.body().images.as_ref().map(BundleSlot::alias),
            defaults: self.defaults.as_ref().map(BundleSlot::alias),
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        bundle_inputs(role, body, prepared.original_credit())
    }
    pub fn default_images(&self) -> &[ManagedImage] {
        self.defaults
            .as_ref()
            .map_or_else(|| self.images(), |defaults| defaults.body().images())
    }
    pub(super) fn record_prompt(
        &self,
        role: &ConstructionRole,
        prompt: &super::PromptSlot,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let auxiliary = PromptAuxiliary::record(
            role,
            self.auxiliary.as_ref().map(BundleSlot::body),
            prompt,
            self,
        )?;
        let body = Self {
            attachments: self.attachments.alias(),
            images: self.images.as_ref().map(BundleSlot::alias),
            defaults: self.defaults.as_ref().map(BundleSlot::alias),
            auxiliary: Some(auxiliary),
        };
        bundle_inputs(role, body, self.attachments.original_credit())
    }
    /// Only the actual original prompt can select its volatile image/context owners.
    /// Imported prompts without these owners require controlled image preparation.
    pub(super) fn begin_edit(
        &self,
        role: &ConstructionRole,
        prompt: &super::PromptSlot,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        if self.defaults.is_some() {
            return Err(crate::model::json::deferred::JsonDecodeError::ConstructionOrder.into());
        }
        let Some((attachments, images)) = self
            .auxiliary
            .as_ref()
            .and_then(|auxiliary| auxiliary.body().find(prompt))
        else {
            return Ok(None);
        };
        let defaults = Some(Defaults::capture(role, self)?);
        let body = Self {
            attachments,
            images,
            defaults,
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        bundle_inputs(role, body, self.attachments.original_credit()).map(Some)
    }
    /// Imported prompts are reconstructed once by the managed image operation.
    /// Their original persisted image bound is separate from interactive8/20MiB.
    pub(super) fn begin_edit_prepared(
        &self,
        role: &ConstructionRole,
        prompt: &super::PromptSlot,
        images: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
        attachments: NativeOwned<Vec<LiveAttachment>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        if self.defaults.is_some()
            || self
                .auxiliary
                .as_ref()
                .is_some_and(|auxiliary| auxiliary.body().contains(prompt))
        {
            return Err(crate::model::json::deferred::JsonDecodeError::ConstructionOrder.into());
        }
        if !images.ledger().belongs_to_document(role.scope())
            || !attachments.ledger().belongs_to_document(role.scope())
        {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let expected_images = prompt.images().filter(|value| matches!(value, super::ContentSlot::Media(media) if media.kind() == crate::model::content::MediaKind::Image)).count();
        if images.get().len() != expected_images {
            return Err(crate::model::json::deferred::JsonDecodeError::ConstructionOrder.into());
        }
        let mut source = NativeSource::new(images.join(attachments)?, role)?;
        let original = source.original().clone();
        let (images, attachments) = source.take();
        let images = Some(LiveImages::build(role, images, &original, source.pool())?);
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(attachments, source.pool(), |value, _, pool| {
            LiveAttachmentSlot::new(value, pool)
        })?;
        let attachments = BundleSlot::new(BundleOwner::build(
            LiveAttachments { values },
            builder,
            &original,
        )?);
        let mut body = Self {
            attachments,
            images,
            defaults: Some(Defaults::capture(role, self)?),
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        let auxiliary = PromptAuxiliary::record(
            role,
            self.auxiliary.as_ref().map(BundleSlot::body),
            prompt,
            &body,
        )?;
        body.auxiliary = Some(auxiliary);
        bundle_inputs(role, body, &original)
    }
    pub(super) fn end_edit(
        &self,
        role: &ConstructionRole,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        let Some(defaults) = &self.defaults else {
            return Ok(None);
        };
        let (attachments, images) = defaults.body().restore();
        let body = Self {
            attachments,
            images,
            defaults: None,
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        bundle_inputs(role, body, self.attachments.original_credit()).map(Some)
    }
    pub(super) fn reset(&self, role: &ConstructionRole) -> Result<BundleSlot<Self>, PhysicalError> {
        let attachments = BundleSlot::new(BundleOwner::build(
            LiveAttachments { values: Vec::new() },
            SlabBuilder::new(role)?,
            self.attachments.original_credit(),
        )?);
        bundle_inputs(
            role,
            Self {
                attachments,
                images: None,
                defaults: None,
                auxiliary: None,
            },
            self.attachments.original_credit(),
        )
    }
    /// Restart preserves current images and restores the ordinary attachment scope,
    /// matching the original composer restart while ending edit coordination.
    pub(super) fn restarted(
        &self,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let attachments = self.defaults.as_ref().map_or_else(
            || self.attachments.alias(),
            |defaults| defaults.body().restore().0,
        );
        let body = Self {
            attachments,
            images: self.images.as_ref().map(BundleSlot::alias),
            defaults: None,
            auxiliary: self.auxiliary.as_ref().map(BundleSlot::alias),
        };
        bundle_inputs(role, body, self.attachments.original_credit())
    }
    pub(super) fn remove_prompt(
        &self,
        role: &ConstructionRole,
        prompt: &super::PromptSlot,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        let Some(auxiliary) = &self.auxiliary else {
            return Ok(None);
        };
        if !auxiliary.body().contains(prompt) {
            return Ok(None);
        }
        let next = auxiliary.body().remove(role, prompt)?;
        let body = Self {
            attachments: self.attachments.alias(),
            images: self.images.as_ref().map(BundleSlot::alias),
            defaults: self.defaults.as_ref().map(BundleSlot::alias),
            auxiliary: Some(next),
        };
        bundle_inputs(role, body, self.attachments.original_credit()).map(Some)
    }
    pub(super) fn equals_attachments(&self, values: &[LiveAttachment]) -> bool {
        let current = self.attachments();
        current.len() == values.len()
            && current
                .iter()
                .zip(values)
                .all(|(left, right)| left.equals(right))
    }
    pub(super) fn prepare_attachments(
        role: &ConstructionRole,
        value: NativeOwned<Vec<LiveAttachment>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(source.take(), source.pool(), |value, _, pool| {
            LiveAttachmentSlot::new(value, pool)
        })?;
        let attachments = BundleSlot::new(BundleOwner::build(
            LiveAttachments { values },
            builder,
            &original,
        )?);
        let body = Self {
            attachments,
            images: None,
            defaults: None,
            auxiliary: None,
        };
        Ok(BundleSlot::new(BundleOwner::build(
            body,
            SlabBuilder::new(role)?,
            &original,
        )?))
    }
}

impl LiveAttachments {
    pub(in crate::history::physical) fn values(&self) -> &[LiveAttachmentSlot] {
        &self.values
    }
}
impl Body for LiveInputs {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.attachments.count(count)?;
        if let Some(images) = &self.images {
            images.count(count)?;
        }
        if let Some(defaults) = &self.defaults {
            defaults.count(count)?;
        }
        if let Some(auxiliary) = &self.auxiliary {
            auxiliary.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.attachments.cleanup(permit);
        if let Some(images) = &self.images {
            images.cleanup(permit);
        }
        if let Some(defaults) = &self.defaults {
            defaults.cleanup(permit);
        }
        if let Some(auxiliary) = &self.auxiliary {
            auxiliary.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.attachments.adopt(publication);
        if let Some(images) = &self.images {
            images.adopt(publication);
        }
        if let Some(defaults) = &self.defaults {
            defaults.adopt(publication);
        }
        if let Some(auxiliary) = &self.auxiliary {
            auxiliary.adopt(publication);
        }
    }
    fn retire(&self) {
        self.attachments.retire();
        if let Some(images) = &self.images {
            images.retire();
        }
        if let Some(defaults) = &self.defaults {
            defaults.retire();
        }
        if let Some(auxiliary) = &self.auxiliary {
            auxiliary.retire();
        }
    }
}
impl Body for LiveAttachments {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.values)?)?;
        for value in &self.values {
            if let Some(text) = value.text() {
                text.count(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for value in &self.values {
            if let Some(text) = value.text() {
                text.cleanup(permit);
            }
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for value in &self.values {
            if let Some(text) = value.text() {
                text.adopt(publication);
            }
        }
    }
    fn retire(&self) {
        for value in &self.values {
            if let Some(text) = value.text() {
                text.retire();
            }
        }
    }
}

fn bundle_inputs(
    role: &ConstructionRole,
    body: LiveInputs,
    original: &crate::model::json::lifetime::Shared<NativeCredit>,
) -> Result<BundleSlot<LiveInputs>, PhysicalError> {
    Ok(BundleSlot::new(BundleOwner::build(
        body,
        SlabBuilder::new(role)?,
        original,
    )?))
}
fn validate_images(
    values: &[crate::images::managed::PreparedPromptImage],
) -> Result<(), PhysicalError> {
    if values.len() > crate::images::MAX_IMAGES
        || values
            .iter()
            .map(|value| value.binary().bytes().len())
            .sum::<usize>()
            > crate::images::MAX_TOTAL_IMAGE_BYTES
    {
        return Err(crate::images::ImageCollectionError.into());
    }
    Ok(())
}
