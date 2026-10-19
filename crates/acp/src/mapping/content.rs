use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{content::*, diagnostic::ModelError, extensions::Extensions};

pub(super) fn extensions(meta: Option<wire::Meta>) -> Option<Extensions> {
    meta.map(Extensions::from)
}
fn meta(value: Option<Extensions>) -> Option<wire::Meta> {
    value.map(Extensions::into_map)
}
fn annotations(value: Option<wire::Annotations>) -> Result<Option<Annotations>, ModelError> {
    value
        .map(|value| {
            Ok(Annotations {
                audience: value
                    .audience
                    .map(|values| {
                        values
                            .into_iter()
                            .map(|role| match role {
                                wire::Role::User => Ok(Audience::User),
                                wire::Role::Assistant => Ok(Audience::Assistant),
                                _ => Err(ModelError::Unsupported("content audience")),
                            })
                            .collect::<Result<_, _>>()
                    })
                    .transpose()?,
                last_modified: value.last_modified,
                priority: value.priority,
                meta: extensions(value.meta),
            })
        })
        .transpose()
}
pub(super) fn content(value: wire::ContentBlock) -> Result<Content, ModelError> {
    Ok(match value {
        wire::ContentBlock::Text(value) => Content::Text(Text {
            annotations: annotations(value.annotations)?,
            text: value.text,
            meta: extensions(value.meta),
        }),
        wire::ContentBlock::Image(value) => Content::Image(Image {
            annotations: annotations(value.annotations)?,
            data: value.data,
            mime_type: value.mime_type,
            uri: value.uri,
            meta: extensions(value.meta),
        }),
        wire::ContentBlock::Audio(value) => Content::Audio(Audio {
            annotations: annotations(value.annotations)?,
            data: value.data,
            mime_type: value.mime_type,
            meta: extensions(value.meta),
        }),
        wire::ContentBlock::ResourceLink(value) => Content::ResourceReference(ResourceReference {
            annotations: annotations(value.annotations)?,
            description: value.description,
            mime_type: value.mime_type,
            name: value.name,
            size: value.size,
            title: value.title,
            uri: value.uri,
            meta: extensions(value.meta),
        }),
        wire::ContentBlock::Resource(value) => Content::EmbeddedResource(EmbeddedResource {
            annotations: annotations(value.annotations)?,
            resource: match value.resource {
                wire::EmbeddedResourceResource::TextResourceContents(value) => {
                    ResourceBody::Text(ResourceText {
                        mime_type: value.mime_type,
                        text: value.text,
                        uri: value.uri,
                        meta: extensions(value.meta),
                    })
                }
                wire::EmbeddedResourceResource::BlobResourceContents(value) => {
                    ResourceBody::Blob(ResourceBlob {
                        blob: value.blob,
                        mime_type: value.mime_type,
                        uri: value.uri,
                        meta: extensions(value.meta),
                    })
                }
                _ => return Err(ModelError::Unsupported("embedded resource")),
            },
            meta: extensions(value.meta),
        }),
        _ => return Err(ModelError::Unsupported("content")),
    })
}
fn wire_annotations(value: Option<Annotations>) -> Option<wire::Annotations> {
    value.map(|value| {
        wire::Annotations::new()
            .audience(value.audience.map(|values| {
                values
                    .into_iter()
                    .map(|value| match value {
                        Audience::User => wire::Role::User,
                        Audience::Assistant => wire::Role::Assistant,
                    })
                    .collect::<Vec<_>>()
            }))
            .last_modified(value.last_modified)
            .priority(value.priority)
            .meta(meta(value.meta))
    })
}
pub(super) fn wire_content(value: Content) -> wire::ContentBlock {
    match value {
        Content::Text(value) => wire::ContentBlock::Text(
            wire::TextContent::new(value.text)
                .annotations(wire_annotations(value.annotations))
                .meta(meta(value.meta)),
        ),
        Content::Image(value) => wire::ContentBlock::Image(
            wire::ImageContent::new(value.data, value.mime_type)
                .uri(value.uri)
                .annotations(wire_annotations(value.annotations))
                .meta(meta(value.meta)),
        ),
        Content::Audio(value) => wire::ContentBlock::Audio(
            wire::AudioContent::new(value.data, value.mime_type)
                .annotations(wire_annotations(value.annotations))
                .meta(meta(value.meta)),
        ),
        Content::ResourceReference(value) => wire::ContentBlock::ResourceLink(
            wire::ResourceLink::new(value.name, value.uri)
                .title(value.title)
                .description(value.description)
                .mime_type(value.mime_type)
                .size(value.size)
                .annotations(wire_annotations(value.annotations))
                .meta(meta(value.meta)),
        ),
        Content::EmbeddedResource(value) => {
            let resource = match value.resource {
                ResourceBody::Text(body) => wire::EmbeddedResourceResource::TextResourceContents(
                    wire::TextResourceContents::new(body.text, body.uri)
                        .mime_type(body.mime_type)
                        .meta(meta(body.meta)),
                ),
                ResourceBody::Blob(body) => wire::EmbeddedResourceResource::BlobResourceContents(
                    wire::BlobResourceContents::new(body.blob, body.uri)
                        .mime_type(body.mime_type)
                        .meta(meta(body.meta)),
                ),
            };
            wire::ContentBlock::Resource(
                wire::EmbeddedResource::new(resource)
                    .annotations(wire_annotations(value.annotations))
                    .meta(meta(value.meta)),
            )
        }
    }
}
