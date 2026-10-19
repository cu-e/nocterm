//! Current binary and encoded image owners alias independently into prompt payloads.
use super::super::{
    ContentSlot,
    bundle::{self, Body, BundleOwner, BundleSlot},
    slots::{
        NativeCredit, NativeSource, NativeSourcePool, PhysicalError, SlabBuilder, vector_bytes,
    },
};
use crate::{
    history::FormatSize,
    images::{
        ImageFacts,
        managed::{BinaryProjection, ManagedImageBytes, PreparedPromptImage},
    },
    model::{
        json::lifetime::Shared,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::{Count, OwnerGraph},
        },
    },
};
use std::{
    ops::Deref,
    sync::{RwLock, RwLockReadGuard},
};

pub struct ManagedImage {
    binary: BundleSlot<BinarySlot>,
    content: BundleSlot<ContentSlot>,
    facts: ImageFacts,
    size: FormatSize,
}
struct BinarySlot {
    value: RwLock<Option<ManagedImageBytes>>,
    projection: BinaryProjection,
}
pub struct ImageBinaryGuard<'a>(RwLockReadGuard<'a, Option<ManagedImageBytes>>);
impl Deref for ImageBinaryGuard<'_> {
    type Target = ManagedImageBytes;
    fn deref(&self) -> &ManagedImageBytes {
        self.0.as_ref().expect("real image owner pins binary")
    }
}
impl ManagedImage {
    fn new(
        image: PreparedPromptImage,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let (binary, media, facts, size) = image.into_parts();
        let projection = binary.projection();
        let binary = bundle::part(
            role,
            original,
            BinarySlot {
                value: RwLock::new(Some(binary)),
                projection,
            },
        )?;
        let content = bundle::part(
            role,
            original,
            ContentSlot::new(
                crate::model::content::Content::Media(media),
                role,
                original,
                pool,
            )?,
        )?;
        Ok(Self {
            binary,
            content,
            facts,
            size,
        })
    }
    pub fn binary(&self) -> ImageBinaryGuard<'_> {
        ImageBinaryGuard(
            self.binary
                .body()
                .value
                .read()
                .expect("immutable binary slot"),
        )
    }
    pub fn content(&self) -> &ContentSlot {
        self.content.body()
    }
    pub fn facts(&self) -> ImageFacts {
        self.facts
    }
    pub fn encoded_size(&self) -> FormatSize {
        self.size
    }
    pub fn same_allocation(&self, other: &Self) -> bool {
        std::ptr::eq(self.binary.body(), other.binary.body())
    }
    pub(in crate::history::physical) fn alias(&self) -> Self {
        Self {
            binary: self.binary.alias(),
            content: self.content.alias(),
            facts: self.facts,
            size: self.size,
        }
    }
    pub(in crate::history::physical) fn content_alias(&self) -> BundleSlot<ContentSlot> {
        self.content.alias()
    }
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.binary.count(count)?;
        self.content.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.binary.cleanup(permit);
        self.content.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.binary.adopt(publication);
        self.content.adopt(publication);
    }
    fn retire(&self) {
        self.binary.retire();
        self.content.retire();
    }
}
impl Body for BinarySlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.projection.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.projection.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.projection.adopt(publication);
    }
    fn retire(&self) {
        drop(self.value.write().expect("immutable binary slot").take());
    }
}
pub(super) struct LiveImages {
    values: Vec<ManagedImage>,
}
impl LiveImages {
    pub(super) fn append(
        previous: Option<&Self>,
        role: &ConstructionRole,
        value: crate::model::native::NativeOwned<PreparedPromptImage>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        if !value.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let previous = previous.map_or(&[][..], Self::values);
        let bytes =
            previous
                .iter()
                .try_fold(value.get().binary().bytes().len(), |sum, image| {
                    sum.checked_add(image.binary().bytes().len())
                        .ok_or(crate::images::ImageCollectionError)
                })?;
        if previous.len() >= crate::images::MAX_IMAGES
            || bytes > crate::images::MAX_TOTAL_IMAGE_BYTES
        {
            return Err(crate::images::ImageCollectionError.into());
        }
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let image = ManagedImage::new(source.take(), role, &original, source.pool())?;
        let mut builder = SlabBuilder::new(role)?;
        let mut values = builder.vector(previous.len() + 1)?;
        values.extend(previous.iter().map(ManagedImage::alias));
        values.push(image);
        Ok(BundleSlot::new(BundleOwner::build(
            Self { values },
            builder,
            &original,
        )?))
    }
    pub(super) fn without(
        &self,
        role: &ConstructionRole,
        index: usize,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let mut values = builder.vector(self.values.len() - 1)?;
        values.extend(
            self.values
                .iter()
                .enumerate()
                .filter(|(position, _)| *position != index)
                .map(|(_, value)| value.alias()),
        );
        Ok(BundleSlot::new(BundleOwner::build(
            Self { values },
            builder,
            original,
        )?))
    }
    pub(super) fn values(&self) -> &[ManagedImage] {
        &self.values
    }
    pub(super) fn build(
        role: &ConstructionRole,
        values: Vec<PreparedPromptImage>,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(values, pool, |value, builder, pool| {
            ManagedImage::new(value, builder.role(), original, pool)
        })?;
        Ok(BundleSlot::new(BundleOwner::build(
            Self { values },
            builder,
            original,
        )?))
    }
}
impl Body for LiveImages {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.values)?)?;
        self.values.iter().try_for_each(|value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.values.iter().for_each(|value| value.cleanup(permit));
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.values
            .iter()
            .for_each(|value| value.adopt(publication));
    }
    fn retire(&self) {
        self.values.iter().for_each(ManagedImage::retire);
    }
}
