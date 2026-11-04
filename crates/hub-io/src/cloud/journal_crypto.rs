//! Captured runner material opens only one account's exact chat generations.
use super::{CloudRunnerIdentity, CloudRunnerIdentityError};
use nocterm_cloud_model::{
    authority::StreamBinding,
    identity::{ChatKeyEpoch, CloudChatId},
    journal::{JournalCipher, JournalCipherContext, JournalError, JournalStorageClass},
};
use nocterm_core::SharedCause;
use nocterm_hub_model::cloud::{ChatCoordinate, JournalClass, JournalCoordinate};
use nocterm_keys::{
    PublicKey,
    cloud::{ChatKey, open_chat_key_from_account},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const MAX_CHAT_KEYS: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum RunnerMaterialError {
    #[error("the material does not belong to the original account and runner")]
    Binding,
    #[error("the runner material capacity is exhausted or invalid")]
    Capacity,
    #[error("this chat generation is already installed or retired")]
    Existing,
    #[error("changing chat key generation requires a verified rotation baseline")]
    RotationRequired,
    #[error("the runner material owner is unavailable")]
    Unavailable,
    #[error(transparent)]
    Identity(#[from] CloudRunnerIdentityError),
    #[error("the encrypted material grant is invalid")]
    Grant(#[source] SharedCause),
    #[error("the pinned account public key is invalid")]
    AccountKey(#[source] SharedCause),
}

struct ChatMaterial {
    epoch: ChatKeyEpoch,
    key: Option<ChatKey>,
}
struct Materials {
    chats: BTreeMap<CloudChatId, ChatMaterial>,
    retired: bool,
}

/// This bounded owner receives only account-authenticated grants to the pinned R.
/// It has no account private/root key, implicit key generation or rotation fallback.
pub struct RunnerJournalCrypto {
    identity: Arc<CloudRunnerIdentity>,
    max_chats: usize,
    materials: Mutex<Materials>,
}
impl RunnerJournalCrypto {
    pub fn new(
        identity: Arc<CloudRunnerIdentity>,
        max_chats: usize,
    ) -> Result<Self, RunnerMaterialError> {
        if max_chats == 0 || max_chats > MAX_CHAT_KEYS {
            return Err(RunnerMaterialError::Capacity);
        }
        identity.public_identity()?;
        Ok(Self {
            identity,
            max_chats,
            materials: Mutex::new(Materials {
                chats: BTreeMap::new(),
                retired: false,
            }),
        })
    }

    /// Delivery dedup precedes this installation. An existing slot is never
    /// replaced, even by another authentic grant with the same descriptive epoch.
    pub fn install_grant(
        &self,
        stream: &StreamBinding,
        epoch: ChatKeyEpoch,
        pinned_account_key: [u8; 32],
        sealed: &[u8],
    ) -> Result<(), RunnerMaterialError> {
        self.require_stream(stream)?;
        if sealed.len() > 256 {
            return Err(RunnerMaterialError::Capacity);
        }
        let fingerprint: [u8; 32] = Sha256::digest(pinned_account_key).into();
        if &fingerprint != stream.scope().account().key().as_bytes() {
            return Err(RunnerMaterialError::Binding);
        }
        let account = PublicKey::from_bytes(&pinned_account_key)
            .map_err(|error| RunnerMaterialError::AccountKey(SharedCause::new(error)))?;
        let coordinate = self
            .coordinate(stream, epoch)
            .map_err(|_| RunnerMaterialError::Binding)?;
        self.identity.with_keys(|keys| {
            let mut materials = self
                .materials
                .lock()
                .map_err(|_| RunnerMaterialError::Unavailable)?;
            if materials.retired {
                return Err(RunnerMaterialError::Unavailable);
            }
            if let Some(existing) = materials.chats.get(&stream.chat()) {
                return Err(if existing.epoch == epoch {
                    RunnerMaterialError::Existing
                } else {
                    RunnerMaterialError::RotationRequired
                });
            }
            if materials.chats.len() >= self.max_chats {
                return Err(RunnerMaterialError::Capacity);
            }
            let key = open_chat_key_from_account(keys.encryption(), &account, &coordinate, sealed)
                .map_err(|error| RunnerMaterialError::Grant(SharedCause::new(error)))?;
            materials.chats.insert(
                stream.chat(),
                ChatMaterial {
                    epoch,
                    key: Some(key),
                },
            );
            Ok(())
        })?
    }

    /// A late retirement for another epoch cannot remove the current generation.
    /// The tombstone remains charged, preventing a replayed initial grant revival.
    pub fn retire_chat(
        &self,
        stream: &StreamBinding,
        expected: ChatKeyEpoch,
    ) -> Result<bool, RunnerMaterialError> {
        self.require_stream(stream)?;
        let mut materials = self
            .materials
            .lock()
            .map_err(|_| RunnerMaterialError::Unavailable)?;
        let Some(slot) = materials.chats.get_mut(&stream.chat()) else {
            return Ok(false);
        };
        if slot.epoch != expected {
            return Ok(false);
        }
        Ok(slot.key.take().is_some())
    }

    pub fn retire(&self) {
        let mut materials = self
            .materials
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        materials.retired = true;
        for slot in materials.chats.values_mut() {
            slot.key.take();
        }
    }

    fn require_stream(&self, stream: &StreamBinding) -> Result<(), RunnerMaterialError> {
        if self.identity.binding().contains(stream) {
            Ok(())
        } else {
            Err(RunnerMaterialError::Binding)
        }
    }
    fn coordinate(
        &self,
        stream: &StreamBinding,
        epoch: ChatKeyEpoch,
    ) -> Result<ChatCoordinate, JournalError> {
        let binding = self.identity.binding();
        ChatCoordinate::new(
            super::metadata_crypto::namespace(binding)?,
            super::metadata_crypto::runner(binding),
            *stream.chat().as_bytes(),
            epoch.get(),
        )
        .map_err(storage_error)
    }
    fn with_key<T>(
        &self,
        stream: &StreamBinding,
        action: impl FnOnce(&ChatKey, &ChatCoordinate) -> Result<T, JournalError>,
    ) -> Result<T, JournalError> {
        self.with_material(stream, |key, coordinate, _| action(key, coordinate))
    }
    pub(super) fn with_material<T>(
        &self,
        stream: &StreamBinding,
        action: impl FnOnce(
            &ChatKey,
            &ChatCoordinate,
            &nocterm_keys::cloud::RunnerSigningKey,
        ) -> Result<T, JournalError>,
    ) -> Result<T, JournalError> {
        self.require_stream(stream)
            .map_err(|_| JournalError::Fenced)?;
        self.identity
            .with_keys(|keys| {
                let materials = self.materials.lock().map_err(|_| JournalError::Fenced)?;
                if materials.retired {
                    return Err(JournalError::Fenced);
                }
                let slot = materials
                    .chats
                    .get(&stream.chat())
                    .ok_or(JournalError::Fenced)?;
                let key = slot.key.as_ref().ok_or(JournalError::Fenced)?;
                action(key, &self.coordinate(stream, slot.epoch)?, keys.signing())
            })
            .map_err(storage_error)?
    }
}

impl JournalCipher for RunnerJournalCrypto {
    fn chat_key_epoch(&self, stream: &StreamBinding) -> Result<ChatKeyEpoch, JournalError> {
        self.with_key(stream, |_, coordinate| {
            ChatKeyEpoch::new(coordinate.key_epoch()).map_err(|_| JournalError::Corrupt)
        })
    }
    fn seal(
        &self,
        context: &JournalCipherContext,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        self.with_key(context.stream(), |key, coordinate| {
            key.seal_journal(&journal_coordinate(coordinate.clone(), context)?, plaintext)
                .map_err(storage_error)
        })
    }
    fn open(&self, context: &JournalCipherContext, sealed: &[u8]) -> Result<Vec<u8>, JournalError> {
        let mut plaintext = self.with_key(context.stream(), |key, coordinate| {
            key.open_journal(&journal_coordinate(coordinate.clone(), context)?, sealed)
                .map_err(storage_error)
        })?;
        Ok(std::mem::take(&mut *plaintext))
    }
}

fn journal_coordinate(
    chat: ChatCoordinate,
    context: &JournalCipherContext,
) -> Result<JournalCoordinate, JournalError> {
    let class = match context.class() {
        JournalStorageClass::Header => JournalClass::Header,
        JournalStorageClass::Record => JournalClass::Record,
        JournalStorageClass::CommandReceipt => JournalClass::CommandReceipt,
        JournalStorageClass::Checkpoint => JournalClass::Checkpoint,
        JournalStorageClass::Snapshot => JournalClass::Snapshot,
        JournalStorageClass::Outbox => JournalClass::Outbox,
        JournalStorageClass::BackgroundCommand => JournalClass::BackgroundCommand,
        JournalStorageClass::Effect => JournalClass::Effect,
        JournalStorageClass::RunHead => JournalClass::RunHead,
        // The contract has no SSH row class yet: the runner journal seals it as an effect
        // slot under one reserved slot id that no intent id can take (ADR 0137).
        JournalStorageClass::SshAccess => JournalClass::Effect,
    };
    JournalCoordinate::new(
        chat,
        class,
        *context.id(),
        context.sequence().get(),
        context.revision().get(),
        context.writer().get(),
    )
    .map_err(storage_error)
}
fn storage_error(error: impl std::error::Error + Send + Sync + 'static) -> JournalError {
    JournalError::Storage(SharedCause::new(error))
}

#[cfg(test)]
mod tests;
