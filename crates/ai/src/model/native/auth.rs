//! Authentication and permission DTO construction joins original paid components.
//! Terminal environment backing and one-shot reply ownership are separate owners.
use super::NativeOwned;
use crate::model::{
    auth::{
        AuthFlow, AuthMethod, EnvironmentPair, PermissionChoice, PermissionEffect,
        ProviderPermission, TerminalEnvironment, TerminalSetup,
    },
    extensions::Extensions,
    ids::{AuthMethodId, PermissionOptionId, SessionId},
    json::deferred::{DecodeLedger, JsonDecodeError},
    tools::ToolPatch,
};
pub fn auth_method_id(
    text: NativeOwned<String>,
) -> Result<NativeOwned<AuthMethodId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = AuthMethodId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn permission_option_id(
    text: NativeOwned<String>,
) -> Result<NativeOwned<PermissionOptionId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = PermissionOptionId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn agent_managed(ledger: &DecodeLedger) -> Result<NativeOwned<AuthFlow>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        AuthFlow::AgentManaged,
        ledger.reserve(0)?,
    ))
}
pub fn terminal_setup_flow(value: NativeOwned<TerminalSetup>) -> NativeOwned<AuthFlow> {
    let (value, allocation) = value.into_parts();
    NativeOwned::from_parts(AuthFlow::TerminalSetup(value), allocation)
}
pub fn auth_method(
    id: NativeOwned<AuthMethodId>,
    label: NativeOwned<String>,
    description: NativeOwned<Option<String>>,
    flow: NativeOwned<AuthFlow>,
) -> Result<NativeOwned<AuthMethod>, JsonDecodeError> {
    let ((((id, label), description), flow), allocation) =
        id.join(label)?.join(description)?.join(flow)?.into_parts();
    Ok(NativeOwned::from_parts(
        AuthMethod {
            id,
            label,
            description,
            flow,
        },
        allocation,
    ))
}
pub fn permission_choice(
    id: NativeOwned<PermissionOptionId>,
    label: NativeOwned<String>,
    effect: PermissionEffect,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<PermissionChoice>, JsonDecodeError> {
    let (((id, label), extensions), allocation) = id.join(label)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        PermissionChoice {
            id,
            label,
            effect,
            extensions,
        },
        allocation,
    ))
}
pub fn permission(
    session: NativeOwned<SessionId>,
    tool: NativeOwned<ToolPatch>,
    choices: NativeOwned<Vec<PermissionChoice>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ProviderPermission>, JsonDecodeError> {
    let ((((session, tool), choices), extensions), allocation) = session
        .join(tool)?
        .join(choices)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ProviderPermission {
            session,
            tool,
            choices,
            extensions,
        },
        allocation,
    ))
}

pub fn environment_pair(
    key: NativeOwned<String>,
    value: NativeOwned<String>,
) -> Result<NativeOwned<EnvironmentPair>, JsonDecodeError> {
    let ((key, value), allocation) = key.join(value)?.into_parts();
    Ok(NativeOwned::from_parts(
        EnvironmentPair::new(key, value),
        allocation,
    ))
}
/// Only unique decoded source keys are accepted. Sorting is allocation-free;
/// the adapter's original-source index determines the last winning value.
pub fn environment(
    pairs: NativeOwned<Vec<EnvironmentPair>>,
) -> Result<NativeOwned<TerminalEnvironment>, JsonDecodeError> {
    let (pairs, allocation) = pairs.into_parts();
    Ok(NativeOwned::from_parts(
        TerminalEnvironment::from_unique(pairs)?,
        allocation,
    ))
}
pub fn terminal_setup(
    args: NativeOwned<Vec<String>>,
    environment: NativeOwned<TerminalEnvironment>,
) -> Result<NativeOwned<TerminalSetup>, JsonDecodeError> {
    let ((args, env), allocation) = args.join(environment)?.into_parts();
    Ok(NativeOwned::from_parts(
        TerminalSetup { args, env },
        allocation,
    ))
}
