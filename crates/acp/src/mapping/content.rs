use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{content::*, diagnostic::ModelError, extensions::Extensions};
pub(super) fn extensions(
    authority: &RetentionAuthority,
    meta: Option<wire::Meta>,
) -> Result<Option<Extensions>, ModelError> {
    meta.map(|meta| Extensions::checked(authority, meta))
        .transpose()
}
fn annotations(
    authority: &RetentionAuthority,
    value: Option<wire::Annotations>,
) -> Result<Option<Annotations>, ModelError> {
    value
        .map(|value| {
            Ok(Annotations {
                audience: value
                    .audience
                    .map(|values| {
                        values
                            .into_iter()
                            .map(|value| match value {
                                wire::Role::User => Ok(Audience::User),
                                wire::Role::Assistant => Ok(Audience::Assistant),
                                _ => Err(ModelError::Unsupported("content audience")),
                            })
                            .collect::<Result<_, _>>()
                    })
                    .transpose()?,
                modified: value.last_modified,
                priority: value.priority,
                extensions: extensions(authority, value.meta)?,
            })
        })
        .transpose()
}
pub(super) fn inbound(
    authority: &RetentionAuthority,
    value: wire::ContentBlock,
) -> Result<Content, ModelError> {
    Ok(match value {
        wire::ContentBlock::Text(value) => Content::Text(Text {
            text: value.text,
            annotations: annotations(authority, value.annotations)?,
            extensions: extensions(authority, value.meta)?,
        }),
        wire::ContentBlock::Image(value) => Content::Media(Media {
            kind: MediaKind::Image,
            encoded: value.data,
            mime: value.mime_type,
            uri: value.uri,
            annotations: annotations(authority, value.annotations)?,
            extensions: extensions(authority, value.meta)?,
        }),
        wire::ContentBlock::Audio(value) => Content::Media(Media {
            kind: MediaKind::Audio,
            encoded: value.data,
            mime: value.mime_type,
            uri: None,
            annotations: annotations(authority, value.annotations)?,
            extensions: extensions(authority, value.meta)?,
        }),
        wire::ContentBlock::ResourceLink(value) => Content::ResourceReference(ResourceReference {
            uri: value.uri,
            name: value.name,
            title: value.title,
            description: value.description,
            mime: value.mime_type,
            reported_size: value.size,
            annotations: annotations(authority, value.annotations)?,
            extensions: extensions(authority, value.meta)?,
        }),
        wire::ContentBlock::Resource(value) => Content::EmbeddedResource(EmbeddedResource {
            body: match value.resource {
                wire::EmbeddedResourceResource::TextResourceContents(value) => {
                    ResourceBody::Text(ResourceText {
                        uri: value.uri,
                        text: value.text,
                        mime: value.mime_type,
                        extensions: extensions(authority, value.meta)?,
                    })
                }
                wire::EmbeddedResourceResource::BlobResourceContents(value) => {
                    ResourceBody::Encoded(ResourceBlob {
                        uri: value.uri,
                        encoded: value.blob,
                        mime: value.mime_type,
                        extensions: extensions(authority, value.meta)?,
                    })
                }
                _ => return Err(ModelError::Unsupported("embedded resource")),
            },
            annotations: annotations(authority, value.annotations)?,
            extensions: extensions(authority, value.meta)?,
        }),
        _ => return Err(ModelError::Unsupported("content")),
    })
}
fn metadata(value: Option<Extensions>) -> Option<wire::Meta> {
    value.map(|value| {
        value
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
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
            .last_modified(value.modified)
            .priority(value.priority)
            .meta(metadata(value.extensions))
    })
}
pub(super) fn outbound(value: Content) -> Result<wire::ContentBlock, ModelError> {
    Ok(match value {
        Content::Text(value) => wire::ContentBlock::Text(
            wire::TextContent::new(value.text)
                .annotations(wire_annotations(value.annotations))
                .meta(metadata(value.extensions)),
        ),
        Content::Media(value) => match value.kind {
            MediaKind::Image => wire::ContentBlock::Image(
                wire::ImageContent::new(value.encoded, value.mime)
                    .uri(value.uri)
                    .annotations(wire_annotations(value.annotations))
                    .meta(metadata(value.extensions)),
            ),
            MediaKind::Audio => {
                if value.uri.is_some() {
                    return Err(ModelError::Unsupported("audio source URI"));
                }
                wire::ContentBlock::Audio(
                    wire::AudioContent::new(value.encoded, value.mime)
                        .annotations(wire_annotations(value.annotations))
                        .meta(metadata(value.extensions)),
                )
            }
        },
        Content::ResourceReference(value) => wire::ContentBlock::ResourceLink(
            wire::ResourceLink::new(value.name, value.uri)
                .title(value.title)
                .description(value.description)
                .mime_type(value.mime)
                .size(value.reported_size)
                .annotations(wire_annotations(value.annotations))
                .meta(metadata(value.extensions)),
        ),
        Content::EmbeddedResource(value) => {
            let body = match value.body {
                ResourceBody::Text(body) => wire::EmbeddedResourceResource::TextResourceContents(
                    wire::TextResourceContents::new(body.text, body.uri)
                        .mime_type(body.mime)
                        .meta(metadata(body.extensions)),
                ),
                ResourceBody::Encoded(body) => {
                    wire::EmbeddedResourceResource::BlobResourceContents(
                        wire::BlobResourceContents::new(body.encoded, body.uri)
                            .mime_type(body.mime)
                            .meta(metadata(body.extensions)),
                    )
                }
            };
            wire::ContentBlock::Resource(
                wire::EmbeddedResource::new(body)
                    .annotations(wire_annotations(value.annotations))
                    .meta(metadata(value.extensions)),
            )
        }
    })
}
