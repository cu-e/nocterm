//! Actual typed content storage. Skeleton vectors survive weak projection;
//! their clearable text/JSON payloads end with the genuine payload token.
use super::{
    bundle::{self, Body, BundleSlot},
    slots::*,
    source_pool::RetainedBacking,
};
use crate::model::{
    content::*,
    extensions::Extensions,
    json::lifetime::Shared,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
};

impl ContentSlot {
    pub fn wire(&self) -> impl serde::Serialize + '_ {
        super::wire::content_value(self)
    }
}

impl Body for ContentSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        ContentSlot::count(self, count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        ContentSlot::cleanup(self, permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        ContentSlot::adopt(self, publication);
    }
    fn retire(&self) {
        ContentSlot::retire(self);
    }
}

pub enum ContentSlot {
    Text(TextContent),
    Media(MediaContent),
    Reference(ReferenceContent),
    Embedded(EmbeddedContent),
}
pub struct TextContent {
    text: BundleSlot<TextSlot>,
    annotations: Option<AnnotationSlot>,
    extensions: Option<ExtensionSlot>,
}
pub struct MediaContent {
    kind: MediaKind,
    encoded: TextSlot,
    mime: TextSlot,
    uri: Option<TextSlot>,
    annotations: Option<AnnotationSlot>,
    extensions: Option<ExtensionSlot>,
}
pub struct ReferenceContent {
    uri: TextSlot,
    name: TextSlot,
    title: Option<TextSlot>,
    description: Option<TextSlot>,
    mime: Option<TextSlot>,
    reported_size: Option<i64>,
    annotations: Option<AnnotationSlot>,
    extensions: Option<ExtensionSlot>,
}
pub struct EmbeddedContent {
    body: ResourceSlot,
    annotations: Option<AnnotationSlot>,
    extensions: Option<ExtensionSlot>,
}
pub enum ResourceSlot {
    Text(ResourceTextSlot),
    Encoded(ResourceBlobSlot),
}
pub struct ResourceTextSlot {
    uri: TextSlot,
    text: TextSlot,
    mime: Option<TextSlot>,
    extensions: Option<ExtensionSlot>,
}
pub struct ResourceBlobSlot {
    uri: TextSlot,
    encoded: TextSlot,
    mime: Option<TextSlot>,
    extensions: Option<ExtensionSlot>,
}
pub struct AnnotationSlot {
    audience: Option<RetainedBacking<Vec<Audience>>>,
    modified: Option<TextSlot>,
    priority: Option<f64>,
    extensions: Option<ExtensionSlot>,
}
impl AnnotationSlot {
    fn new(value: Annotations, pool: &mut NativeSourcePool) -> Result<Self, PhysicalError> {
        Ok(Self {
            audience: value
                .audience
                .map(|value| {
                    let bytes = vector_bytes(&value)?;
                    pool.retained(value, bytes)
                })
                .transpose()?,
            modified: optional_text(value.modified, pool)?,
            priority: value.priority,
            extensions: value.extensions.map(ExtensionSlot::new),
        })
    }
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if let Some(audience) = &self.audience {
            count.add_exclusive(vector_bytes(audience.get())?)?;
            audience.count_metadata(count)?;
        }
        if let Some(value) = &self.modified {
            value.count(count)?;
        }
        if let Some(value) = &self.extensions {
            value.count(count)?;
        }
        Ok(())
    }
    fn retire(&self) {
        retire_text(&self.modified);
        if let Some(value) = &self.extensions {
            value.retire();
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(value) = &self.audience {
            value.cleanup(permit);
        }
        if let Some(value) = &self.modified {
            value.cleanup(permit);
        }
        if let Some(value) = &self.extensions {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(value) = &self.audience {
            value.adopt(publication);
        }
        if let Some(value) = &self.modified {
            value.adopt(publication);
        }
        if let Some(value) = &self.extensions {
            value.adopt(publication);
        }
    }
}
fn same_text(left: &Option<TextSlot>, right: &Option<String>) -> bool {
    left.as_ref().map(TextSlot::read).as_deref() == right.as_deref()
}
pub(super) fn same_extensions(left: &Option<ExtensionSlot>, right: &Option<Extensions>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => *left.read() == *right,
        _ => false,
    }
}
fn same_annotations(left: &Option<AnnotationSlot>, right: &Option<Annotations>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.audience.as_ref().map(RetainedBacking::get) == right.audience.as_ref()
                && same_text(&left.modified, &right.modified)
                && left.priority == right.priority
                && same_extensions(&left.extensions, &right.extensions)
        }
        _ => false,
    }
}
impl PartialEq<Content> for ContentSlot {
    fn eq(&self, right: &Content) -> bool {
        match (self, right) {
            (Self::Text(left), Content::Text(right)) => left == right,
            (Self::Media(left), Content::Media(right)) => left == right,
            (Self::Reference(left), Content::ResourceReference(right)) => left == right,
            (Self::Embedded(left), Content::EmbeddedResource(right)) => left == right,
            _ => false,
        }
    }
}
impl PartialEq<Text> for TextContent {
    fn eq(&self, right: &Text) -> bool {
        *self.text.body().read() == right.text
            && same_annotations(&self.annotations, &right.annotations)
            && same_extensions(&self.extensions, &right.extensions)
    }
}
impl PartialEq<Media> for MediaContent {
    fn eq(&self, right: &Media) -> bool {
        self.kind == right.kind
            && *self.encoded.read() == right.encoded
            && *self.mime.read() == right.mime
            && same_text(&self.uri, &right.uri)
            && same_annotations(&self.annotations, &right.annotations)
            && same_extensions(&self.extensions, &right.extensions)
    }
}
impl PartialEq<ResourceReference> for ReferenceContent {
    fn eq(&self, right: &ResourceReference) -> bool {
        *self.uri.read() == right.uri
            && *self.name.read() == right.name
            && same_text(&self.title, &right.title)
            && same_text(&self.description, &right.description)
            && same_text(&self.mime, &right.mime)
            && self.reported_size == right.reported_size
            && same_annotations(&self.annotations, &right.annotations)
            && same_extensions(&self.extensions, &right.extensions)
    }
}
impl PartialEq<EmbeddedResource> for EmbeddedContent {
    fn eq(&self, right: &EmbeddedResource) -> bool {
        self.body == right.body
            && same_annotations(&self.annotations, &right.annotations)
            && same_extensions(&self.extensions, &right.extensions)
    }
}
impl PartialEq<ResourceBody> for ResourceSlot {
    fn eq(&self, right: &ResourceBody) -> bool {
        match (self, right) {
            (Self::Text(left), ResourceBody::Text(right)) => {
                *left.uri.read() == right.uri
                    && *left.text.read() == right.text
                    && same_text(&left.mime, &right.mime)
                    && same_extensions(&left.extensions, &right.extensions)
            }
            (Self::Encoded(left), ResourceBody::Encoded(right)) => {
                *left.uri.read() == right.uri
                    && *left.encoded.read() == right.encoded
                    && same_text(&left.mime, &right.mime)
                    && same_extensions(&left.extensions, &right.extensions)
            }
            _ => false,
        }
    }
}
impl ContentSlot {
    pub(super) fn new(
        value: Content,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(match value {
            Content::Text(value) => Self::Text(TextContent {
                text: bundle::part(role, original, TextSlot::new(value.text, pool)?)?,
                annotations: value
                    .annotations
                    .map(|value| AnnotationSlot::new(value, pool))
                    .transpose()?,
                extensions: value.extensions.map(ExtensionSlot::new),
            }),
            Content::Media(value) => Self::Media(MediaContent {
                kind: value.kind,
                encoded: TextSlot::new(value.encoded, pool)?,
                mime: TextSlot::new(value.mime, pool)?,
                uri: optional_text(value.uri, pool)?,
                annotations: value
                    .annotations
                    .map(|value| AnnotationSlot::new(value, pool))
                    .transpose()?,
                extensions: value.extensions.map(ExtensionSlot::new),
            }),
            Content::ResourceReference(value) => Self::Reference(ReferenceContent {
                uri: TextSlot::new(value.uri, pool)?,
                name: TextSlot::new(value.name, pool)?,
                title: optional_text(value.title, pool)?,
                description: optional_text(value.description, pool)?,
                mime: optional_text(value.mime, pool)?,
                reported_size: value.reported_size,
                annotations: value
                    .annotations
                    .map(|value| AnnotationSlot::new(value, pool))
                    .transpose()?,
                extensions: value.extensions.map(ExtensionSlot::new),
            }),
            Content::EmbeddedResource(value) => Self::Embedded(EmbeddedContent {
                body: match value.body {
                    ResourceBody::Text(value) => ResourceSlot::Text(ResourceTextSlot {
                        uri: TextSlot::new(value.uri, pool)?,
                        text: TextSlot::new(value.text, pool)?,
                        mime: optional_text(value.mime, pool)?,
                        extensions: value.extensions.map(ExtensionSlot::new),
                    }),
                    ResourceBody::Encoded(value) => ResourceSlot::Encoded(ResourceBlobSlot {
                        uri: TextSlot::new(value.uri, pool)?,
                        encoded: TextSlot::new(value.encoded, pool)?,
                        mime: optional_text(value.mime, pool)?,
                        extensions: value.extensions.map(ExtensionSlot::new),
                    }),
                },
                annotations: value
                    .annotations
                    .map(|value| AnnotationSlot::new(value, pool))
                    .transpose()?,
                extensions: value.extensions.map(ExtensionSlot::new),
            }),
        })
    }
    pub(super) fn from_prompt_text(text: BundleSlot<TextSlot>) -> Self {
        Self::Text(TextContent {
            text,
            annotations: None,
            extensions: None,
        })
    }
    fn parts(&self) -> (Option<&AnnotationSlot>, Option<&ExtensionSlot>) {
        match self {
            Self::Text(v) => (v.annotations.as_ref(), v.extensions.as_ref()),
            Self::Media(v) => (v.annotations.as_ref(), v.extensions.as_ref()),
            Self::Reference(v) => (v.annotations.as_ref(), v.extensions.as_ref()),
            Self::Embedded(v) => (v.annotations.as_ref(), v.extensions.as_ref()),
        }
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        match self {
            Self::Text(value) => value.text.count(count)?,
            Self::Media(value) => {
                value.encoded.count(count)?;
                value.mime.count(count)?;
                if let Some(value) = &value.uri {
                    value.count(count)?;
                }
            }
            Self::Reference(value) => {
                value.uri.count(count)?;
                value.name.count(count)?;
                for value in [&value.title, &value.description, &value.mime]
                    .into_iter()
                    .flatten()
                {
                    value.count(count)?;
                }
            }
            Self::Embedded(value) => match &value.body {
                ResourceSlot::Text(value) => {
                    value.uri.count(count)?;
                    value.text.count(count)?;
                    if let Some(value) = &value.mime {
                        value.count(count)?;
                    }
                    if let Some(value) = &value.extensions {
                        value.count(count)?;
                    }
                }
                ResourceSlot::Encoded(value) => {
                    value.uri.count(count)?;
                    value.encoded.count(count)?;
                    if let Some(value) = &value.mime {
                        value.count(count)?;
                    }
                    if let Some(value) = &value.extensions {
                        value.count(count)?;
                    }
                }
            },
        }
        let (annotations, extensions) = self.parts();
        if let Some(v) = annotations {
            v.count(count)?;
        }
        if let Some(v) = extensions {
            v.count(count)?;
        }
        Ok(())
    }
    pub(super) fn retire(&self) {
        match self {
            Self::Text(v) => v.text.retire(),
            Self::Media(v) => {
                v.encoded.retire();
                v.mime.retire();
                retire_text(&v.uri);
            }
            Self::Reference(v) => {
                v.uri.retire();
                v.name.retire();
                retire_text(&v.title);
                retire_text(&v.description);
                retire_text(&v.mime);
            }
            Self::Embedded(v) => match &v.body {
                ResourceSlot::Text(v) => {
                    v.uri.retire();
                    v.text.retire();
                    retire_text(&v.mime);
                    if let Some(e) = &v.extensions {
                        e.retire();
                    }
                }
                ResourceSlot::Encoded(v) => {
                    v.uri.retire();
                    v.encoded.retire();
                    retire_text(&v.mime);
                    if let Some(e) = &v.extensions {
                        e.retire();
                    }
                }
            },
        }
        let (annotations, extensions) = self.parts();
        if let Some(v) = annotations {
            v.retire();
        }
        if let Some(v) = extensions {
            v.retire();
        }
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        match self {
            Self::Text(value) => value.text.cleanup(permit),
            Self::Media(value) => {
                value.encoded.cleanup(permit);
                value.mime.cleanup(permit);
                if let Some(value) = &value.uri {
                    value.cleanup(permit);
                }
            }
            Self::Reference(value) => {
                value.uri.cleanup(permit);
                value.name.cleanup(permit);
                for value in [&value.title, &value.description, &value.mime]
                    .into_iter()
                    .flatten()
                {
                    value.cleanup(permit);
                }
            }
            Self::Embedded(value) => match &value.body {
                ResourceSlot::Text(value) => {
                    value.uri.cleanup(permit);
                    value.text.cleanup(permit);
                    if let Some(value) = &value.mime {
                        value.cleanup(permit);
                    }
                }
                ResourceSlot::Encoded(value) => {
                    value.uri.cleanup(permit);
                    value.encoded.cleanup(permit);
                    if let Some(value) = &value.mime {
                        value.cleanup(permit);
                    }
                }
            },
        }
        let (annotations, extensions) = self.parts();
        if let Some(v) = annotations {
            v.cleanup(permit);
        }
        if let Some(v) = extensions {
            v.cleanup(permit);
        }
        if let Self::Embedded(v) = self {
            match &v.body {
                ResourceSlot::Text(v) => {
                    if let Some(e) = &v.extensions {
                        e.cleanup(permit);
                    }
                }
                ResourceSlot::Encoded(v) => {
                    if let Some(e) = &v.extensions {
                        e.cleanup(permit);
                    }
                }
            }
        }
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        match self {
            Self::Text(value) => value.text.adopt(publication),
            Self::Media(value) => {
                value.encoded.adopt(publication);
                value.mime.adopt(publication);
                if let Some(value) = &value.uri {
                    value.adopt(publication);
                }
            }
            Self::Reference(value) => {
                value.uri.adopt(publication);
                value.name.adopt(publication);
                for value in [&value.title, &value.description, &value.mime]
                    .into_iter()
                    .flatten()
                {
                    value.adopt(publication);
                }
            }
            Self::Embedded(value) => match &value.body {
                ResourceSlot::Text(value) => {
                    value.uri.adopt(publication);
                    value.text.adopt(publication);
                    if let Some(value) = &value.mime {
                        value.adopt(publication);
                    }
                }
                ResourceSlot::Encoded(value) => {
                    value.uri.adopt(publication);
                    value.encoded.adopt(publication);
                    if let Some(value) = &value.mime {
                        value.adopt(publication);
                    }
                }
            },
        }
        let (annotations, extensions) = self.parts();
        if let Some(v) = annotations {
            v.adopt(publication);
        }
        if let Some(v) = extensions {
            v.adopt(publication);
        }
        if let Self::Embedded(v) = self {
            match &v.body {
                ResourceSlot::Text(v) => {
                    if let Some(e) = &v.extensions {
                        e.adopt(publication);
                    }
                }
                ResourceSlot::Encoded(v) => {
                    if let Some(e) = &v.extensions {
                        e.adopt(publication);
                    }
                }
            }
        }
    }
}
impl TextContent {
    pub fn text(&self) -> TextGuard<'_> {
        self.text.body().read()
    }
}
impl MediaContent {
    pub fn kind(&self) -> MediaKind {
        self.kind
    }
    pub fn encoded(&self) -> TextGuard<'_> {
        self.encoded.read()
    }
    pub fn mime(&self) -> TextGuard<'_> {
        self.mime.read()
    }
}

