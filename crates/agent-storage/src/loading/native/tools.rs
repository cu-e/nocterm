//! Concrete v2 tool records, outputs and locations without SDK materialization.
use super::{
    Error, fields as f, lists,
    schema::{self as s, Scope},
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::{Key, Key::*},
};
use nocterm_ai::model::{
    json::{OwnedJson, deferred::DecodeLedger},
    native::NativeOwned,
    tools::*,
};
pub(super) mod display;
pub(super) fn tool(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ToolRecord>, Error> {
    let id = nocterm_ai::model::native::tools::tool_id(f::text(plan, held, Id, ledger)?)?;
    let title = f::text(plan, held, Title, ledger)?;
    let name = f::optional_text(plan, held, Name, ledger)?;
    let kind = f::text(plan, held, Kind, ledger)?;
    let kind = kind_of(kind.as_str())?;
    let status = f::text(plan, held, Status, ledger)?;
    let status = match status.as_str() {
        "pending" => ToolStatus::Pending,
        "running" => ToolStatus::Running,
        "succeeded" => ToolStatus::Succeeded,
        "failed" => ToolStatus::Failed,
        _ => return Err(Error::Variant),
    };
    let header = nocterm_ai::model::native::tools::header(id, title, name, kind, status)?;
    let output = lists::list(plan, held, Output, ledger, usize::MAX, s::TAG, output)?;
    let locations = lists::list(
        plan,
        held,
        Locations,
        ledger,
        usize::MAX,
        s::LOCATION,
        location,
    )?;
    let arguments = json_presence(plan, held, Arguments, ledger)?;
    let result = json_presence(plan, held, Result, ledger)?;
    let extensions = f::extensions(plan, held, Extensions, ledger)?;
    let client_display = f::optional(plan, held, ClientDisplay, ledger, display::display)?;
    Ok(nocterm_ai::model::native::tools::record(
        header,
        output,
        locations,
        arguments,
        result,
        extensions,
        client_display,
    )?)
}
fn kind_of(text: &str) -> std::result::Result<ToolKind, Error> {
    Ok(match text {
        "read" => ToolKind::Read,
        "edit" => ToolKind::Edit,
        "delete" => ToolKind::Delete,
        "move" => ToolKind::Move,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Execute,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "change_mode" => ToolKind::ChangeMode,
        "other" => ToolKind::Other,
        _ => return Err(Error::Variant),
    })
}
fn json_presence(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Option<OwnedJson>>, Error> {
    let p = f::child(plan, held, key, s::PRESENCE, ledger)?;
    f::semantic_value(&p, held, ledger)?;
    let presence = f::text(&p, held, Presence, ledger)?;
    match presence.as_str() {
        "absent" => Ok(NativeOwned::<OwnedJson>::absent(ledger)?),
        "supplied" => {
            let value = if p.payload().has(Value) {
                nocterm_ai::model::native::content::json(f::node(&p, held, Value, ledger)?, ledger)?
            } else {
                nocterm_ai::model::native::content::null_json(ledger)?
            };
            Ok(NativeOwned::supplied(value))
        }
        _ => Err(Error::Variant),
    }
}
fn output(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ToolOutput>, Error> {
    let kind = f::tag(plan, held, ledger)?;
    match kind.as_str() {
        "content" => {
            let p = f::child(plan, held, Value, s::OUTPUT_CONTENT, ledger)?;
            let content = f::child(&p, held, Content, s::TAG, ledger)?;
            let content = super::content::content(&content, held, ledger)?;
            Ok(nocterm_ai::model::native::tools::content_output(
                content,
                f::extensions(&p, held, Extensions, ledger)?,
            )?)
        }
        "file_diff" => {
            let p = f::child(plan, held, Value, s::DIFF, ledger)?;
            let path = f::path(&p, held, Path, ledger)?;
            let previous = f::optional_text(&p, held, Previous, ledger)?;
            let replacement = f::text(&p, held, Replacement, ledger)?;
            let extensions = f::extensions(&p, held, Extensions, ledger)?;
            Ok(nocterm_ai::model::native::tools::diff_output(
                nocterm_ai::model::native::tools::diff(path, previous, replacement, extensions)?,
            ))
        }
        "terminal_reference" => {
            let p = f::child(plan, held, Value, s::OUTPUT_TERMINAL, ledger)?;
            let id = nocterm_ai::model::native::tools::terminal_id(f::text(&p, held, Id, ledger)?)?;
            Ok(nocterm_ai::model::native::tools::terminal_output(
                id,
                f::extensions(&p, held, Extensions, ledger)?,
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
    let line = f::optional_number(plan, held, Line, ledger, |n| {
        n.as_u64().and_then(|v| u32::try_from(v).ok())
    })?;
    Ok(nocterm_ai::model::native::tools::location(
        path,
        line,
        f::extensions(plan, held, Extensions, ledger)?,
    )?)
}
