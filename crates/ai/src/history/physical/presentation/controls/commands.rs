//! Advertised commands own clearable physical strings, independently of configuration.
use super::super::super::{bundle::Body, slots::*};
use crate::model::{
    notice::AdvertisedCommand,
    retention::{CensusPermit, RetentionError, document::Publication, graph::Count},
};
pub struct CommandSlot {
    name: TextSlot,
    description: TextSlot,
    input_hint: Option<TextSlot>,
}
pub(super) struct CommandsSlot(pub(super) Vec<CommandSlot>);
impl CommandSlot {
    pub(super) fn new(
        value: AdvertisedCommand,
        _builder: &mut SlabBuilder<'_>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            name: TextSlot::new(value.name, pool)?,
            description: TextSlot::new(value.description, pool)?,
            input_hint: optional_text(value.input_hint, pool)?,
        })
    }
    pub fn name(&self) -> TextGuard<'_> {
        self.name.read()
    }
    pub fn description(&self) -> TextGuard<'_> {
        self.description.read()
    }
    pub fn input_hint(&self) -> Option<TextGuard<'_>> {
        self.input_hint.as_ref().map(TextSlot::read)
    }
}
impl Body for CommandsSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.0)?)?;
        for command in &self.0 {
            command.name.count(count)?;
            command.description.count(count)?;
            if let Some(value) = &command.input_hint {
                value.count(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for command in &self.0 {
            command.name.cleanup(permit);
            command.description.cleanup(permit);
            if let Some(value) = &command.input_hint {
                value.cleanup(permit);
            }
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for command in &self.0 {
            command.name.adopt(publication);
            command.description.adopt(publication);
            if let Some(value) = &command.input_hint {
                value.adopt(publication);
            }
        }
    }
    fn retire(&self) {
        for command in &self.0 {
            command.name.retire();
            command.description.retire();
            retire_text(&command.input_hint);
        }
    }
}
