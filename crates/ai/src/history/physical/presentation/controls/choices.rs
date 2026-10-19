//! Actual configuration choices retain identifier receipts and clearable payloads.
use super::super::super::{bundle::Body, slots::*};
use crate::model::{
    controls::{ConfigChoice, ConfigGroup, SelectChoices},
    extensions::Extensions,
    ids::{ConfigChoiceId, ConfigGroupId},
    retention::{CensusPermit, RetentionError, document::Publication, graph::Count},
};

pub struct ConfigChoiceSlot {
    id: IdSlot<ConfigChoiceId>,
    name: TextSlot,
    description: Option<TextSlot>,
    extensions: Option<ExtensionSlot>,
}
pub struct ConfigGroupSlot {
    id: IdSlot<ConfigGroupId>,
    name: TextSlot,
    choices: Vec<ConfigChoiceSlot>,
    extensions: Option<ExtensionSlot>,
}
pub enum SelectChoicesSlot {
    Flat(Vec<ConfigChoiceSlot>),
    Groups(Vec<ConfigGroupSlot>),
}
impl ConfigChoiceSlot {
    fn new(
        value: ConfigChoice,
        _builder: &mut SlabBuilder<'_>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            id: IdSlot::choice(value.id),
            name: TextSlot::new(value.name, pool)?,
            description: optional_text(value.description, pool)?,
            extensions: value.extensions.map(ExtensionSlot::new),
        })
    }
    pub fn id(&self) -> PayloadGuard<'_, ConfigChoiceId> {
        self.id.read()
    }
    pub fn name(&self) -> TextGuard<'_> {
        self.name.read()
    }
    pub fn description(&self) -> Option<TextGuard<'_>> {
        self.description.as_ref().map(TextSlot::read)
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
impl ConfigGroupSlot {
    fn new(
        value: ConfigGroup,
        builder: &mut SlabBuilder<'_>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            id: IdSlot::group(value.id),
            name: TextSlot::new(value.name, pool)?,
            choices: builder.map_in(value.choices, pool, ConfigChoiceSlot::new)?,
            extensions: value.extensions.map(ExtensionSlot::new),
        })
    }
    pub fn id(&self) -> PayloadGuard<'_, ConfigGroupId> {
        self.id.read()
    }
    pub fn name(&self) -> TextGuard<'_> {
        self.name.read()
    }
    pub fn choices(&self) -> &[ConfigChoiceSlot] {
        &self.choices
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
impl SelectChoicesSlot {
    pub(super) fn new(
        value: SelectChoices,
        builder: &mut SlabBuilder<'_>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(match value {
            SelectChoices::Flat(values) => {
                Self::Flat(builder.map_in(values, pool, ConfigChoiceSlot::new)?)
            }
            SelectChoices::Groups(values) => {
                Self::Groups(builder.map_in(values, pool, ConfigGroupSlot::new)?)
            }
        })
    }
}
impl Body for ConfigChoiceSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.id.count(count)?;
        self.name.count(count)?;
        if let Some(value) = &self.description {
            value.count(count)?;
        }
        self.extensions
            .as_ref()
            .map_or(Ok(()), |value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.id.cleanup(permit);
        self.name.cleanup(permit);
        if let Some(value) = &self.description {
            value.cleanup(permit);
        }
        if let Some(value) = &self.extensions {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.id.adopt(publication);
        self.name.adopt(publication);
        if let Some(value) = &self.description {
            value.adopt(publication);
        }
        if let Some(value) = &self.extensions {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        self.id.retire();
        self.name.retire();
        retire_text(&self.description);
        if let Some(value) = &self.extensions {
            value.retire();
        }
    }
}
impl Body for ConfigGroupSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.id.count(count)?;
        self.name.count(count)?;
        count.add_exclusive(vector_bytes(&self.choices)?)?;
        self.choices
            .iter()
            .try_for_each(|value| value.count(count))?;
        self.extensions
            .as_ref()
            .map_or(Ok(()), |value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.id.cleanup(permit);
        self.name.cleanup(permit);
        self.choices.iter().for_each(|value| value.cleanup(permit));
        if let Some(value) = &self.extensions {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.id.adopt(publication);
        self.name.adopt(publication);
        self.choices
            .iter()
            .for_each(|value| value.adopt(publication));
        if let Some(value) = &self.extensions {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        self.id.retire();
        self.name.retire();
        self.choices.iter().for_each(Body::retire);
        if let Some(value) = &self.extensions {
            value.retire();
        }
    }
}
impl Body for SelectChoicesSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        match self {
            Self::Flat(values) => {
                count.add_exclusive(vector_bytes(values)?)?;
                values.iter().try_for_each(|value| value.count(count))
            }
            Self::Groups(values) => {
                count.add_exclusive(vector_bytes(values)?)?;
                values.iter().try_for_each(|value| value.count(count))
            }
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        match self {
            Self::Flat(values) => values.iter().for_each(|value| value.cleanup(permit)),
            Self::Groups(values) => values.iter().for_each(|value| value.cleanup(permit)),
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        match self {
            Self::Flat(values) => values.iter().for_each(|value| value.adopt(publication)),
            Self::Groups(values) => values.iter().for_each(|value| value.adopt(publication)),
        }
    }
    fn retire(&self) {
        match self {
            Self::Flat(values) => values.iter().for_each(Body::retire),
            Self::Groups(values) => values.iter().for_each(Body::retire),
        }
    }
}
