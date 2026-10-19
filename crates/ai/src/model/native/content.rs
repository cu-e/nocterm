//! Concrete content construction consumes text, collections and opaque JSON.
use super::NativeOwned;
use crate::model::{
    content::*,
    extensions::Extensions,
    json::{
        OwnedJson,
        deferred::{DecodeLedger, JsonDecodeError, native::NativeNode},
    },
};

pub fn null_json(ledger: &DecodeLedger) -> Result<NativeOwned<OwnedJson>, JsonDecodeError> {
    let mut builder =
        crate::model::json::deferred::literal::LiteralBuilder::with_cleanup_frontier(ledger, 1)?;
    builder.null()?;
    let value = builder.finish()?.into_owned(ledger)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn json(
    node: NativeNode,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<OwnedJson>, JsonDecodeError> {
    let value = node.into_decoded(ledger)?.into_owned(ledger)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn extensions(
    node: NativeNode,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<Extensions>, JsonDecodeError> {
    let value = Extensions::from_decoded(node.into_decoded(ledger)?, ledger)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn text_content(
    text: NativeOwned<String>,
    annotations: NativeOwned<Option<Annotations>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<Text>, JsonDecodeError> {
    let (((text, annotations), extensions), allocation) =
        text.join(annotations)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        Text {
            text,
            annotations,
            extensions,
        },
        allocation,
    ))
}
pub fn from_text(owned: NativeOwned<Text>) -> NativeOwned<Content> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Content::Text(value), allocation)
}
pub fn media(
    kind: MediaKind,
    encoded: NativeOwned<String>,
    mime: NativeOwned<String>,
    uri: NativeOwned<Option<String>>,
    annotations: NativeOwned<Option<Annotations>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<Media>, JsonDecodeError> {
    let (((((encoded, mime), uri), annotations), extensions), allocation) = encoded
        .join(mime)?
        .join(uri)?
        .join(annotations)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        Media {
            kind,
            encoded,
            mime,
            uri,
            annotations,
            extensions,
        },
        allocation,
    ))
}
pub fn from_media(owned: NativeOwned<Media>) -> NativeOwned<Content> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Content::Media(value), allocation)
}
pub fn reference(
    identity: (NativeOwned<String>, NativeOwned<String>),
    title: NativeOwned<Option<String>>,
    description: NativeOwned<Option<String>>,
    mime: NativeOwned<Option<String>>,
    reported_size: Option<i64>,
    annotations: NativeOwned<Option<Annotations>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ResourceReference>, JsonDecodeError> {
    let (((((((uri, name), title), description), mime), annotations), extensions), allocation) =
        identity
            .0
            .join(identity.1)?
            .join(title)?
            .join(description)?
            .join(mime)?
            .join(annotations)?
            .join(extensions)?
            .into_parts();
    Ok(NativeOwned::from_parts(
        ResourceReference {
            uri,
            name,
            title,
            description,
            mime,
            reported_size,
            annotations,
            extensions,
        },
        allocation,
    ))
}
pub fn from_reference(owned: NativeOwned<ResourceReference>) -> NativeOwned<Content> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Content::ResourceReference(value), allocation)
}
pub fn resource_text(
    uri: NativeOwned<String>,
    text: NativeOwned<String>,
    mime: NativeOwned<Option<String>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ResourceText>, JsonDecodeError> {
    let ((((uri, text), mime), extensions), allocation) =
        uri.join(text)?.join(mime)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        ResourceText {
            uri,
            text,
            mime,
            extensions,
        },
        allocation,
    ))
}
pub fn text_body(owned: NativeOwned<ResourceText>) -> NativeOwned<ResourceBody> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(ResourceBody::Text(value), allocation)
}
pub fn resource_blob(
    uri: NativeOwned<String>,
    encoded: NativeOwned<String>,
    mime: NativeOwned<Option<String>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ResourceBlob>, JsonDecodeError> {
    let ((((uri, encoded), mime), extensions), allocation) = uri
        .join(encoded)?
        .join(mime)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ResourceBlob {
            uri,
            encoded,
            mime,
            extensions,
        },
        allocation,
    ))
}
pub fn blob_body(owned: NativeOwned<ResourceBlob>) -> NativeOwned<ResourceBody> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(ResourceBody::Encoded(value), allocation)
}
pub fn embedded(
    body: NativeOwned<ResourceBody>,
    annotations: NativeOwned<Option<Annotations>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<EmbeddedResource>, JsonDecodeError> {
    let (((body, annotations), extensions), allocation) =
        body.join(annotations)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        EmbeddedResource {
            body,
            annotations,
            extensions,
        },
        allocation,
    ))
}
pub fn from_embedded(owned: NativeOwned<EmbeddedResource>) -> NativeOwned<Content> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Content::EmbeddedResource(value), allocation)
}
pub fn annotations(
    audience: NativeOwned<Option<Vec<Audience>>>,
    modified: NativeOwned<Option<String>>,
    priority: Option<f64>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<Annotations>, JsonDecodeError> {
    let (((audience, modified), extensions), allocation) =
        audience.join(modified)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        Annotations {
            audience,
            modified,
            priority,
            extensions,
        },
        allocation,
    ))
}
pub fn audience(
    value: Audience,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<Audience>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
