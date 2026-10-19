//! ACP content grammar is projected directly into prepaid neutral components.
use super::{error::DecodeError, value::Value};
use nocterm_ai::model::{
    content::*,
    extensions::Extensions,
    json::deferred::JsonDecodeError,
    native::{NativeList, NativeOwned},
};
type Error = DecodeError<JsonDecodeError>;

const CONTENT_FIELDS: [&str; 12] = [
    "type",
    "annotations",
    "text",
    "data",
    "mimeType",
    "uri",
    "name",
    "title",
    "description",
    "size",
    "resource",
    "_meta",
];
pub(super) fn content(value: Value<'_>) -> Result<NativeOwned<Content>, Error> {
    if !value.raw().starts_with('{') {
        return Err(DecodeError::Field("content object"));
    }
    let [
        kind,
        annotations,
        text,
        data,
        mime,
        uri,
        name,
        title,
        description,
        size,
        resource,
        meta,
    ] = value.fields(CONTENT_FIELDS)?;
    let kind = Value::required(kind, "content.type")?;
    let annotations = optional_annotations(annotations, &value)?;
    let extensions = extensions(meta, &value)?;
    if kind.equals("text")? {
        Ok(nocterm_ai::model::native::content::from_text(
            nocterm_ai::model::native::content::text_content(
                Value::required(text, "text")?.text()?,
                annotations,
                extensions,
            )?,
        ))
    } else if kind.equals("image")? || kind.equals("audio")? {
        let image = kind.equals("image")?;
        Ok(nocterm_ai::model::native::content::from_media(
            nocterm_ai::model::native::content::media(
                if image {
                    MediaKind::Image
                } else {
                    MediaKind::Audio
                },
                Value::required(data, "data")?.text()?,
                Value::required(mime, "mimeType")?.text()?,
                if image {
                    optional_text(uri, &value)?
                } else {
                    NativeOwned::<String>::absent(value.ledger())?
                },
                annotations,
                extensions,
            )?,
        ))
    } else if kind.equals("resource_link")? {
        let size = size.and_then(|size| size.raw().parse::<i64>().ok());
        Ok(nocterm_ai::model::native::content::from_reference(
            nocterm_ai::model::native::content::reference(
                (
                    Value::required(uri, "uri")?.text()?,
                    Value::required(name, "name")?.text()?,
                ),
                optional_text(title, &value)?,
                optional_text(description, &value)?,
                optional_text(mime, &value)?,
                size,
                annotations,
                extensions,
            )?,
        ))
    } else if kind.equals("resource")? {
        Ok(nocterm_ai::model::native::content::from_embedded(
            nocterm_ai::model::native::content::embedded(
                resource_body(Value::required(resource, "resource")?)?,
                annotations,
                extensions,
            )?,
        ))
    } else {
        Err(DecodeError::Field("supported content type"))
    }
}
fn resource_body(value: Value<'_>) -> Result<NativeOwned<ResourceBody>, Error> {
    // The untagged schema tries text before encoded, with each variant's own
    // positional field order. Inspect shape before allocating either branch.
    match value.fields(["mimeType", "text", "uri", "_meta"]) {
        Ok([mime, Some(text), Some(uri), meta])
            if text.raw().starts_with('"') && uri.raw().starts_with('"') =>
        {
            return Ok(nocterm_ai::model::native::content::text_body(
                nocterm_ai::model::native::content::resource_text(
                    uri.text()?,
                    text.text()?,
                    optional_text(mime, &value)?,
                    extensions(meta, &value)?,
                )?,
            ));
        }
        Ok(_) | Err(DecodeError::Field(_)) => {}
        Err(error) => return Err(error),
    }
    let [blob, mime, uri, meta] = value.fields(["blob", "mimeType", "uri", "_meta"])?;
    Ok(nocterm_ai::model::native::content::blob_body(
        nocterm_ai::model::native::content::resource_blob(
            Value::required(uri, "resource.uri")?.text()?,
            Value::required(blob, "resource.blob")?.text()?,
            optional_text(mime, &value)?,
            extensions(meta, &value)?,
        )?,
    ))
}
pub(super) fn optional_text(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<NativeOwned<Option<String>>, Error> {
    match value.filter(|value| value.raw().starts_with('"')) {
        Some(value) => Ok(NativeOwned::supplied(value.text()?)),
        None => Ok(NativeOwned::<String>::absent(fallback.ledger())?),
    }
}
pub(super) fn extensions(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<NativeOwned<Option<Extensions>>, Error> {
    match value.filter(|value| value.raw().starts_with('{')) {
        Some(value) => Ok(NativeOwned::supplied(
            nocterm_ai::model::native::content::extensions(value.literal()?, value.ledger())?,
        )),
        None => Ok(NativeOwned::<Extensions>::absent(fallback.ledger())?),
    }
}
fn optional_annotations(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<NativeOwned<Option<Annotations>>, Error> {
    match value.filter(|value| value.raw().starts_with('{') || value.raw().starts_with('[')) {
        Some(value) => match annotations(value) {
            Ok(value) => Ok(NativeOwned::supplied(value)),
            Err(DecodeError::Field(_) | DecodeError::Syntax(_)) => {
                Ok(NativeOwned::<Annotations>::absent(fallback.ledger())?)
            }
            Err(error) => Err(error),
        },
        None => Ok(NativeOwned::<Annotations>::absent(fallback.ledger())?),
    }
}
fn annotations(value: Value<'_>) -> Result<NativeOwned<Annotations>, Error> {
    let [audience, modified, priority, meta] =
        value.fields(["audience", "lastModified", "priority", "_meta"])?;
    let audience = match audience.filter(|value| value.raw().starts_with('[')) {
        Some(audience) => {
            // VecSkipError retains only actual known roles. Count them before
            // allocating the final typed vector, without retaining DTO copies.
            let mut count = 0;
            let mut items = audience.elements()?;
            while let Some(item) = items.next()? {
                if item.equals("user")? || item.equals("assistant")? {
                    count += 1;
                }
            }
            let mut result = NativeList::new(value.ledger(), count)?;
            let mut items = audience.elements()?;
            while let Some(item) = items.next()? {
                let role = if item.equals("user")? {
                    Some(Audience::User)
                } else if item.equals("assistant")? {
                    Some(Audience::Assistant)
                } else {
                    None
                };
                if let Some(role) = role {
                    result.push(nocterm_ai::model::native::content::audience(
                        role,
                        value.ledger(),
                    )?)?;
                }
            }
            NativeOwned::supplied(result.finish())
        }
        None => NativeOwned::<Vec<Audience>>::absent(value.ledger())?,
    };
    let priority = priority
        .and_then(|value| value.raw().parse::<f64>().ok())
        .filter(|value| value.is_finite());
    Ok(nocterm_ai::model::native::content::annotations(
        audience,
        optional_text(modified, &value)?,
        priority,
        extensions(meta, &value)?,
    )?)
}

/// Schema shape is checked before a skipped-list allocates any final payload.
/// Optional DefaultOnError fields cannot invalidate an otherwise valid block.
pub(super) fn shape(value: Value<'_>) -> Result<(), Error> {
    if !value.raw().starts_with('{') {
        return Err(DecodeError::Field("content object"));
    }
    let [kind, text, data, mime, uri, name, resource] = value.fields([
        "type", "text", "data", "mimeType", "uri", "name", "resource",
    ])?;
    let kind = Value::required(kind, "content.type")?;
    fn required_text(value: Option<Value<'_>>) -> Result<(), Error> {
        if Value::required(value, "content text")?
            .raw()
            .starts_with('"')
        {
            Ok(())
        } else {
            Err(DecodeError::Field("content text"))
        }
    }
    if kind.equals("text")? {
        required_text(text)?;
    } else if kind.equals("image")? || kind.equals("audio")? {
        required_text(data)?;
        required_text(mime)?;
    } else if kind.equals("resource_link")? {
        required_text(uri)?;
        required_text(name)?;
    } else if kind.equals("resource")? {
        let resource = Value::required(resource, "resource")?;
        let text_shape =
            match super::lists::shape(resource.fields(["mimeType", "text", "uri", "_meta"]))? {
                Some([_, Some(text), Some(uri), _]) => {
                    text.raw().starts_with('"') && uri.raw().starts_with('"')
                }
                _ => false,
            };
        if !text_shape {
            let [blob, _mime, uri, _meta] =
                resource.fields(["blob", "mimeType", "uri", "_meta"])?;
            required_text(blob)?;
            required_text(uri)?;
        }
    } else {
        return Err(DecodeError::Field("content type"));
    }
    Ok(())
}