impl ContentSlot {
    pub fn annotations(&self) -> Option<&AnnotationSlot> {
        self.parts().0
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.parts().1.map(ExtensionSlot::read)
    }
}
impl MediaContent {
    pub fn uri(&self) -> Option<TextGuard<'_>> {
        self.uri.as_ref().map(TextSlot::read)
    }
}
impl ReferenceContent {
    pub fn uri(&self) -> TextGuard<'_> {
        self.uri.read()
    }
    pub fn name(&self) -> TextGuard<'_> {
        self.name.read()
    }
    pub fn title(&self) -> Option<TextGuard<'_>> {
        self.title.as_ref().map(TextSlot::read)
    }
    pub fn description(&self) -> Option<TextGuard<'_>> {
        self.description.as_ref().map(TextSlot::read)
    }
    pub fn mime(&self) -> Option<TextGuard<'_>> {
        self.mime.as_ref().map(TextSlot::read)
    }
    pub fn reported_size(&self) -> Option<i64> {
        self.reported_size
    }
}
impl EmbeddedContent {
    pub fn body(&self) -> &ResourceSlot {
        &self.body
    }
}
impl ResourceTextSlot {
    pub fn uri(&self) -> TextGuard<'_> {
        self.uri.read()
    }
    pub fn text(&self) -> TextGuard<'_> {
        self.text.read()
    }
    pub fn mime(&self) -> Option<TextGuard<'_>> {
        self.mime.as_ref().map(TextSlot::read)
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
impl ResourceBlobSlot {
    pub fn uri(&self) -> TextGuard<'_> {
        self.uri.read()
    }
    pub fn encoded(&self) -> TextGuard<'_> {
        self.encoded.read()
    }
    pub fn mime(&self) -> Option<TextGuard<'_>> {
        self.mime.as_ref().map(TextSlot::read)
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
impl AnnotationSlot {
    pub(super) fn audience_capacity(&self) -> Option<usize> {
        self.audience.as_ref().map(|value| value.get().capacity())
    }
    pub fn audience(&self) -> Option<&[Audience]> {
        self.audience.as_ref().map(|value| value.get().as_slice())
    }
    pub fn modified(&self) -> Option<TextGuard<'_>> {
        self.modified.as_ref().map(TextSlot::read)
    }
    pub fn priority(&self) -> Option<f64> {
        self.priority
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
