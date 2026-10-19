//! Borrowed physical content projects into paid native request values without DTO clones.
use super::{NativeCopies, NativeList, NativeOwned};
use crate::{
    history::physical::{AnnotationSlot, ContentSlot, ResourceSlot},
    model::{content::*, json::deferred::JsonDecodeError},
};

pub(super) struct PhysicalContentCopies<'a, 'l> {
    copies: &'a NativeCopies<'l>,
}
impl<'a, 'l> PhysicalContentCopies<'a, 'l> {
    pub(super) fn new(copies: &'a NativeCopies<'l>) -> Self {
        Self { copies }
    }
    pub(super) fn copy(
        &self,
        value: &ContentSlot,
    ) -> Result<NativeOwned<Content>, JsonDecodeError> {
        let c = self.copies;
        let annotations = self.annotations(value.annotations())?;
        let extensions = c.extensions(value.extensions().as_deref())?;
        match value {
            ContentSlot::Text(value) => Ok(crate::model::native::content::from_text(
                crate::model::native::content::text_content(
                    c.text(&value.text())?,
                    annotations,
                    extensions,
                )?,
            )),
            ContentSlot::Media(value) => Ok(crate::model::native::content::from_media(
                crate::model::native::content::media(
                    value.kind(),
                    c.text(&value.encoded())?,
                    c.text(&value.mime())?,
                    c.optional_text(value.uri().as_deref())?,
                    annotations,
                    extensions,
                )?,
            )),
            ContentSlot::Reference(value) => Ok(crate::model::native::content::from_reference(
                crate::model::native::content::reference(
                    (c.text(&value.uri())?, c.text(&value.name())?),
                    c.optional_text(value.title().as_deref())?,
                    c.optional_text(value.description().as_deref())?,
                    c.optional_text(value.mime().as_deref())?,
                    value.reported_size(),
                    annotations,
                    extensions,
                )?,
            )),
            ContentSlot::Embedded(value) => Ok(crate::model::native::content::from_embedded(
                crate::model::native::content::embedded(
                    self.body(value.body())?,
                    annotations,
                    extensions,
                )?,
            )),
        }
    }
    fn body(&self, value: &ResourceSlot) -> Result<NativeOwned<ResourceBody>, JsonDecodeError> {
        let c = self.copies;
        match value {
            ResourceSlot::Text(value) => Ok(crate::model::native::content::text_body(
                crate::model::native::content::resource_text(
                    c.text(&value.uri())?,
                    c.text(&value.text())?,
                    c.optional_text(value.mime().as_deref())?,
                    c.extensions(value.extensions().as_deref())?,
                )?,
            )),
            ResourceSlot::Encoded(value) => Ok(crate::model::native::content::blob_body(
                crate::model::native::content::resource_blob(
                    c.text(&value.uri())?,
                    c.text(&value.encoded())?,
                    c.optional_text(value.mime().as_deref())?,
                    c.extensions(value.extensions().as_deref())?,
                )?,
            )),
        }
    }
    fn annotations(
        &self,
        value: Option<&AnnotationSlot>,
    ) -> Result<NativeOwned<Option<Annotations>>, JsonDecodeError> {
        let c = self.copies;
        let Some(value) = value else {
            return NativeOwned::absent(c.ledger);
        };
        let audience = match value.audience() {
            Some(values) => {
                let mut output = NativeList::new(c.ledger, values.len())?;
                for value in values {
                    output.push(crate::model::native::content::audience(*value, c.ledger)?)?;
                }
                NativeOwned::supplied(output.finish())
            }
            None => NativeOwned::absent(c.ledger)?,
        };
        Ok(NativeOwned::supplied(
            crate::model::native::content::annotations(
                audience,
                c.optional_text(value.modified().as_deref())?,
                value.priority(),
                c.extensions(value.extensions().as_deref())?,
            )?,
        ))
    }
}
