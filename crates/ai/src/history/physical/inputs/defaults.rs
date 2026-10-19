//! Independently retained ordinary inputs while an original queued prompt is edited.
use super::super::{
    bundle::{Body, BundleOwner, BundleSlot},
    slots::{PhysicalError, SlabBuilder},
};
use super::{LiveAttachmentSlot, LiveAttachments, LiveImages, LiveInputs, ManagedImage};
use crate::model::retention::{
    CensusPermit, RetentionError,
    document::{ConstructionRole, Publication},
    graph::Count,
};

pub(super) struct Defaults {
    attachments: BundleSlot<LiveAttachments>,
    images: Option<BundleSlot<LiveImages>>,
}
impl Defaults {
    pub(super) fn capture(
        role: &ConstructionRole,
        inputs: &LiveInputs,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let body = Self {
            attachments: inputs.attachments.alias(),
            images: inputs.images.as_ref().map(BundleSlot::alias),
        };
        Ok(BundleSlot::new(BundleOwner::build(
            body,
            SlabBuilder::new(role)?,
            inputs.attachments.original_credit(),
        )?))
    }
    pub(super) fn attachments(&self) -> &[LiveAttachmentSlot] {
        &self.attachments.body().values
    }
    pub(super) fn images(&self) -> &[ManagedImage] {
        self.images
            .as_ref()
            .map_or(&[], |images| images.body().values())
    }
    pub(super) fn restore(&self) -> (BundleSlot<LiveAttachments>, Option<BundleSlot<LiveImages>>) {
        (
            self.attachments.alias(),
            self.images.as_ref().map(BundleSlot::alias),
        )
    }
}
impl Body for Defaults {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.attachments.count(count)?;
        if let Some(images) = &self.images {
            images.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.attachments.cleanup(permit);
        if let Some(images) = &self.images {
            images.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.attachments.adopt(publication);
        if let Some(images) = &self.images {
            images.adopt(publication);
        }
    }
    fn retire(&self) {
        self.attachments.retire();
        if let Some(images) = &self.images {
            images.retire();
        }
    }
}
