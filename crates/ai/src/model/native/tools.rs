//! Concrete tool construction preserves ID/header and exclusive-payload leases.
use super::NativeOwned;
use crate::model::{
    content::Content,
    extensions::Extensions,
    ids::{ProviderTerminalId, ToolId},
    json::{OwnedJson, deferred::JsonDecodeError},
    shared_display::SharedDisplay,
    tools::*,
};
use std::{mem::size_of, path::PathBuf};
pub fn tool_id(text: NativeOwned<String>) -> Result<NativeOwned<ToolId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = ToolId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn terminal_id(
    text: NativeOwned<String>,
) -> Result<NativeOwned<ProviderTerminalId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = ProviderTerminalId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
/// The row's identity/header is assembled separately from its payload collections.
/// These private construction fields are consumed once, never mutated as live state.
pub struct NativeToolHeader {
    id: ToolId,
    title: String,
    name: Option<String>,
    kind: ToolKind,
    status: ToolStatus,
}
pub fn header(
    id: NativeOwned<ToolId>,
    title: NativeOwned<String>,
    name: NativeOwned<Option<String>>,
    kind: ToolKind,
    status: ToolStatus,
) -> Result<NativeOwned<NativeToolHeader>, JsonDecodeError> {
    let (((id, title), name), allocation) = id.join(title)?.join(name)?.into_parts();
    Ok(NativeOwned::from_parts(
        NativeToolHeader {
            id,
            title,
            name,
            kind,
            status,
        },
        allocation,
    ))
}
pub fn with_literal(
    mut owned: NativeOwned<ToolRecord>,
    literal: crate::model::literal::LiteralEvidence,
) -> NativeOwned<ToolRecord> {
    owned.value.literal = literal;
    owned
}
pub fn record(
    header: NativeOwned<NativeToolHeader>,
    output: NativeOwned<Vec<ToolOutput>>,
    locations: NativeOwned<Vec<Location>>,
    arguments: NativeOwned<Option<OwnedJson>>,
    result: NativeOwned<Option<OwnedJson>>,
    extensions: NativeOwned<Option<Extensions>>,
    client_display: NativeOwned<Option<SharedDisplay>>,
) -> Result<NativeOwned<ToolRecord>, JsonDecodeError> {
    let (
        ((((((header, output), locations), arguments), result), extensions), client_display),
        allocation,
    ) = header
        .join(output)?
        .join(locations)?
        .join(arguments)?
        .join(result)?
        .join(extensions)?
        .join(client_display)?
        .into_parts();
    let NativeToolHeader {
        id,
        title,
        name,
        kind,
        status,
    } = header;
    Ok(NativeOwned::from_parts(
        ToolRecord {
            id,
            title,
            name,
            kind,
            status,
            output,
            locations,
            arguments,
            result,
            extensions,
            client_display,
            claims: BridgeClaims::default(),
            literal: Default::default(),
        },
        allocation,
    ))
}
/// String -> OsString is guaranteed to move without allocation or copying
/// on every supported platform, including Windows' valid UTF-8/WTF-8 subset.
/// PathBuf then takes that same owned OsString backing.
pub fn path(text: NativeOwned<String>) -> Result<NativeOwned<PathBuf>, JsonDecodeError> {
    let (text, allocation) = text.into_parts();
    Ok(NativeOwned::from_parts(
        std::ffi::OsString::from(text).into(),
        allocation,
    ))
}
pub fn diff(
    path: NativeOwned<PathBuf>,
    previous: NativeOwned<Option<String>>,
    replacement: NativeOwned<String>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<FileDiff>, JsonDecodeError> {
    let ((((path, previous), replacement), extensions), allocation) = path
        .join(previous)?
        .join(replacement)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        FileDiff {
            path,
            previous,
            replacement,
            extensions,
        },
        allocation,
    ))
}
pub fn diff_output(owned: NativeOwned<FileDiff>) -> NativeOwned<ToolOutput> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(ToolOutput::FileDiff(value), allocation)
}
pub fn location(
    path: NativeOwned<PathBuf>,
    line: Option<u32>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<Location>, JsonDecodeError> {
    let ((path, extensions), allocation) = path.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        Location {
            path,
            line,
            extensions,
        },
        allocation,
    ))
}
pub fn content_output(
    content: NativeOwned<Content>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ToolOutput>, JsonDecodeError> {
    let joined = content.join(extensions)?;
    let mut box_allocation = joined.ledger().reserve(size_of::<Content>())?;
    let ((content, extensions), allocation) = joined.into_parts();
    box_allocation
        .merge(allocation)
        .expect("same-ledger disjoint payload and prepaid Box");
    Ok(NativeOwned::from_parts(
        ToolOutput::Content {
            content: Box::new(content),
            extensions,
        },
        box_allocation,
    ))
}
pub fn terminal_output(
    id: NativeOwned<ProviderTerminalId>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ToolOutput>, JsonDecodeError> {
    let ((id, extensions), allocation) = id.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        ToolOutput::TerminalReference { id, extensions },
        allocation,
    ))
}
pub fn display(
    version: u8,
    tool: NativeOwned<String>,
    arguments: NativeOwned<OwnedJson>,
    destination: NativeOwned<crate::model::controls::Presence<String>>,
    redacted: crate::model::controls::Presence<bool>,
    outcome: NativeOwned<crate::model::controls::Presence<DisplayOutcome>>,
) -> Result<NativeOwned<ClientDisplay>, JsonDecodeError> {
    let ((((tool, arguments), destination), outcome), allocation) = tool
        .join(arguments)?
        .join(destination)?
        .join(outcome)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ClientDisplay {
            version,
            tool,
            arguments,
            destination,
            redacted,
            outcome,
        },
        allocation,
    ))
}
pub fn shared(
    owned: NativeOwned<ClientDisplay>,
) -> Result<NativeOwned<SharedDisplay>, JsonDecodeError> {
    let ledger = owned.ledger().share();
    let value = SharedDisplay::admitted(owned)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn pending(
    ledger: &super::DecodeLedger,
) -> Result<NativeOwned<DisplayOutcome>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        DisplayOutcome::Pending,
        ledger.reserve(0)?,
    ))
}
pub fn succeeded(value: NativeOwned<OwnedJson>) -> NativeOwned<DisplayOutcome> {
    let (value, allocation) = value.into_parts();
    NativeOwned::from_parts(DisplayOutcome::Succeeded(value), allocation)
}
pub fn failed(value: NativeOwned<String>) -> NativeOwned<DisplayOutcome> {
    let (value, allocation) = value.into_parts();
    NativeOwned::from_parts(DisplayOutcome::Failed(value), allocation)
}
