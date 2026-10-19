//! Independently shared metadata strings; scalar edits never copy unchanged text.
use super::super::{
    bundle::{self, Body, BundleSlot},
    slots::{NativeCredit, NativeSource, NativeSourcePool, PhysicalError, TextGuard, TextSlot},
};
use crate::{
    history::ChatMetadata,
    model::{
        json::lifetime::Shared,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
};

#[derive(Clone, Copy)]
enum MetadataText {
    Title,
    Name,
    Model,
}

pub struct MetadataSlot {
    agent_id: BundleSlot<TextSlot>,
    title: Option<BundleSlot<TextSlot>>,
    name: Option<BundleSlot<TextSlot>>,
    pinned: bool,
    updated: u64,
    model: Option<BundleSlot<TextSlot>>,
}
impl MetadataSlot {
    pub(in crate::history::physical) fn replacing_title(
        &self,
        value: crate::model::native::NativeOwned<Option<String>>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        self.replacing_text(value, role, MetadataText::Title)
    }
    pub(in crate::history::physical) fn replacing_model(
        &self,
        value: crate::model::native::NativeOwned<Option<String>>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        self.replacing_text(value, role, MetadataText::Model)
    }
    pub(in crate::history::physical) fn renaming(
        &self,
        value: crate::model::native::NativeOwned<Option<String>>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        self.replacing_text(value, role, MetadataText::Name)
    }
    fn replacing_text(
        &self,
        value: crate::model::native::NativeOwned<Option<String>>,
        role: &ConstructionRole,
        target: MetadataText,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut body = self.alias();
        let value = text(source.take(), role, &original, source.pool())?;
        match target {
            MetadataText::Title => body.title = value,
            MetadataText::Name => body.name = value,
            MetadataText::Model => body.model = value,
        }
        bundle::part(role, &original, body)
    }

    pub(in crate::history::physical) fn stamped(
        &self,
        updated: u64,
        pinned: bool,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut body = self.alias();
        body.timestamp(updated);
        body.pin(pinned);
        bundle::part(role, original, body)
    }
    pub(in crate::history::physical) fn new(
        value: ChatMetadata,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            agent_id: bundle::part(role, original, TextSlot::new(value.agent_id, pool)?)?,
            title: text(value.title, role, original, pool)?,
            name: text(value.name, role, original, pool)?,
            pinned: value.pinned,
            updated: value.updated,
            model: text(value.model, role, original, pool)?,
        })
    }
    pub fn agent_id(&self) -> TextGuard<'_> {
        self.agent_id.body().read()
    }
    pub fn title(&self) -> Option<TextGuard<'_>> {
        self.title.as_ref().map(|value| value.body().read())
    }
    pub fn name(&self) -> Option<TextGuard<'_>> {
        self.name.as_ref().map(|value| value.body().read())
    }
    pub fn model(&self) -> Option<TextGuard<'_>> {
        self.model.as_ref().map(|value| value.body().read())
    }
    pub fn pinned(&self) -> bool {
        self.pinned
    }
    pub fn updated(&self) -> u64 {
        self.updated
    }
    pub(in crate::history::physical) fn alias(&self) -> Self {
        Self {
            agent_id: self.agent_id.alias(),
            title: self.title.as_ref().map(BundleSlot::alias),
            name: self.name.as_ref().map(BundleSlot::alias),
            pinned: self.pinned,
            updated: self.updated,
            model: self.model.as_ref().map(BundleSlot::alias),
        }
    }
    pub(in crate::history::physical) fn timestamp(&mut self, updated: u64) {
        self.updated = updated;
    }
    pub(in crate::history::physical) fn pin(&mut self, pinned: bool) {
        self.pinned = pinned;
    }
}
fn text(
    value: Option<String>,
    role: &ConstructionRole,
    original: &Shared<NativeCredit>,
    pool: &mut NativeSourcePool,
) -> Result<Option<BundleSlot<TextSlot>>, PhysicalError> {
    value
        .map(|value| bundle::part(role, original, TextSlot::new(value, pool)?))
        .transpose()
}
impl Body for MetadataSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.agent_id.count(count)?;
        for value in [&self.title, &self.name, &self.model].into_iter().flatten() {
            value.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.agent_id.cleanup(permit);
        for value in [&self.title, &self.name, &self.model].into_iter().flatten() {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.agent_id.adopt(publication);
        for value in [&self.title, &self.name, &self.model].into_iter().flatten() {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        self.agent_id.retire();
        for value in [&self.title, &self.name, &self.model].into_iter().flatten() {
            value.retire();
        }
    }
}
