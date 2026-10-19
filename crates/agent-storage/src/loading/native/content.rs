//! All neutral content and annotation branches consume guarded field owners.
use super::{
    Error, fields as f, lists,
    schema::{self as s, Scope},
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key::*,
};
use nocterm_ai::model::{
    content::{
        Annotations as AnnotationsValue, Audience as AudienceValue, Content as ContentValue,
        EmbeddedResource, Media, MediaKind, ResourceBody, ResourceReference, Text as TextValue,
    },
    json::deferred::DecodeLedger,
    native::NativeOwned,
};
pub(super) fn content(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ContentValue>, Error> {
    let kind = f::tag(plan, held, ledger)?;
    match kind.as_str() {
        "text" => {
            let p = f::child(plan, held, Value, s::TEXT, ledger)?;
            Ok(nocterm_ai::model::native::content::from_text(text(
                &p, held, ledger,
            )?))
        }
        "media" => {
            let p = f::child(plan, held, Value, s::MEDIA, ledger)?;
            Ok(nocterm_ai::model::native::content::from_media(media(
                &p, held, ledger,
            )?))
        }
        "resource_reference" => {
            let p = f::child(plan, held, Value, s::REFERENCE, ledger)?;
            Ok(nocterm_ai::model::native::content::from_reference(
                reference(&p, held, ledger)?,
            ))
        }
        "embedded_resource" => {
            let p = f::child(plan, held, Value, s::EMBEDDED, ledger)?;
            Ok(nocterm_ai::model::native::content::from_embedded(embedded(
                &p, held, ledger,
            )?))
        }
        _ => Err(Error::Variant),
    }
}
fn annotations(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: crate::loading::json_fields::Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<AnnotationsValue>, Error> {
    let p = f::child(plan, held, key, s::ANNOTATIONS, ledger)?;
    let audience = f::optional(&p, held, Audience, ledger, |p, h, k, l| {
        lists::primitive(p, h, k, l, usize::MAX, |node, l| {
            let audience = match node.text() {
                Some("user") => AudienceValue::User,
                Some("assistant") => AudienceValue::Assistant,
                _ => return Err(Error::Variant),
            };
            Ok(nocterm_ai::model::native::content::audience(audience, l)?)
        })
    })?;
    let modified = f::optional_text(&p, held, Modified, ledger)?;
    let priority = f::optional_number(&p, held, Priority, ledger, |n| n.as_f64())?;
    let extensions = f::extensions(&p, held, Extensions, ledger)?;
    Ok(nocterm_ai::model::native::content::annotations(
        audience, modified, priority, extensions,
    )?)
}
fn common(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<
    (
        NativeOwned<Option<AnnotationsValue>>,
        NativeOwned<Option<nocterm_ai::model::extensions::Extensions>>,
    ),
    Error,
> {
    Ok((
        f::optional(plan, held, Annotations, ledger, annotations)?,
        f::extensions(plan, held, Extensions, ledger)?,
    ))
}
fn text(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<TextValue>, Error> {
    let text = f::text(plan, held, Text, ledger)?;
    let (annotations, extensions) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::text_content(
        text,
        annotations,
        extensions,
    )?)
}
fn media(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Media>, Error> {
    let kind = f::text(plan, held, Kind, ledger)?;
    let kind = match kind.as_str() {
        "image" => MediaKind::Image,
        "audio" => MediaKind::Audio,
        _ => return Err(Error::Variant),
    };
    let encoded = f::text(plan, held, Encoded, ledger)?;
    let mime = f::text(plan, held, Mime, ledger)?;
    let uri = f::optional_text(plan, held, Uri, ledger)?;
    let (annotations, extensions) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::media(
        kind,
        encoded,
        mime,
        uri,
        annotations,
        extensions,
    )?)
}
fn reference(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ResourceReference>, Error> {
    let uri = f::text(plan, held, Uri, ledger)?;
    let name = f::text(plan, held, Name, ledger)?;
    let title = f::optional_text(plan, held, Title, ledger)?;
    let description = f::optional_text(plan, held, Description, ledger)?;
    let mime = f::optional_text(plan, held, Mime, ledger)?;
    let size = f::optional_number(plan, held, ReportedSize, ledger, |n| n.as_i64())?;
    let (annotations, extensions) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::reference(
        (uri, name),
        title,
        description,
        mime,
        size,
        annotations,
        extensions,
    )?)
}
fn embedded(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<EmbeddedResource>, Error> {
    let body = f::child(plan, held, Body, s::TAG, ledger)?;
    let body = resource(&body, held, ledger)?;
    let (annotations, extensions) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::embedded(
        body,
        annotations,
        extensions,
    )?)
}
fn resource(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ResourceBody>, Error> {
    let kind = f::tag(plan, held, ledger)?;
    let schema = match kind.as_str() {
        "text" => s::RESOURCE_TEXT,
        "encoded" => s::RESOURCE_BLOB,
        _ => return Err(Error::Variant),
    };
    let p = f::child(plan, held, Value, schema, ledger)?;
    let uri = f::text(&p, held, Uri, ledger)?;
    let payload = f::text(
        &p,
        held,
        if kind.as_str() == "text" {
            Text
        } else {
            Encoded
        },
        ledger,
    )?;
    let mime = f::optional_text(&p, held, Mime, ledger)?;
    let extensions = f::extensions(&p, held, Extensions, ledger)?;
    if kind.as_str() == "text" {
        Ok(nocterm_ai::model::native::content::text_body(
            nocterm_ai::model::native::content::resource_text(uri, payload, mime, extensions)?,
        ))
    } else {
        Ok(nocterm_ai::model::native::content::blob_body(
            nocterm_ai::model::native::content::resource_blob(uri, payload, mime, extensions)?,
        ))
    }
}
