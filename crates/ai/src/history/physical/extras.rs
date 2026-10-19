//! Volatile graph edges are shared independently and omitted only from candidates.
use super::{
    bundle::{self, Body, BundleSlot},
    inputs::LiveInputs,
    presentation::LivePresentation,
    slots::{NativeCredit, PhysicalError},
};
use crate::model::{
    json::lifetime::Shared,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
};

pub(super) struct LiveExtras {
    inputs: Option<BundleSlot<LiveInputs>>,
    presentation: Option<BundleSlot<LivePresentation>>,
}
impl LiveExtras {
    pub(super) fn inputs(&self) -> Option<&LiveInputs> {
        self.inputs.as_ref().map(BundleSlot::body)
    }
    pub(super) fn presentation(&self) -> Option<&LivePresentation> {
        self.presentation.as_ref().map(BundleSlot::body)
    }
    pub(super) fn replace_inputs(
        current: Option<&Self>,
        inputs: BundleSlot<LiveInputs>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let original = inputs.original_credit().clone();
        let body = Self {
            inputs: Some(inputs),
            presentation: current
                .and_then(|value| value.presentation.as_ref())
                .map(BundleSlot::alias),
        };
        bundle::part(role, &original, body)
    }
    pub(super) fn replace_presentation(
        current: Option<&Self>,
        presentation: BundleSlot<LivePresentation>,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let body = Self {
            inputs: current
                .and_then(|value| value.inputs.as_ref())
                .map(BundleSlot::alias),
            presentation: Some(presentation),
        };
        bundle::part(role, original, body)
    }
}
impl Body for LiveExtras {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if let Some(value) = &self.inputs {
            value.count(count)?;
        }
        if let Some(value) = &self.presentation {
            value.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(value) = &self.inputs {
            value.cleanup(permit);
        }
        if let Some(value) = &self.presentation {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(value) = &self.inputs {
            value.adopt(publication);
        }
        if let Some(value) = &self.presentation {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        if let Some(value) = &self.inputs {
            value.retire();
        }
        if let Some(value) = &self.presentation {
            value.retire();
        }
    }
}
