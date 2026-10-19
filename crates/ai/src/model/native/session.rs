//! Named session results preserve the original native component reservations.
use super::NativeOwned;
use crate::{
    connection::AgentInfo,
    model::{
        auth::AuthMethod,
        controls::{ConfigOption, Modes},
        ids::SessionId,
        json::deferred::{DecodeLedger, JsonDecodeError},
        session::{
            AgentCapabilities, ContentCapabilities, EndOfTurn, PromptOutcome, PromptRequest,
            SessionOpened,
        },
        usage::TokenTotals,
    },
};
pub fn opened(
    session: NativeOwned<SessionId>,
    modes: NativeOwned<Option<Modes>>,
    configuration: NativeOwned<Option<Vec<ConfigOption>>>,
) -> Result<NativeOwned<SessionOpened>, JsonDecodeError> {
    let (((session, modes), configuration), allocation) =
        session.join(modes)?.join(configuration)?.into_parts();
    Ok(NativeOwned::from_parts(
        SessionOpened {
            session,
            modes,
            configuration,
        },
        allocation,
    ))
}
pub fn finished(ledger: &DecodeLedger) -> Result<NativeOwned<EndOfTurn>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        EndOfTurn::Finished,
        ledger.reserve(0)?,
    ))
}
pub fn token_limit(ledger: &DecodeLedger) -> Result<NativeOwned<EndOfTurn>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        EndOfTurn::TokenLimit,
        ledger.reserve(0)?,
    ))
}
pub fn request_limit(ledger: &DecodeLedger) -> Result<NativeOwned<EndOfTurn>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        EndOfTurn::RequestLimit,
        ledger.reserve(0)?,
    ))
}
pub fn refused(ledger: &DecodeLedger) -> Result<NativeOwned<EndOfTurn>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        EndOfTurn::Refused,
        ledger.reserve(0)?,
    ))
}
pub fn cancelled(ledger: &DecodeLedger) -> Result<NativeOwned<EndOfTurn>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        EndOfTurn::Cancelled,
        ledger.reserve(0)?,
    ))
}
pub fn other(value: NativeOwned<String>) -> NativeOwned<EndOfTurn> {
    let (value, allocation) = value.into_parts();
    NativeOwned::from_parts(EndOfTurn::Other(value), allocation)
}
pub fn totals(
    total: u64,
    input: u64,
    output: u64,
    thought: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<TokenTotals>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        TokenTotals {
            total,
            input,
            output,
            thought,
            cache_read,
            cache_write,
        },
        ledger.reserve(0)?,
    ))
}
pub fn outcome(
    ending: NativeOwned<EndOfTurn>,
    tokens: NativeOwned<Option<TokenTotals>>,
) -> Result<NativeOwned<PromptOutcome>, JsonDecodeError> {
    let ((ending, tokens), allocation) = ending.join(tokens)?.into_parts();
    Ok(NativeOwned::from_parts(
        PromptOutcome { ending, tokens },
        allocation,
    ))
}
pub fn capabilities(
    image: bool,
    audio: bool,
    embedded_resources: bool,
    restore: bool,
    fork: bool,
    close: bool,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<AgentCapabilities>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        AgentCapabilities {
            content: ContentCapabilities {
                image,
                audio,
                embedded_resources,
            },
            restore,
            fork,
            close,
        },
        ledger.reserve(0)?,
    ))
}
pub fn agent_info(
    name: NativeOwned<String>,
    version: NativeOwned<String>,
    capabilities: NativeOwned<AgentCapabilities>,
    auth_methods: NativeOwned<Vec<AuthMethod>>,
) -> Result<NativeOwned<AgentInfo>, JsonDecodeError> {
    let ((((name, version), capabilities), auth_methods), allocation) = name
        .join(version)?
        .join(capabilities)?
        .join(auth_methods)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        AgentInfo {
            name,
            version,
            capabilities,
            auth_methods,
        },
        allocation,
    ))
}

pub fn request(
    session: NativeOwned<SessionId>,
    content: NativeOwned<Vec<crate::model::content::Content>>,
) -> Result<NativeOwned<PromptRequest>, JsonDecodeError> {
    let ((session, content), allocation) = session.join(content)?.into_parts();
    Ok(NativeOwned::from_parts(
        PromptRequest { session, content },
        allocation,
    ))
}
