//! Original SDK content fields and Text-first embedded-resource selection.
use super::{Error, extensions, f, fallback, optional, optional_text, raw_node, resource_text};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key::*,
    native::{
        lists,
        schema::{self as s, Field, Scope},
    },
};
use nocterm_ai::model::{
    content::{
        Annotations as AnnotationsValue, Audience as AudienceValue, Content as ContentValue,
        MediaKind, ResourceBody,
    },
    json::deferred::DecodeLedger,
    native::NativeOwned,
};
pub(in crate::loading::native) fn content(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ContentValue>, Error> {
    let tag = f::text(plan, held, Type, ledger)?;
    match tag.as_str() {
        "text" => text(plan, held, ledger),
        "image" => media(plan, held, ledger, MediaKind::Image),
        "audio" => media(plan, held, ledger, MediaKind::Audio),
        "resource_link" => reference(plan, held, ledger),
        "resource" => embedded(plan, held, ledger),
        _ => Err(Error::Variant),
    }
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
        optional(
            plan,
            held,
            Annotations,
            ledger,
            s::LEGACY_ANNOTATIONS,
            annotations,
        )?,
        extensions(plan, held, Meta, ledger)?,
    ))
}
fn text(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ContentValue>, Error> {
    plan.payload()
        .validate(&[Type, Text, Annotations, Meta], &[Type, Text])?;
    let text = f::text(plan, held, Text, ledger)?;
    let (a, e) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::from_text(
        nocterm_ai::model::native::content::text_content(text, a, e)?,
    ))
}
fn media(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
    kind: MediaKind,
) -> std::result::Result<NativeOwned<ContentValue>, Error> {
    let keys = if kind == MediaKind::Image {
        &[Type, Data, MimeType, Uri, Annotations, Meta][..]
    } else {
        &[Type, Data, MimeType, Annotations, Meta][..]
    };
    plan.payload().validate(keys, &[Type, Data, MimeType])?;
    let data = f::text(plan, held, Data, ledger)?;
    let mime = f::text(plan, held, MimeType, ledger)?;
    let uri = if kind == MediaKind::Image {
        optional_text(plan, held, Uri, ledger)?
    } else {
        NativeOwned::<String>::absent(ledger)?
    };
    let (a, e) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::from_media(
        nocterm_ai::model::native::content::media(kind, data, mime, uri, a, e)?,
    ))
}
fn reference(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ContentValue>, Error> {
    plan.payload().validate(
        &[
            Type,
            Uri,
            Name,
            Title,
            Description,
            MimeType,
            Size,
            Annotations,
            Meta,
        ],
        &[Type, Uri, Name],
    )?;
    let uri = f::text(plan, held, Uri, ledger)?;
    let name = f::text(plan, held, Name, ledger)?;
    let title = optional_text(plan, held, Title, ledger)?;
    let description = optional_text(plan, held, Description, ledger)?;
    let mime = optional_text(plan, held, MimeType, ledger)?;
    let size = fallback(
        f::optional_number(plan, held, Size, ledger, |v| v.as_i64()),
        || Ok(None),
    )?;
    let (a, e) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::from_reference(
        nocterm_ai::model::native::content::reference(
            (uri, name),
            title,
            description,
            mime,
            size,
            a,
            e,
        )?,
    ))
}
fn embedded(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ContentValue>, Error> {
    plan.payload()
        .validate(&[Type, Resource, Annotations, Meta], &[Type, Resource])?;
    if f::starts(plan, held, Resource, b'{')? {
        lists::validate_members(
            plan,
            held,
            Resource,
            ledger,
            Some(&[Uri, MimeType, Meta, Text, Blob]),
            |p, h, member, l| {
                super::super::legacy_value::authenticated(
                    p,
                    h,
                    |_| member.native_span(),
                    l,
                    false,
                )??;
                Ok(())
            },
        )?;
    }
    let p = f::child(plan, held, Resource, s::LEGACY_RESOURCE, ledger)?;
    let body = resource(&p, held, ledger)?;
    let (a, e) = common(plan, held, ledger)?;
    Ok(nocterm_ai::model::native::content::from_embedded(
        nocterm_ai::model::native::content::embedded(body, a, e)?,
    ))
}
fn resource(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ResourceBody>, Error> {
    plan.payload().validate(&[Uri, MimeType, Meta], &[Uri])?;
    let uri = resource_text(plan, held, Uri, ledger)?;
    let extensions = if plan.payload().has(Meta) {
        let node = raw_node(plan, held, Meta, ledger)?;
        if node.kind() == nocterm_ai::model::json::deferred::native::NativeKind::Object {
            NativeOwned::supplied(nocterm_ai::model::native::content::extensions(
                node, ledger,
            )?)
        } else {
            NativeOwned::<nocterm_ai::model::extensions::Extensions>::absent(ledger)?
        }
    } else {
        NativeOwned::<nocterm_ai::model::extensions::Extensions>::absent(ledger)?
    };
    let text = plan
        .payload()
        .validate(&[Text], &[Text])
        .and_then(|()| resource_text(plan, held, Text, ledger));
    match text {
        Ok(text) => {
            let mime = fallback(
                resource_text(plan, held, MimeType, ledger).map(NativeOwned::supplied),
                || Ok(NativeOwned::<String>::absent(ledger)?),
            )?;
            Ok(nocterm_ai::model::native::content::text_body(
                nocterm_ai::model::native::content::resource_text(uri, text, mime, extensions)?,
            ))
        }
        Err(error) if error.defaultable() => {
            let positional = matches!(plan.payload().field(MimeType), Some(Field::Positional(_)));
            let key = if positional { MimeType } else { Blob };
            plan.payload().validate(&[key], &[key])?;
            let blob = resource_text(plan, held, key, ledger)?;
            let mime = if positional {
                NativeOwned::<String>::absent(ledger)?
            } else {
                fallback(
                    resource_text(plan, held, MimeType, ledger).map(NativeOwned::supplied),
                    || Ok(NativeOwned::<String>::absent(ledger)?),
                )?
            };
            Ok(nocterm_ai::model::native::content::blob_body(
                nocterm_ai::model::native::content::resource_blob(uri, blob, mime, extensions)?,
            ))
        }
        Err(error) => Err(error),
    }
}
fn annotations(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<AnnotationsValue>, Error> {
    // Audience is a primitive sequence, unlike the enclosing struct.
    let audience = if !plan.payload().has(Audience) || !f::starts(plan, held, Audience, b'[')? {
        NativeOwned::<Vec<AudienceValue>>::absent(ledger)?
    } else {
        let values =
            lists::primitive_skipping(plan, held, Audience, ledger, usize::MAX, |node, l| {
                let role = match node.text() {
                    Some("user") => AudienceValue::User,
                    Some("assistant") => AudienceValue::Assistant,
                    _ => return Ok(None),
                };
                Ok(Some(nocterm_ai::model::native::content::audience(role, l)?))
            });
        fallback(values.map(NativeOwned::supplied), || {
            Ok(NativeOwned::<Vec<AudienceValue>>::absent(ledger)?)
        })?
    };
    let modified = optional_text(plan, held, LastModified, ledger)?;
    let priority = fallback(
        f::optional_number(plan, held, Priority, ledger, |v| v.as_f64()),
        || Ok(None),
    )?;
    Ok(nocterm_ai::model::native::content::annotations(
        audience,
        modified,
        priority,
        extensions(plan, held, Meta, ledger)?,
    )?)
}
