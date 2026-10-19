//! Original Call field defaults and per-element output/location skip policies.
use super::{Error, extensions, f, fallback, opaque, optional_text};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key::{self, *},
    native::{
        lists,
        schema::{self as s, Scope},
    },
};
use nocterm_ai::model::{
    json::{OwnedJson, deferred::DecodeLedger},
    native::NativeOwned,
    tools::*,
};
mod meta;
pub(in crate::loading::native) fn tool(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<
    (
        NativeOwned<ToolRecord>,
        Option<crate::legacy::ControlledRowOrder>,
    ),
    Error,
> {
    let id = nocterm_ai::model::native::tools::tool_id(f::text(plan, held, ToolCallId, ledger)?)?;
    let title = f::text(plan, held, Title, ledger)?;
    let name = optional_text(plan, held, Name, ledger)?;
    let kind = enum_text(plan, held, Kind, ledger, kind, ToolKind::Other)?;
    let status = enum_text(plan, held, Status, ledger, status, ToolStatus::Pending)?;
    let arguments = json(plan, held, RawInput, ledger)?;
    let result = json(plan, held, RawOutput, ledger)?;
    let literal = crate::legacy::controlled_literal(
        name.as_optional_str(),
        Some(title.as_str()),
        arguments.as_optional_json().map(|v| v.as_value()),
    );
    let header = nocterm_ai::model::native::tools::header(id, title, name, kind, status)?;
    let output = lists::skipping(plan, held, Content, ledger, s::LEGACY_OUTPUT, output)?;
    let locations = lists::skipping(plan, held, Locations, ledger, s::LEGACY_LOCATION, location)?;
    let (extensions, display, order) =
        meta::metadata(opaque(plan, held, Meta, ledger, true)?, ledger)?;
    let row = nocterm_ai::model::native::tools::with_literal(
        nocterm_ai::model::native::tools::record(
            header, output, locations, arguments, result, extensions, display,
        )?,
        literal,
    );
    let order =
        order.and_then(|(slot, fields)| crate::legacy::controlled_order(row.get(), slot, fields));
    Ok((row, order))
}
fn enum_text<T: Copy>(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    parse: impl FnOnce(&str) -> Option<T>,
    default: T,
) -> std::result::Result<T, Error> {
    if !plan.payload().has(key) {
        return Ok(default);
    }
    fallback(
        f::text(plan, held, key, ledger).and_then(|v| parse(v.as_str()).ok_or(Error::Variant)),
        || Ok(default),
    )
}
fn kind(value: &str) -> Option<ToolKind> {
    Some(match value {
        "read" => ToolKind::Read,
        "edit" => ToolKind::Edit,
        "delete" => ToolKind::Delete,
        "move" => ToolKind::Move,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Execute,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "switch_mode" => ToolKind::ChangeMode,
        "other" => ToolKind::Other,
        _ => return None,
    })
}
fn status(value: &str) -> Option<ToolStatus> {
    Some(match value {
        "pending" => ToolStatus::Pending,
        "in_progress" => ToolStatus::Running,
        "completed" => ToolStatus::Succeeded,
        "failed" => ToolStatus::Failed,
        _ => return None,
    })
}
fn json(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Option<OwnedJson>>, Error> {
    match opaque(plan, held, key, ledger, false)? {
        Some(node) => Ok(NativeOwned::supplied(
            nocterm_ai::model::native::content::json(node, ledger)?,
        )),
        None => Ok(NativeOwned::<OwnedJson>::absent(ledger)?),
    }
}
fn output(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ToolOutput>, Error> {
    let kind = f::text(plan, held, Type, ledger)?;
    match kind.as_str() {
        "content" => {
            plan.payload()
                .validate(&[Type, Content, Meta], &[Type, Content])?;
            let p = f::child(plan, held, Content, s::LEGACY_BLOCK, ledger)?;
            let content = super::content::content(&p, held, ledger)?;
            Ok(nocterm_ai::model::native::tools::content_output(
                content,
                extensions(plan, held, Meta, ledger)?,
            )?)
        }
        "diff" => {
            plan.payload().validate(
                &[Type, Path, OldText, NewText, Meta],
                &[Type, Path, NewText],
            )?;
            let path = f::path(plan, held, Path, ledger)?;
            let previous = optional_text(plan, held, OldText, ledger)?;
            let replacement = f::text(plan, held, NewText, ledger)?;
            Ok(nocterm_ai::model::native::tools::diff_output(
                nocterm_ai::model::native::tools::diff(
                    path,
                    previous,
                    replacement,
                    extensions(plan, held, Meta, ledger)?,
                )?,
            ))
        }
        "terminal" => {
            plan.payload()
                .validate(&[Type, TerminalId, Meta], &[Type, TerminalId])?;
            let id = nocterm_ai::model::native::tools::terminal_id(f::text(
                plan, held, TerminalId, ledger,
            )?)?;
            Ok(nocterm_ai::model::native::tools::terminal_output(
                id,
                extensions(plan, held, Meta, ledger)?,
            )?)
        }
        _ => Err(Error::Variant),
    }
}
fn location(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Location>, Error> {
    let path = f::path(plan, held, Path, ledger)?;
    let line = fallback(
        f::optional_number(plan, held, Line, ledger, |n| {
            n.as_u64().and_then(|v| u32::try_from(v).ok())
        }),
        || Ok(None),
    )?;
    Ok(nocterm_ai::model::native::tools::location(
        path,
        line,
        extensions(plan, held, Meta, ledger)?,
    )?)
}
