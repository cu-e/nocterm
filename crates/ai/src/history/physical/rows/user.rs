//! The actual User row: every content item is an independent aliasable bundle,
//! so appending a chunk shares all earlier items with the previous row version.
use super::super::{
    bundle::{self, BundleSlot},
    content::ContentSlot,
    slots::*,
};
use crate::model::{
    content::Content,
    json::lifetime::Shared,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
};

pub struct UserSlot {
    items: Vec<BundleSlot<ContentSlot>>,
}
impl UserSlot {
    pub(super) fn new(
        values: Vec<Content>,
        builder: &mut SlabBuilder<'_>,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let items = builder.map_in(values, pool, |value, builder, pool| {
            bundle::part(
                builder.role(),
                original,
                ContentSlot::new(value, builder.role(), original, pool)?,
            )
        })?;
        Ok(Self { items })
    }
    pub(super) fn from_prompt(
        prompt: &super::super::PromptSlot,
        builder: &mut SlabBuilder<'_>,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            items: prompt.user_items(builder)?,
        })
    }
    /// A new version aliasing every existing item, followed by `item`.
    pub(super) fn appended(
        &self,
        item: BundleSlot<ContentSlot>,
        builder: &mut SlabBuilder<'_>,
    ) -> Result<Self, PhysicalError> {
        let len = self
            .items
            .len()
            .checked_add(1)
            .ok_or(RetentionError::Arithmetic)?;
        let mut items = builder.vector(len)?;
        items.extend(self.items.iter().map(BundleSlot::alias));
        items.push(item);
        Ok(Self { items })
    }
    pub(super) fn single(
        item: BundleSlot<ContentSlot>,
        builder: &mut SlabBuilder<'_>,
    ) -> Result<Self, PhysicalError> {
        let mut items = builder.vector(1)?;
        items.push(item);
        Ok(Self { items })
    }
    pub(in crate::history::physical) fn capacity(&self) -> usize {
        self.items.capacity()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn contents(&self) -> impl ExactSizeIterator<Item = &ContentSlot> + Clone {
        self.items.iter().map(BundleSlot::body)
    }
}

impl UserSlot {
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.items)?)?;
        self.items.iter().try_for_each(|item| item.count(count))
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.items.iter().for_each(|item| item.cleanup(permit));
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.items.iter().for_each(|item| item.adopt(publication));
    }
    pub(super) fn retire(&self) {
        self.items.iter().for_each(BundleSlot::retire);
    }
}
/// Builds one appended content item owned by its own native credit.
pub(super) fn item(
    role: &ConstructionRole,
    value: Content,
    original: &Shared<NativeCredit>,
    pool: &mut NativeSourcePool,
) -> Result<BundleSlot<ContentSlot>, PhysicalError> {
    bundle::part(
        role,
        original,
        ContentSlot::new(value, role, original, pool)?,
    )
}
