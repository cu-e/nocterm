//! Prompt-local volatile payloads bind the actual prompt owner, never a numeric key.
use super::super::{
    PromptSlot,
    bundle::{Body, BundleOwner, BundleSlot},
    slots::{PhysicalError, SlabBuilder, vector_bytes},
};
use super::{LiveAttachments, LiveImages, LiveInputs};
use crate::model::retention::{
    CensusPermit, RetentionError,
    document::{ConstructionRole, Publication},
    graph::Count,
};

pub(super) struct PromptAuxiliary {
    values: Vec<PromptAuxSlot>,
}
struct PromptAuxSlot {
    prompt: PromptSlot,
    attachments: BundleSlot<LiveAttachments>,
    images: Option<BundleSlot<LiveImages>>,
}
impl PromptAuxSlot {
    fn alias(&self) -> Self {
        Self {
            prompt: self.prompt.alias(),
            attachments: self.attachments.alias(),
            images: self.images.as_ref().map(BundleSlot::alias),
        }
    }
}
impl PromptAuxiliary {
    pub(super) fn record(
        role: &ConstructionRole,
        previous: Option<&Self>,
        prompt: &PromptSlot,
        inputs: &LiveInputs,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let previous = previous.map_or(&[][..], |value| value.values.as_slice());
        let exists = previous
            .iter()
            .any(|value| value.prompt.same_payload(prompt));
        let mut builder = SlabBuilder::new(role)?;
        let mut values = builder.vector(previous.len() + usize::from(!exists))?;
        for value in previous {
            values.push(value.alias());
        }
        if !exists {
            values.push(PromptAuxSlot {
                prompt: prompt.alias(),
                attachments: inputs.attachments.alias(),
                images: inputs.images.as_ref().map(BundleSlot::alias),
            });
        }
        Ok(BundleSlot::new(BundleOwner::build(
            Self { values },
            builder,
            inputs.attachments.original_credit(),
        )?))
    }
    pub(super) fn contains(&self, prompt: &PromptSlot) -> bool {
        self.values
            .iter()
            .any(|value| value.prompt.same_payload(prompt))
    }
    pub(super) fn attachments(&self, prompt: &PromptSlot) -> Option<BundleSlot<LiveAttachments>> {
        self.values
            .iter()
            .find(|value| value.prompt.same_payload(prompt))
            .map(|value| value.attachments.alias())
    }
    pub(super) fn find(
        &self,
        prompt: &PromptSlot,
    ) -> Option<(BundleSlot<LiveAttachments>, Option<BundleSlot<LiveImages>>)> {
        self.values
            .iter()
            .find(|value| value.prompt.same_payload(prompt))
            .map(|value| {
                (
                    value.attachments.alias(),
                    value.images.as_ref().map(BundleSlot::alias),
                )
            })
    }
    /// Drop the removed original prompt's auxiliary ownership; equal numeric ids
    /// of other prompt allocations remain independent.
    pub(super) fn remove(
        &self,
        role: &ConstructionRole,
        prompt: &PromptSlot,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let len = self
            .values
            .iter()
            .filter(|value| !value.prompt.same_payload(prompt))
            .count();
        let mut builder = SlabBuilder::new(role)?;
        let mut values = builder.vector(len)?;
        for value in &self.values {
            if !value.prompt.same_payload(prompt) {
                values.push(value.alias());
            }
        }
        let original = self
            .values
            .first()
            .ok_or(RetentionError::FrozenOwner)?
            .attachments
            .original_credit();
        Ok(BundleSlot::new(BundleOwner::build(
            Self { values },
            builder,
            original,
        )?))
    }
}
impl Body for PromptAuxiliary {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.values)?)?;
        for value in &self.values {
            value.prompt.count(count)?;
            value.attachments.count(count)?;
            if let Some(images) = &value.images {
                images.count(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for value in &self.values {
            value.prompt.cleanup(permit);
            value.attachments.cleanup(permit);
            if let Some(images) = &value.images {
                images.cleanup(permit);
            }
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for value in &self.values {
            value.prompt.adopt(publication);
            value.attachments.adopt(publication);
            if let Some(images) = &value.images {
                images.adopt(publication);
            }
        }
    }
    fn retire(&self) {
        for value in &self.values {
            value.prompt.retire();
            value.attachments.retire();
            if let Some(images) = &value.images {
                images.retire();
            }
        }
    }
}
