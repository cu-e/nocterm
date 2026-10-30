//! Private closed storage layout, located using only normal account metadata.
use super::*;
use nocterm_cloud_model::identity::{AccountIdentity, Fingerprint, HubIdentity, NamespaceId};
use sha2::{Digest, Sha256};
pub(super) const MAX_PLAINTEXT: usize = 4 * 1024;
pub(super) const KIND: &str = "cloud-hub-trust";
pub(super) const OBJECT: &str = "hub-authority-v1";
const LABEL: &[u8] = b"nocterm/private-hub-trust/v1";

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
    pub(super) fn context(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            LABEL.len()
                + self.origin.len()
                + self.raw_hub.len()
                + self.account.id().as_str().len()
                + 38,
        );
        bytes.extend_from_slice(LABEL);
        for text in [
            self.origin.as_str(),
            self.raw_hub.as_str(),
            self.account.id().as_str(),
        ] {
            bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes.extend_from_slice(self.account.key().as_bytes());
        bytes
    }
}
pub(super) fn collection_id(context: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nocterm/private-hub-trust/collection/v1");
    hash.update(context);
    let digest: [u8; 32] = hash.finalize().into();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes)
        .into_uuid()
        .to_string()
}
pub(super) fn encode(
    context: &[u8],
    authority: [u8; 32],
) -> Result<zeroize::Zeroizing<Vec<u8>>, CloudTrustError> {
    if context.len().saturating_add(32) > MAX_PLAINTEXT {
        return Err(CloudTrustError::Capacity);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(context.len() + 32));
    bytes.extend_from_slice(context);
    bytes.extend_from_slice(&authority);
    Ok(bytes)
}
pub(super) fn decode(context: &[u8], bytes: &[u8]) -> Result<[u8; 32], CloudTrustError> {
    if bytes.len() > MAX_PLAINTEXT {
        return Err(CloudTrustError::Capacity);
    }
    if bytes.len() != context.len() + 32 || !bytes.starts_with(context) {
        return Err(CloudTrustError::Binding);
    }
    bytes[context.len()..]
        .try_into()
        .map_err(|_| CloudTrustError::Format)
}
