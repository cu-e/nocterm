//! Concrete live tool patches. Each supplied change joins its exclusive payload
//! lease into the patch, so physical application allocates only what changed.
use super::NativeOwned;
use crate::model::{
    ids::ToolId,
    json::{OwnedJson, deferred::JsonDecodeError},
    literal::LiteralEvidence,
    tools::*,
};

/// A patch with no payload changes; `kind`/`status` carry no payload.
pub fn tool_patch(
    id: NativeOwned<ToolId>,
    kind: Option<ToolKind>,
    status: Option<ToolStatus>,
) -> NativeOwned<ToolPatch> {
    let (id, allocation) = id.into_parts();
    NativeOwned::from_parts(
        ToolPatch {
            id,
            changes: ToolChanges {
                kind,
                status,
                ..ToolChanges::default()
            },
            extensions: None,
            claims: BridgeClaims::default(),
            literal: LiteralEvidence::default(),
        },
        allocation,
    )
}
fn change<T>(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<T>,
    install: impl FnOnce(&mut ToolChanges, T),
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    let ((mut patch, value), allocation) = owned.join(value)?.into_parts();
    install(&mut patch.changes, value);
    Ok(NativeOwned::from_parts(patch, allocation))
}
pub fn with_title(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<String>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    change(owned, value, |changes, value| changes.title = Some(value))
}
pub fn with_name(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<String>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    change(owned, value, |changes, value| changes.name = Some(value))
}
pub fn with_output(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<Vec<ToolOutput>>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    change(owned, value, |changes, value| changes.output = Some(value))
}
pub fn with_locations(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<Vec<Location>>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    change(owned, value, |changes, value| {
        changes.locations = Some(value)
    })
}
pub fn with_arguments(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<OwnedJson>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    change(owned, value, |changes, value| {
        changes.arguments = Some(value)
    })
}
pub fn with_result(
    owned: NativeOwned<ToolPatch>,
    value: NativeOwned<OwnedJson>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    change(owned, value, |changes, value| changes.result = Some(value))
}
pub fn with_literal(
    mut owned: NativeOwned<ToolPatch>,
    literal: LiteralEvidence,
) -> NativeOwned<ToolPatch> {
    owned.value.literal = literal;
    owned
}
