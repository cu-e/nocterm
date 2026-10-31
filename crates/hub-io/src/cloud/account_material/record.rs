//! Bounded native account object; this framing is never a public Cloud wire codec.
use super::{AccountMaterialError, ChatMaterialRequest};
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_hub_model::cloud::ChatCoordinate;
use sha2::{Digest, Sha256};

pub(super) const KIND: &str = "cloud-chat-material";
pub(super) const MAX_RECORD: usize = 1024;
pub(super) const WRAPPED_KEY: usize = 74;

pub(super) struct MaterialRecord {
    request: ChatMaterialRequest,
    wrapped: Vec<u8>,
}
impl MaterialRecord {
    pub(super) fn new(request: ChatMaterialRequest, wrapped: Vec<u8>) -> Self {
        Self { request, wrapped }
    }
    pub(super) fn request(&self) -> &ChatMaterialRequest {
        &self.request
    }
    pub(super) fn wrapped(&self) -> &[u8] {
        &self.wrapped
    }
    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountMaterialError> {
        if self.wrapped.len() != WRAPPED_KEY {
            return Err(AccountMaterialError::Format);
        }
        let mut bytes = prefix(&self.request)?;
        bytes.extend_from_slice(&self.wrapped);
        Ok(bytes)
    }
    pub(super) fn decode(
        request: ChatMaterialRequest,
        bytes: &[u8],
    ) -> Result<Self, AccountMaterialError> {
        if bytes.len() > MAX_RECORD {
            return Err(AccountMaterialError::Capacity);
        }
        let expected = prefix(&request)?;
        if bytes.len() != expected.len() + WRAPPED_KEY || !bytes.starts_with(&expected) {
            return Err(AccountMaterialError::Binding);
        }
        Ok(Self::new(request, bytes[expected.len()..].to_vec()))
    }
}
pub(super) fn coordinate(
    request: &ChatMaterialRequest,
) -> Result<ChatCoordinate, AccountMaterialError> {
    let stream = &request.stream;
    let binding = JournalCatalogBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
    );
    ChatCoordinate::new(
        crate::cloud::metadata_crypto::namespace(&binding).map_err(super::cause)?,
        crate::cloud::metadata_crypto::runner(&binding),
        *stream.chat().as_bytes(),
        request.epoch.get(),
    )
    .map_err(super::cause)
}
pub(super) fn object_id(request: &ChatMaterialRequest) -> Result<String, AccountMaterialError> {
    // Pins are authenticated payload, not selectors: changing a pin must conflict
    // with the existing immutable object instead of creating another key.
    let stream = &request.stream;
    let mut hash = Sha256::new();
    hash.update(
        coordinate(request)?
            .canonical_bytes()
            .map_err(super::cause)?,
    );
    hash.update(stream.scope().hub().authority().as_bytes());
    hash.update(stream.scope().account().key().as_bytes());
    Ok(format!("{:x}", hash.finalize()))
}
/// Stable account/hub+kind identity; runner/chat selectors never choose another
/// personal collection. UUIDv8 preserves the public Store identifier constraint.
pub(super) fn collection_id(
    scope: &nocterm_cloud_model::identity::AccountScope,
) -> Result<String, AccountMaterialError> {
    collection_id_of(scope, KIND)
}
/// The same identity for another personal object kind of the account.
pub(super) fn collection_id_of(
    scope: &nocterm_cloud_model::identity::AccountScope,
    kind: &str,
) -> Result<String, AccountMaterialError> {
    let mut hash = Sha256::new();
    hash.update(b"nocterm/cloud/account-material-collection/v1");
    for field in [
        scope.hub().instance().as_str().as_bytes(),
        scope.account().id().as_str().as_bytes(),
        kind.as_bytes(),
    ] {
        hash.update(
            u64::try_from(field.len())
                .map_err(super::cause)?
                .to_be_bytes(),
        );
        hash.update(field);
    }
    hash.update(scope.hub().authority().as_bytes());
    hash.update(scope.account().key().as_bytes());
    let digest = hash.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().map_err(super::cause)?;
    Ok(uuid::Builder::from_custom_bytes(bytes)
        .into_uuid()
        .to_string())
}

fn prefix(request: &ChatMaterialRequest) -> Result<Vec<u8>, AccountMaterialError> {
    let stream = &request.stream;
    let hub = stream.scope().hub().instance().as_str().as_bytes();
    let account = stream.scope().account().id().as_str().as_bytes();
    if hub.len() > 128 || account.len() > 128 {
        return Err(AccountMaterialError::Capacity);
    }
    let mut bytes = Vec::with_capacity(MAX_RECORD - WRAPPED_KEY);
    bytes.push(1);
    bytes.push(u8::try_from(hub.len()).map_err(super::cause)?);
    bytes.extend_from_slice(hub);
    bytes.push(u8::try_from(account.len()).map_err(super::cause)?);
    bytes.extend_from_slice(account);
    bytes.extend_from_slice(stream.scope().hub().authority().as_bytes());
    bytes.extend_from_slice(stream.scope().account().key().as_bytes());
    bytes.extend_from_slice(stream.runner().as_bytes());
    bytes.extend_from_slice(&stream.runner_epoch().get().to_be_bytes());
    bytes.extend_from_slice(stream.chat().as_bytes());
    bytes.extend_from_slice(&request.epoch.get().to_be_bytes());
    bytes.extend_from_slice(&request.pins.encryption);
    bytes.extend_from_slice(&request.pins.signing);
    bytes.extend_from_slice(&request.pins.proof);
    if bytes.len() + WRAPPED_KEY > MAX_RECORD {
        return Err(AccountMaterialError::Capacity);
    }
    Ok(bytes)
}
