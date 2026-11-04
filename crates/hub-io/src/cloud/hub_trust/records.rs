//! The hub-trust record of the signed-in account. Its layout is the contract's
//! (`nocterm_cloud_device::records::HubTrustRecord`, the same bytes every device reads); this
//! module captures what names the account and converts to and from the record.
use super::*;
use nocterm_cloud_device::CloudDeviceError;
use nocterm_cloud_device::records::{HUB_TRUST_KIND, HUB_TRUST_OBJECT, HubTrustRecord};
use nocterm_cloud_model::identity::{AccountIdentity, Fingerprint, HubIdentity, NamespaceId};
use sha2::{Digest, Sha256};
pub(super) const MAX_PLAINTEXT: usize = 4 * 1024;
pub(super) const KIND: &str = HUB_TRUST_KIND;
pub(super) const OBJECT: &str = HUB_TRUST_OBJECT;

#[derive(Clone)]
pub(super) struct NormalAccount {
    origin: String,
    raw_hub: String,
    account: AccountIdentity,
}
impl NormalAccount {
    pub(super) fn capture(bound: &BoundOperations, raw_hub: &str) -> Result<Self, CloudTrustError> {
        bound.source().check().map_err(account)?;
        let native = bound.source().native();
        let subject = native.subject().map_err(account)?;
        let origin = native.membership().map_err(account)?.hub().to_string();
        if origin.len() > 2048
            || raw_hub.is_empty()
            || raw_hub.len() > 128
            || subject.account().len() > 128
        {
            return Err(CloudTrustError::Capacity);
        }
        let id = NamespaceId::new(subject.account().to_owned()).map_err(backend)?;
        let digest = native
            .with_current(|keys| {
                Ok(<[u8; 32]>::from(Sha256::digest(
                    keys.keyset().public_key().to_bytes(),
                )))
            })
            .map_err(account)?;
        Ok(Self {
            origin,
            raw_hub: raw_hub.to_owned(),
            account: AccountIdentity::new(id, Fingerprint::from_bytes(digest)),
        })
    }
    pub(super) fn scope(&self, authority: [u8; 32]) -> Result<AccountScope, CloudTrustError> {
        let digest: [u8; 32] = Sha256::digest(authority).into();
        let hub =
            nocterm_hub_model::cloud::hub_namespace_id(&self.raw_hub, digest).map_err(backend)?;
        Ok(AccountScope::new(
            HubIdentity::new(
                NamespaceId::new(hub.as_str().to_owned()).map_err(backend)?,
                Fingerprint::from_bytes(digest),
            ),
            self.account.clone(),
        ))
    }
    pub(super) fn require_setup(&self, setup: &CloudRunnerSetup) -> Result<(), CloudTrustError> {
        if setup.origin.to_string() != self.origin
            || setup.raw_hub != self.raw_hub
            || setup.runner.binding().scope() != &self.scope(setup.authority)?
        {
            return Err(CloudTrustError::Binding);
        }
        Ok(())
    }
    /// The record's context: everything but the pinned authority.
    pub(super) fn context(&self) -> Vec<u8> {
        HubTrustRecord::new(
            &self.origin,
            &self.raw_hub,
            self.account.id().as_str(),
            *self.account.key().as_bytes(),
            [0; 32],
        )
        .and_then(|record| record.context())
        .unwrap_or_default()
    }
}
/// The record of `context` pinning `authority`, as the contract reads it.
fn record(context: &[u8], authority: [u8; 32]) -> Result<HubTrustRecord, CloudTrustError> {
    let mut bytes = zeroize::Zeroizing::new(context.to_vec());
    bytes.extend_from_slice(&authority);
    HubTrustRecord::decode(&bytes).map_err(refused)
}
fn refused(error: CloudDeviceError) -> CloudTrustError {
    match error {
        CloudDeviceError::Capacity => CloudTrustError::Capacity,
        CloudDeviceError::Binding => CloudTrustError::Binding,
        _ => CloudTrustError::Format,
    }
}
pub(super) fn collection_id(context: &[u8]) -> String {
    record(context, [0; 32])
        .and_then(|record| record.collection_id().map_err(refused))
        .unwrap_or_default()
}
pub(super) fn encode(
    context: &[u8],
    authority: [u8; 32],
) -> Result<zeroize::Zeroizing<Vec<u8>>, CloudTrustError> {
    if context.len().saturating_add(32) > MAX_PLAINTEXT {
        return Err(CloudTrustError::Capacity);
    }
    record(context, authority)?.encode().map_err(refused)
}
pub(super) fn decode(context: &[u8], bytes: &[u8]) -> Result<[u8; 32], CloudTrustError> {
    if bytes.len() > MAX_PLAINTEXT {
        return Err(CloudTrustError::Capacity);
    }
    let record = HubTrustRecord::decode(bytes).map_err(refused)?;
    if record.context().map_err(refused)? != context {
        return Err(CloudTrustError::Binding);
    }
    Ok(record.authority())
}
