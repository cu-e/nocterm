//! The provider key of an agent that works only while a device is online.
//!
//! The key never goes to a pool: it is kept as an object of the account's own encrypted
//! collection (a hub sees ciphertext only) and is read back only on a device of the account
//! to seal a short-lived lease to the own runner (ADR 0126). `Debug` never shows it.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use nocterm_cloud_agents_model::CloudAgentError;
use nocterm_core::SharedCause;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::account::operations::OpenedWork;
use crate::store::OpError;

/// The kind of the personal collection that holds these keys.
pub(crate) const KIND: &str = "agent_provider_keys";
const FORMAT: u8 = 1;

#[derive(Serialize, Deserialize)]
struct Stored {
    format: u8,
    provider: String,
    model: String,
    key: String,
}

/// What a runner-less agent needs to lease its credential: the provider family, the model and
/// the key.
pub(crate) struct OwnAgentKey {
    provider: String,
    model: String,
    key: Zeroizing<String>,
}

impl OwnAgentKey {
    pub(crate) fn new(provider: &str, model: &str, key: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            model: model.to_owned(),
            key: Zeroizing::new(key.to_owned()),
        }
    }

    pub(crate) fn provider(&self) -> &str {
        &self.provider
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    /// The key bytes; callers hand them to the sealer and nothing else.
    pub(crate) fn expose(&self) -> &str {
        &self.key
    }

    fn encode(&self) -> Result<Zeroizing<Vec<u8>>, CloudAgentError> {
        serde_json::to_vec(&Stored {
            format: FORMAT,
            provider: self.provider.clone(),
            model: self.model.clone(),
            key: STANDARD.encode(self.key.as_bytes()),
        })
        .map(Zeroizing::new)
        .map_err(failed)
    }

    fn decode(bytes: &[u8]) -> Result<Self, CloudAgentError> {
        let stored: Stored = serde_json::from_slice(bytes).map_err(failed)?;
        if stored.format != FORMAT {
            return Err(CloudAgentError::Registration(SharedCause::new(
                KeyDocumentError::Format,
            )));
        }
        let key = STANDARD.decode(&stored.key).map_err(failed)?;
        let key = String::from_utf8(key).map_err(failed)?;
        Ok(Self {
            provider: stored.provider,
            model: stored.model,
            key: Zeroizing::new(key),
        })
    }
}

impl fmt::Debug for OwnAgentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnAgentKey")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

/// Why the stored key document cannot be used.
#[derive(Debug, thiserror::Error)]
enum KeyDocumentError {
    #[error("the provider key document has another format")]
    Format,
}

fn failed(error: impl std::error::Error + Send + Sync + 'static) -> CloudAgentError {
    CloudAgentError::Registration(SharedCause::new(error))
}

fn store(error: OpError) -> CloudAgentError {
    match error {
        OpError::Hub(error) => super::error::hub_error(error),
        other => failed(other),
    }
}

/// The object id of an agent's key.
fn object(agent: &str) -> String {
    format!("key-{agent}")
}

/// Stores `key` for `agent`, replacing an earlier one.
pub(crate) async fn put(
    work: &OpenedWork,
    agent: &str,
    key: &OwnAgentKey,
) -> Result<(), CloudAgentError> {
    let store_ = work.transient_store();
    let at = store_.ensure(KIND).await.map_err(store)?;
    let id = object(agent);
    let current = work.ops().object(at.id(), &id).await.map_err(store)?;
    let expected = current.map_or(0, |object| object.version);
    store_
        .put(&at, KIND, &id, &key.encode()?, expected)
        .await
        .map_err(store)?;
    Ok(())
}

/// The key stored for `agent`, if this account has one.
pub(crate) async fn get(
    work: &OpenedWork,
    agent: &str,
) -> Result<Option<OwnAgentKey>, CloudAgentError> {
    let Some(at) = work.transient_store().find(KIND).await.map_err(store)? else {
        return Ok(None);
    };
    let id = object(agent);
    let Some(stored) = work.ops().object(at.id(), &id).await.map_err(store)? else {
        return Ok(None);
    };
    if stored.deleted {
        return Ok(None);
    }
    let plain = at
        .key()
        .open_object(at.id(), &id, KIND, &stored.ciphertext)
        .map_err(failed)?;
    OwnAgentKey::decode(&plain).map(Some)
}

/// Forgets the key of `agent`; an agent without one succeeds.
pub(crate) async fn remove(work: &OpenedWork, agent: &str) -> Result<(), CloudAgentError> {
    let store_ = work.transient_store();
    let Some(at) = store_.find(KIND).await.map_err(store)? else {
        return Ok(());
    };
    let id = object(agent);
    let Some(stored) = work.ops().object(at.id(), &id).await.map_err(store)? else {
        return Ok(());
    };
    if stored.deleted {
        return Ok(());
    }
    store_
        .remove(&at, &id, stored.version)
        .await
        .map_err(store)?;
    Ok(())
}

#[cfg(test)]
mod tests;
