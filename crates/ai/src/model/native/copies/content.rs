//! Copies every neutral content variant through its original named constructors.
use super::{NativeCopies, NativeList, NativeOwned};
use crate::model::{content::*, json::deferred::JsonDecodeError};
pub(super) struct ContentCopies<'a, 'l> {
    copies: &'a NativeCopies<'l>,
}
impl<'a, 'l> ContentCopies<'a, 'l> {
    pub(super) fn new(copies: &'a NativeCopies<'l>) -> Self {
        Self { copies }
    }
    pub(super) fn copy(&self, value: &Content) -> Result<NativeOwned<Content>, JsonDecodeError> {
        let c = self.copies;
        match value {
            Content::Text(value) => Ok(crate::model::native::content::from_text(
                crate::model::native::content::text_content(
                    c.text(&value.text)?,
                    self.annotations(value.annotations.as_ref())?,
                    c.extensions(value.extensions.as_ref())?,
                )?,
            )),
            Content::Media(value) => Ok(crate::model::native::content::from_media(
                crate::model::native::content::media(
                    value.kind,
                    c.text(&value.encoded)?,
                    c.text(&value.mime)?,
                    c.optional_text(value.uri.as_deref())?,
                    self.annotations(value.annotations.as_ref())?,
                    c.extensions(value.extensions.as_ref())?,
                )?,
            )),
            Content::ResourceReference(value) => Ok(crate::model::native::content::from_reference(
                crate::model::native::content::reference(
                    (c.text(&value.uri)?, c.text(&value.name)?),
                    c.optional_text(value.title.as_deref())?,
                    c.optional_text(value.description.as_deref())?,
                    c.optional_text(value.mime.as_deref())?,
                    value.reported_size,
                    self.annotations(value.annotations.as_ref())?,
                    c.extensions(value.extensions.as_ref())?,
                )?,
            )),
            Content::EmbeddedResource(value) => Ok(crate::model::native::content::from_embedded(
                crate::model::native::content::embedded(
                    self.body(&value.body)?,
                    self.annotations(value.annotations.as_ref())?,
                    c.extensions(value.extensions.as_ref())?,
                )?,
            )),
        }
    }
    fn body(&self, value: &ResourceBody) -> Result<NativeOwned<ResourceBody>, JsonDecodeError> {
        let c = self.copies;
        match value {
            ResourceBody::Text(value) => Ok(crate::model::native::content::text_body(
                crate::model::native::content::resource_text(
                    c.text(&value.uri)?,
                    c.text(&value.text)?,
                    c.optional_text(value.mime.as_deref())?,
                    c.extensions(value.extensions.as_ref())?,
                )?,
            )),
            ResourceBody::Encoded(value) => Ok(crate::model::native::content::blob_body(
                crate::model::native::content::resource_blob(
                    c.text(&value.uri)?,
                    c.text(&value.encoded)?,
                    c.optional_text(value.mime.as_deref())?,
                    c.extensions(value.extensions.as_ref())?,
                )?,
            )),
        }
    }
    fn annotations(
        &self,
        value: Option<&Annotations>,
    ) -> Result<NativeOwned<Option<Annotations>>, JsonDecodeError> {
        let c = self.copies;
        let Some(value) = value else {
            return NativeOwned::absent(c.ledger);
        };
        let audience = match &value.audience {
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
                c.optional_text(value.modified.as_deref())?,
                value.priority,
                c.extensions(value.extensions.as_ref())?,
            )?,
        ))
    }
}
