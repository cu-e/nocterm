//! Private versioned SDK envelope, always anchored by the host journal receipt.
use crate::{config::ProviderKind, limits, state::ActiveRun};
use nocterm_cloud_model::{
    engine::{EngineCheckpoint, EngineConfig, EngineError},
    identity::{NamespaceId, StreamRevision},
    payload::{MAX_LOGICAL_PAYLOAD_BYTES, OpaqueBytes},
};
use serde::{Deserialize, Serialize};

const FORMAT: &str = "nocterm.rig.a0efbf90.v2";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineIdentity {
    source: [String; 2],
    keys: [[u8; 32]; 2],
    ids: [[u8; 16]; 3],
    epochs: [u64; 2],
    model: String,
    limits: [u64; 5],
    provider: (u8, String),
    history: [u8; 32],
}
impl EngineIdentity {
    pub(crate) fn new(
        config: &EngineConfig,
        kind: ProviderKind,
        endpoint: &str,
    ) -> Result<Self, EngineError> {
        use sha2::{Digest, Sha256};
        if config.history.encoded_bytes() > config.budget.input_bytes() {
            return Err(EngineError::BudgetExceeded);
        }
        let seed = limits::encode(
            &crate::mapping::history(&config.history)?,
            config.budget.input_bytes(),
        )?;
        let history = Sha256::digest(seed).into();
        let scope = config.run.scope();
        let budget = config.budget;
        Ok(Self {
            source: [
                scope.hub().instance().as_str().to_owned(),
                scope.account().id().as_str().to_owned(),
            ],
            keys: [
                *scope.hub().authority().as_bytes(),
                *scope.account().key().as_bytes(),
            ],
            ids: [
                *config.run.runner().as_bytes(),
                *config.run.chat().as_bytes(),
                *config.run.run().as_bytes(),
            ],
            epochs: [config.run.runner_epoch().get(), config.run.epoch().get()],
            model: config.model.as_str().to_owned(),
            limits: [
                budget.calls() as u64,
                budget.turns() as u64,
                budget.input_bytes() as u64,
                budget.output_bytes() as u64,
                budget.output_tokens(),
            ],
            history,
            provider: (
                match kind {
                    ProviderKind::OpenAiChat => 1,
                    ProviderKind::AnthropicMessages => 2,
                },
                endpoint.to_owned(),
            ),
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    identity: EngineIdentity,
    state: Option<ActiveRun>,
}
pub(crate) fn restore(
    config: &EngineConfig,
    identity: &EngineIdentity,
    checkpoint: EngineCheckpoint,
) -> Result<Option<ActiveRun>, EngineError> {
    if checkpoint.format().as_str() != FORMAT {
        return Err(EngineError::UnsupportedFormat);
    }
    if checkpoint.run() != &config.run {
        return Err(EngineError::InvalidStream);
    }
    if checkpoint.bytes().len() > MAX_LOGICAL_PAYLOAD_BYTES {
        return Err(EngineError::BudgetExceeded);
    }
    let snapshot: Snapshot = serde_json::from_slice(checkpoint.bytes().as_bytes())
        .map_err(|_| EngineError::UnsupportedFormat)?;
    if snapshot.version != 2 || &snapshot.identity != identity {
        return Err(EngineError::UnsupportedFormat);
    }
    if let Some(state) = &snapshot.state {
        state.require_budget(config.budget)?;
    }
    Ok(snapshot.state)
}
pub(crate) fn save(
    config: &EngineConfig,
    identity: &EngineIdentity,
    state: Option<&ActiveRun>,
    revision: StreamRevision,
) -> Result<EngineCheckpoint, EngineError> {
    let bytes = encoded(identity, state)?;
    let format = NamespaceId::new(FORMAT).map_err(|_| EngineError::UnsupportedFormat)?;
    EngineCheckpoint::new(
        config.run.clone(),
        format,
        revision,
        OpaqueBytes::new(bytes)?,
    )
}

#[derive(Serialize)]
struct SnapshotView<'a> {
    version: u32,
    identity: &'a EngineIdentity,
    state: Option<&'a ActiveRun>,
}
fn encoded(identity: &EngineIdentity, state: Option<&ActiveRun>) -> Result<Vec<u8>, EngineError> {
    limits::encode(
        &SnapshotView {
            version: 2,
            identity,
            state,
        },
        MAX_LOGICAL_PAYLOAD_BYTES,
    )
}
pub(crate) fn validate_size(
    identity: &EngineIdentity,
    state: &ActiveRun,
) -> Result<(), EngineError> {
    // Include raw provider metadata and SDK history, not only visible text.
    // This bounds retained state even if a provider sends huge opaque fields
    // beside tiny tool calls and reports zero generated tokens.
    encoded(identity, Some(state)).map(|_| ())
}
