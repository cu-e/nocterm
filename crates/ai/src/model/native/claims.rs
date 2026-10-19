//! Descriptive bridge evidence joins original native text credits, never authority.
use super::NativeOwned;
use crate::model::{
    extensions::Extensions,
    json::deferred::{DecodeLedger, JsonDecodeError},
    tools::{BridgeClaims, Claim, ToolPatch, ToolRecord},
};
pub fn known(
    registration: NativeOwned<String>,
    tool: NativeOwned<String>,
) -> Result<NativeOwned<Claim>, JsonDecodeError> {
    let ((registration, tool), allocation) = registration.join(tool)?.into_parts();
    Ok(NativeOwned::from_parts(
        Claim::Known { registration, tool },
        allocation,
    ))
}
pub fn absent_claim(ledger: &DecodeLedger) -> Result<NativeOwned<Claim>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(Claim::Absent, ledger.reserve(0)?))
}
pub fn invalid_claim(ledger: &DecodeLedger) -> Result<NativeOwned<Claim>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(Claim::Invalid, ledger.reserve(0)?))
}
pub fn claims(
    name: NativeOwned<Claim>,
    title: NativeOwned<Claim>,
    input: NativeOwned<Claim>,
    legacy_display_hint: NativeOwned<Option<String>>,
) -> Result<NativeOwned<BridgeClaims>, JsonDecodeError> {
    let ((((name, title), input), legacy_display_hint), allocation) = name
        .join(title)?
        .join(input)?
        .join(legacy_display_hint)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        BridgeClaims {
            name,
            title,
            input,
            legacy_display_hint,
        },
        allocation,
    ))
}
pub fn record_with_claims(
    owned: NativeOwned<ToolRecord>,
    claims: NativeOwned<BridgeClaims>,
) -> Result<NativeOwned<ToolRecord>, JsonDecodeError> {
    let ((mut record, claims), allocation) = owned.join(claims)?.into_parts();
    record.claims = claims;
    Ok(NativeOwned::from_parts(record, allocation))
}
pub fn patch_with_claims(
    owned: NativeOwned<ToolPatch>,
    claims: NativeOwned<BridgeClaims>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    let ((mut patch, claims), allocation) = owned.join(claims)?.into_parts();
    patch.claims = claims;
    Ok(NativeOwned::from_parts(patch, allocation))
}
pub fn with_extensions(
    owned: NativeOwned<ToolPatch>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
    let ((mut patch, extensions), allocation) = owned.join(extensions)?.into_parts();
    patch.extensions = extensions;
    Ok(NativeOwned::from_parts(patch, allocation))
}
