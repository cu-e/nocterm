//! Durable original account-authenticated grants, restored before inbox admission.
mod file;
use super::super::{CloudRunnerIdentity, RunnerJournalCrypto, RunnerMaterialError};
use file::MaterialFile;
use nocterm_cloud_model::{
    authority::StreamBinding,
    identity::{ChatKeyEpoch, CloudChatId},
    journal::{JournalCatalogBinding, JournalCipher},
};
use nocterm_core::SharedCause;
use nocterm_hub_model::cloud::ChatCoordinate;
use nocterm_keys::{PublicKey, cloud::open_chat_key_from_account};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

pub(super) const MAX_MATERIALS: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum MaterialStoreError {
    #[error("the material does not belong to the configured original runner/account")]
    Binding,
    #[error("the bounded material store is full or its input is oversized")]
    Capacity,
    #[error("this material slot requires explicit verified rotation")]
    Conflict,
    #[error("the material generation has been durably retired")]
    Retired,
    #[error("the private material record is malformed or inconsistent")]
    Format,
    #[error("the material store awaits durable publication or is unavailable")]
    Unavailable,
    #[error("the private material record could not be accessed")]
    Storage(#[source] SharedCause),
    #[error("the material record was published but durability is unconfirmed")]
    StoredButUnavailable(#[source] SharedCause),
    #[error("account-authenticated material could not be opened")]
    Crypto(#[source] SharedCause),
    #[error(transparent)]
    Identity(#[from] super::super::CloudRunnerIdentityError),
    #[error(transparent)]
    Material(#[from] RunnerMaterialError),
}

/// Plain native persistence data; the content key itself never enters this record.
#[derive(Clone)]
struct StoredGrant {
    epoch: ChatKeyEpoch,
    account: [u8; 32],
    original_delivery: [u8; 16],
    digest: [u8; 32],
    sealed: Vec<u8>,
    retired: bool,
}
struct StoreState {
    file: MaterialFile,
    entries: BTreeMap<CloudChatId, StoredGrant>,
    pending: Option<BTreeMap<CloudChatId, StoredGrant>>,
}

/// Sole writer for initial material and retirement. Construct the journal/event
/// crypto collaborators from `journal_cipher` and `event_encoder`, never from a
/// replacement identity or a public content-key installation path.
pub struct RunnerMaterialStore {
    binding: JournalCatalogBinding,
    identity: Arc<CloudRunnerIdentity>,
    crypto: Arc<RunnerJournalCrypto>,
    maximum: usize,
    state: Mutex<StoreState>,
}
impl RunnerMaterialStore {
    pub fn create_new(
        directory: &Path,
        binding: JournalCatalogBinding,
        identity: Arc<CloudRunnerIdentity>,
        maximum: usize,
    ) -> Result<Arc<Self>, MaterialStoreError> {
        Self::open(directory, binding, identity, maximum, true)
    }
    pub fn open_existing(
        directory: &Path,
        binding: JournalCatalogBinding,
        identity: Arc<CloudRunnerIdentity>,
        maximum: usize,
    ) -> Result<Arc<Self>, MaterialStoreError> {
        Self::open(directory, binding, identity, maximum, false)
    }
    fn open(
        directory: &Path,
        binding: JournalCatalogBinding,
        identity: Arc<CloudRunnerIdentity>,
        maximum: usize,
        create: bool,
    ) -> Result<Arc<Self>, MaterialStoreError> {
        if maximum == 0 || maximum > MAX_MATERIALS {
            return Err(MaterialStoreError::Capacity);
        }
        if identity.binding() != &binding {
            return Err(MaterialStoreError::Binding);
        }
        let public = identity.public_identity()?;
        let mut file = MaterialFile::acquire(directory, &binding, public.encryption_key())?;
        let entries = if create {
            BTreeMap::new()
        } else {
            file.read(maximum)?
        };
        if create {
            file.save(&entries)?;
        }
        let crypto = Arc::new(RunnerJournalCrypto::new(identity.clone(), maximum)?);
        let owner = Arc::new(Self {
            binding,
            identity,
            crypto,
            maximum,
            state: Mutex::new(StoreState {
                file,
                entries,
                pending: None,
            }),
        });
        owner.restore_live()?;
        Ok(owner)
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn journal_cipher(&self) -> Arc<dyn JournalCipher> {
        self.crypto.clone()
    }
    pub fn event_encoder(
        &self,
        clock: Arc<dyn nocterm_core::clock::WallClock>,
    ) -> Arc<super::super::RunnerEventEncoder> {
        Arc::new(super::super::RunnerEventEncoder::new(
            self.crypto.clone(),
            clock,
        ))
    }
    pub(super) fn identity(&self) -> &Arc<CloudRunnerIdentity> {
        &self.identity
    }

    /// Authentication is repeated here, before persistence; DTO shape alone
    /// never causes a material file write or content-key installation.
    pub(super) fn admit(
        &self,
        stream: &StreamBinding,
        epoch: ChatKeyEpoch,
        account: [u8; 32],
        delivery: [u8; 16],
        sealed: &[u8],
    ) -> Result<(), MaterialStoreError> {
        self.authenticate(stream, epoch, account, sealed)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterialStoreError::Unavailable)?;
        if state.pending.is_some() {
            return Err(MaterialStoreError::Unavailable);
        }
        if state.entries.iter().any(|(chat, original)| {
            original.original_delivery == delivery
                && (*chat != stream.chat()
                    || original.epoch != epoch
                    || original.account != account
                    || original.sealed != sealed)
        }) {
            return Err(MaterialStoreError::Conflict);
        }
        if let Some(original) = state.entries.get(&stream.chat()) {
            if original.epoch != epoch || original.account != account || original.sealed != sealed {
                return Err(MaterialStoreError::Conflict);
            }
            if original.retired {
                return Err(MaterialStoreError::Retired);
            }
            return Ok(());
        }
        if state.entries.len() >= self.maximum {
            return Err(MaterialStoreError::Capacity);
        }
        let mut candidate = state.entries.clone();
        candidate.insert(
            stream.chat(),
            StoredGrant {
                epoch,
                account,
                original_delivery: delivery,
                digest: Sha256::digest(sealed).into(),
                sealed: sealed.to_vec(),
                retired: false,
            },
        );
        state.pending = Some(candidate);
        persist_pending(&mut state)?;
        self.identity.with_keys(|_| ())?;
        self.crypto.install_grant(stream, epoch, account, sealed)?;
        Ok(())
    }

    /// A late retirement cannot remove another generation. The encrypted grant
    /// and its tombstone stay charged and survive a runner restart.
    pub fn retire_chat(
        &self,
        stream: &StreamBinding,
        epoch: ChatKeyEpoch,
    ) -> Result<bool, MaterialStoreError> {
        self.require_stream(stream)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterialStoreError::Unavailable)?;
        if state.pending.is_some() {
            return Err(MaterialStoreError::Unavailable);
        }
        let Some(old) = state.entries.get(&stream.chat()) else {
            return Ok(false);
        };
        if old.epoch != epoch || old.retired {
            return Ok(false);
        }
        let mut candidate = state.entries.clone();
        candidate
            .get_mut(&stream.chat())
            .ok_or(MaterialStoreError::Format)?
            .retired = true;
        state.pending = Some(candidate);
        // Immediate local fencing precedes disk I/O; a write failure never revives
        // the key in this runtime. Persistent failure remains explicitly pending.
        self.crypto.retire_chat(stream, epoch)?;
        persist_pending(&mut state)?;
        Ok(true)
    }
    pub fn retry_pending(&self) -> Result<(), MaterialStoreError> {
        self.identity.with_keys(|_| ())?;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| MaterialStoreError::Unavailable)?;
            persist_pending(&mut state)?;
        }
        self.restore_live()
    }
    fn restore_live(&self) -> Result<(), MaterialStoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| MaterialStoreError::Unavailable)?;
        if state.pending.is_some() {
            return Err(MaterialStoreError::Unavailable);
        }
        for (chat, grant) in &state.entries {
            let stream = self.binding.stream(*chat);
            self.authenticate(&stream, grant.epoch, grant.account, &grant.sealed)?;
            if grant.retired {
                continue;
            }
            match self
                .crypto
                .install_grant(&stream, grant.epoch, grant.account, &grant.sealed)
            {
                Ok(()) | Err(RunnerMaterialError::Existing) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    fn require_stream(&self, stream: &StreamBinding) -> Result<(), MaterialStoreError> {
        self.identity.with_keys(|_| ())?;
        if !self.binding.contains(stream) {
            return Err(MaterialStoreError::Binding);
        }
        Ok(())
    }
    fn authenticate(
        &self,
        stream: &StreamBinding,
        epoch: ChatKeyEpoch,
        account: [u8; 32],
        sealed: &[u8],
    ) -> Result<(), MaterialStoreError> {
        self.require_stream(stream)?;
        if sealed.is_empty() || sealed.len() > 256 {
            return Err(MaterialStoreError::Capacity);
        }
        if <[u8; 32]>::from(Sha256::digest(account)) != *stream.scope().account().key().as_bytes() {
            return Err(MaterialStoreError::Binding);
        }
        let account = PublicKey::from_bytes(&account).map_err(crypto_error)?;
        let chat = ChatCoordinate::new(
            super::super::metadata_crypto::namespace(&self.binding).map_err(crypto_error)?,
            super::super::metadata_crypto::runner(&self.binding),
            *stream.chat().as_bytes(),
            epoch.get(),
        )
        .map_err(crypto_error)?;
        self.identity
            .with_keys(|keys| {
                open_chat_key_from_account(keys.encryption(), &account, &chat, sealed)
            })?
            .map_err(crypto_error)?;
        Ok(())
    }
}
fn persist_pending(state: &mut StoreState) -> Result<(), MaterialStoreError> {
    if let Some(candidate) = state.pending.as_ref() {
        state.file.save(candidate)?;
        state.entries = state
            .pending
            .take()
            .ok_or(MaterialStoreError::Unavailable)?;
    }
    Ok(())
}
fn crypto_error(error: impl std::error::Error + Send + Sync + 'static) -> MaterialStoreError {
    MaterialStoreError::Crypto(SharedCause::new(error))
}

#[cfg(test)]
mod tests;
