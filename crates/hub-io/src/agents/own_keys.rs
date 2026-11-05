//! The provider key of an agent that works only while a device is online.
//!
//! The key never goes to a pool: it is kept as an object of the account's own encrypted
//! collection (a hub sees ciphertext only) and is read back only on a device of the account
//! to seal a short-lived lease to the own runner (ADR 0126). `Debug` never shows it.

use crate::account::operations::{OpenedSource, OpenedWork};
use crate::store::{Located, OpError, Store};
use nocterm_cloud_agents_model::CloudAgentError;
use nocterm_cloud_device::records::{PROVIDER_KEYS_KIND, ProviderKeyRecord, provider_key_object};
use nocterm_core::SharedCause;
use std::sync::Arc;
use zeroize::Zeroizing;

pub(crate) const KIND: &str = PROVIDER_KEYS_KIND;

/// The canonical account record; no copied native JSON layout or second plaintext key.
#[derive(Debug)]
pub(crate) struct OwnAgentKey(ProviderKeyRecord);
impl OwnAgentKey {
    pub(crate) fn new(
        provider: &str,
        models: &[String],
        key: &str,
    ) -> Result<Self, CloudAgentError> {
        ProviderKeyRecord::new_models(provider, models.to_vec(), key.as_bytes())
            .map(Self)
            .map_err(failed)
    }
    pub(crate) fn provider(&self) -> &str {
        self.0.provider()
    }
    pub(crate) fn model(&self) -> &str {
        self.0.model()
    }
    pub(crate) fn models(&self) -> &[String] {
        self.0.models()
    }
    #[cfg(test)]
    pub(crate) fn expose(&self) -> &str {
        self.0.expose_text()
    }
    pub(crate) fn with_models(self, models: Vec<String>) -> Result<Self, CloudAgentError> {
        self.0.with_models(models).map(Self).map_err(failed)
    }
    fn encode(&self) -> Result<Zeroizing<Vec<u8>>, CloudAgentError> {
        self.0.encode().map_err(failed)
    }
    fn decode(bytes: &[u8]) -> Result<Self, CloudAgentError> {
        ProviderKeyRecord::decode(bytes).map(Self).map_err(failed)
    }
}

fn failed(error: impl std::error::Error + Send + Sync + 'static) -> CloudAgentError {
    CloudAgentError::Registration(SharedCause::new(error))
}

fn store(error: OpError) -> CloudAgentError {
    match error {
        OpError::Hub(error) => super::error::hub_error(error),
        OpError::Conflict => CloudAgentError::Conflict,
        other => failed(other),
    }
}

/// The object id of an agent's key.
fn object(agent: &str) -> String {
    provider_key_object(agent)
}

/// The exact authenticated object and account read for a model-only edit. It cannot be
/// redirected to another collection or updated to a later writer's version.
pub(crate) struct OwnKeyVersion {
    store: Store,
    at: Located,
    object: String,
    expected: u64,
    source: Arc<OpenedSource>,
}

/// Captures a compare-and-swap receipt, including a tombstone or a missing object.
pub(crate) async fn prepare_write(
    work: &OpenedWork,
    agent: &str,
) -> Result<(Option<OwnAgentKey>, OwnKeyVersion), CloudAgentError> {
    work.source().check().map_err(super::error::hub_error)?;
    let store_ = work.transient_store();
    let at = store_.ensure(KIND).await.map_err(store)?;
    read_at(work, store_, at, agent).await
}

pub(crate) async fn put_expected(
    receipt: OwnKeyVersion,
    key: &OwnAgentKey,
) -> Result<(), CloudAgentError> {
    receipt.source.check().map_err(super::error::hub_error)?;
    receipt
        .store
        .put(
            &receipt.at,
            KIND,
            &receipt.object,
            &key.encode()?,
            receipt.expected,
        )
        .await
        .map_err(store)?;
    receipt.source.check().map_err(super::error::hub_error)
}

#[cfg(test)]
async fn put(work: &OpenedWork, agent: &str, key: &OwnAgentKey) -> Result<(), CloudAgentError> {
    let (_, version) = prepare_write(work, agent).await?;
    put_expected(version, key).await
}

/// The key stored for `agent`, if this account has one.
pub(crate) async fn get(
    work: &OpenedWork,
    agent: &str,
) -> Result<Option<OwnAgentKey>, CloudAgentError> {
    Ok(get_versioned(work, agent).await?.map(|(key, _)| key))
}

pub(crate) async fn get_versioned(
    work: &OpenedWork,
    agent: &str,
) -> Result<Option<(OwnAgentKey, OwnKeyVersion)>, CloudAgentError> {
    work.source().check().map_err(super::error::hub_error)?;
    let store_ = work.transient_store();
    let Some(at) = store_.find(KIND).await.map_err(store)? else {
        return Ok(None);
    };
    let (key, version) = read_at(work, store_, at, agent).await?;
    Ok(key.map(|key| (key, version)))
}

async fn read_at(
    work: &OpenedWork,
    store_: Store,
    at: Located,
    agent: &str,
) -> Result<(Option<OwnAgentKey>, OwnKeyVersion), CloudAgentError> {
    let object = object(agent);
    let stored = work.ops().object(at.id(), &object).await.map_err(store)?;
    let expected = stored.as_ref().map_or(0, |stored| stored.version);
    let key = match stored {
        Some(stored) if !stored.deleted => {
            let plain = at
                .key()
                .open_object(at.id(), &object, KIND, &stored.ciphertext)
                .map_err(failed)?;
            Some(OwnAgentKey::decode(&plain)?)
        }
        _ => None,
    };
    work.source().check().map_err(super::error::hub_error)?;
    Ok((
        key,
        OwnKeyVersion {
            store: store_,
            at,
            object,
            expected,
            source: work.source().clone(),
        },
    ))
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
