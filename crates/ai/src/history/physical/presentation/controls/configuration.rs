//! Supplied configuration options become physical fields once, without DTO mirrors.
use super::super::super::{bundle::Body, slots::*};
use super::choices::SelectChoicesSlot;
use crate::model::{
    controls::{ConfigCategory, ConfigKind, ConfigOption},
    extensions::Extensions,
    ids::{ConfigChoiceId, ConfigId},
    retention::{CensusPermit, RetentionError, document::Publication, graph::Count},
};
pub struct ConfigOptionSlot {
    id: IdSlot<ConfigId>,
    name: TextSlot,
    description: Option<TextSlot>,
    category: Option<CategorySlot>,
    kind: ConfigKindSlot,
    extensions: Option<ExtensionSlot>,
}
pub enum CategorySlot {
    Mode,
    Model,
    ModelConfig,
    ThoughtLevel,
    Other(TextSlot),
}
pub enum ConfigKindSlot {
    Boolean(bool),
    Select {
        current: ConfigSelectionSlot,
        choices: SelectChoicesSlot,
    },
    Unsupported(TextSlot),
}
pub struct ConfigSelectionSlot(IdSlot<ConfigChoiceId>);
impl ConfigSelectionSlot {
    pub fn read(&self) -> PayloadGuard<'_, ConfigChoiceId> {
        self.0.read()
    }
}
pub(super) struct ConfigurationSlot(pub(super) Vec<ConfigOptionSlot>);
impl ConfigOptionSlot {
    pub(super) fn new(
        value: ConfigOption,
        builder: &mut SlabBuilder<'_>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let category = value
            .category
            .map(|category| -> Result<_, PhysicalError> {
                Ok(match category {
                    ConfigCategory::Mode => CategorySlot::Mode,
                    ConfigCategory::Model => CategorySlot::Model,
                    ConfigCategory::ModelConfig => CategorySlot::ModelConfig,
                    ConfigCategory::ThoughtLevel => CategorySlot::ThoughtLevel,
                    ConfigCategory::Other(value) => {
                        CategorySlot::Other(TextSlot::new(value, pool)?)
                    }
                })
            })
            .transpose()?;
        let kind = match value.kind {
            ConfigKind::Boolean(value) => ConfigKindSlot::Boolean(value),
            ConfigKind::Unsupported { label } => {
                ConfigKindSlot::Unsupported(TextSlot::new(label, pool)?)
            }
            ConfigKind::Select { current, choices } => ConfigKindSlot::Select {
                current: ConfigSelectionSlot(IdSlot::choice(current)),
                choices: SelectChoicesSlot::new(choices, builder, pool)?,
            },
        };
        Ok(Self {
            id: IdSlot::config(value.id),
            name: TextSlot::new(value.name, pool)?,
            description: optional_text(value.description, pool)?,
            category,
            kind,
            extensions: value.extensions.map(ExtensionSlot::new),
        })
    }
    pub fn id(&self) -> PayloadGuard<'_, ConfigId> {
        self.id.read()
    }
    pub fn name(&self) -> TextGuard<'_> {
        self.name.read()
    }
    pub fn description(&self) -> Option<TextGuard<'_>> {
        self.description.as_ref().map(TextSlot::read)
    }
    pub fn category(&self) -> Option<&CategorySlot> {
        self.category.as_ref()
    }
    pub fn kind(&self) -> &ConfigKindSlot {
        &self.kind
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
impl Body for ConfigOptionSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.id.count(count)?;
        self.name.count(count)?;
        if let Some(value) = &self.description {
            value.count(count)?;
        }
        if let Some(CategorySlot::Other(value)) = &self.category {
            value.count(count)?;
        }
        match &self.kind {
            ConfigKindSlot::Select { current, choices } => {
                current.0.count(count)?;
                choices.count(count)?;
            }
            ConfigKindSlot::Unsupported(value) => value.count(count)?,
            ConfigKindSlot::Boolean(_) => {}
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
        if let Some(CategorySlot::Other(value)) = &self.category {
            value.cleanup(permit);
        }
        if let ConfigKindSlot::Unsupported(value) = &self.kind {
            value.cleanup(permit);
        }
        if let ConfigKindSlot::Select { current, choices } = &self.kind {
            current.0.cleanup(permit);
            choices.cleanup(permit);
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
        if let Some(CategorySlot::Other(value)) = &self.category {
            value.adopt(publication);
        }
        if let ConfigKindSlot::Unsupported(value) = &self.kind {
            value.adopt(publication);
        }
        if let ConfigKindSlot::Select { current, choices } = &self.kind {
            current.0.adopt(publication);
            choices.adopt(publication);
        }
        if let Some(value) = &self.extensions {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        self.id.retire();
        self.name.retire();
        retire_text(&self.description);
        if let Some(CategorySlot::Other(value)) = &self.category {
            value.retire();
        }
        match &self.kind {
            ConfigKindSlot::Select { current, choices } => {
                current.0.retire();
                choices.retire();
            }
            ConfigKindSlot::Unsupported(value) => value.retire(),
            ConfigKindSlot::Boolean(_) => {}
        }
        if let Some(value) = &self.extensions {
            value.retire();
        }
    }
}
impl Body for ConfigurationSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.0)?)?;
        self.0.iter().try_for_each(|value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.0.iter().for_each(|value| value.cleanup(permit));
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.0.iter().for_each(|value| value.adopt(publication));
    }
    fn retire(&self) {
        self.0.iter().for_each(Body::retire);
    }
}
