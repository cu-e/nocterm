//! The encrypted copy of an account's keyset and vault kept on this device.
//!
//! One mode-0600 file per hub, `<host>_<port>.cache` beside the credential file. It holds only
//! what the hub holds: the sealed keyset, the sealed key envelope of the vault collection and
//! sealed objects, plus the non-secret facts needed to derive the unlock key again (the user
//! name, the way the key is derived and the hub's own identifier). Nothing secret in it opens
//! without the vault password, so it may sit on disk, and it lets the vault unlock while the hub
//! is unreachable.
//! Read and write share a 32 MiB byte limit. Personal-vault refresh publication
//! keeps the newest collection cursor and skips identical snapshots. Vault data
//! is shared immutably across readers; staging and parent sync use two physical
//! installation-owned workers, independent of the Slot's short authority lock.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use nocterm_core::{
    bounded_io::BoundedBuffer,
    fs::{AtomicPublication, BoundedReadError, FileError, read_regular_bounded},
};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubAddress;
use serde::{Deserialize, Serialize};

use crate::credentials::{MAX_NAME_BYTES, file_stem, prepare_private_directory};

/// The cache holds sealed credentials of one account; anything larger is not ours.
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
/// The format the file is written in. A newer file is left alone and not read.
const FORMAT: u32 = 2;

/// Why the cache of a hub could not be read or kept.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CacheError {
    #[error("the account cache cannot be read")]
    Read(#[from] BoundedReadError),
    #[error("the account cache cannot be written")]
    Write(#[from] FileError),
    #[error("the account cache directory cannot be prepared")]
    Prepare(#[source] io::Error),
    #[error("the account cache is not valid")]
    Format(#[source] serde_json::Error),
    #[error("the account cache was written by a newer version")]
    Newer,
    #[error("the account cache exceeds its 32 MiB limit")]
    Limit,
    #[error("the hub address is too long for a file name")]
    NameTooLong,
    #[error("the account cache worker is unavailable")]
    Worker,
    #[error("the account cache worker failed")]
    Join(#[from] tokio::task::JoinError),
}

/// How the unlock key of the account's keyset is derived from what the user types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum UnlockMethod {
    /// The login password of the account, bound to the hub and the user name.
    Login { username: String },
    /// A vault passphrase of its own (accounts that sign in through a provider).
    Passphrase,
}

/// Bytes as base64 text.
mod bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(value: &[u8], out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&STANDARD.encode(value))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(input)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// The sealed keyset as the hub stores it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeysetCopy {
    pub version: u64,
    pub kdf: String,
    #[serde(with = "bytes")]
    pub salt: Vec<u8>,
    #[serde(with = "bytes")]
    pub wrapped_account_key: Vec<u8>,
    #[serde(with = "bytes")]
    pub recovery_wrapped_account_key: Vec<u8>,
    /// Which password the unlock key comes from, as the hub's `UnlockKind` number.
    pub unlock_kind: i32,
}

impl KeysetCopy {
    pub(crate) fn of(record: &v2::Keyset) -> Self {
        Self {
            version: record.version,
            kdf: record.kdf.clone(),
            salt: record.salt.clone(),
            wrapped_account_key: record.wrapped_account_key.clone(),
            recovery_wrapped_account_key: record.recovery_wrapped_account_key.clone(),
            unlock_kind: record.unlock_kind,
        }
    }

    pub(crate) fn record(&self) -> v2::Keyset {
        v2::Keyset {
            version: self.version,
            kdf: self.kdf.clone(),
            salt: self.salt.clone(),
            wrapped_account_key: self.wrapped_account_key.clone(),
            updated_at_ms: 0,
            recovery_wrapped_account_key: self.recovery_wrapped_account_key.clone(),
            unlock_kind: self.unlock_kind,
        }
    }
}

/// One sealed object (or tombstone) of the vault collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ObjectCopy {
    pub version: u64,
    pub kind: String,
    #[serde(with = "bytes")]
    pub ciphertext: Vec<u8>,
    pub deleted: bool,
}

/// The vault collection: its key envelope and every object read so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VaultCopy {
    pub collection_id: String,
    #[serde(with = "bytes")]
    pub key_envelope: Vec<u8>,
    /// The stream version up to which `objects` is complete.
    pub cursor: u64,
    pub objects: BTreeMap<String, ObjectCopy>,
}

/// What the cache file holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccountCache {
    pub v: u32,
    /// The hub's own identifier, which the login key is derived with.
    pub hub_uid: String,
    pub account_id: String,
    pub method: UnlockMethod,
    pub keyset: KeysetCopy,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "shared_vault"
    )]
    pub vault: Option<Arc<VaultCopy>>,
    /// The keys were created on this device and the person has not yet confirmed saving the
    /// recovery key shown then. A fact about the device, not a secret: the next time the keys
    /// open, a new recovery key is offered.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recovery_unconfirmed: bool,
}

impl AccountCache {
    /// A delayed refresh never replaces a more recent personal-vault snapshot.
    /// Equal snapshots need neither another memory publication nor a disk write.
    pub(crate) fn accept_shared_vault(&mut self, next: Arc<VaultCopy>) -> bool {
        if self.vault.as_ref().is_some_and(|held| {
            held == &next || (held.collection_id == next.collection_id && held.cursor > next.cursor)
        }) {
            return false;
        }
        self.vault = Some(next);
        true
    }

    pub(crate) fn new(
        hub_uid: String,
        account_id: String,
        method: UnlockMethod,
        record: &v2::Keyset,
    ) -> Self {
        Self {
            v: FORMAT,
            hub_uid,
            account_id,
            method,
            keyset: KeysetCopy::of(record),
            vault: None,
            recovery_unconfirmed: false,
        }
    }
}

/// Sharing is an in-memory ownership detail; the existing file representation stays plain.
mod shared_vault {
    use super::{Arc, VaultCopy};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(
        value: &Option<Arc<VaultCopy>>,
        out: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_deref().serialize(out)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        input: D,
    ) -> Result<Option<Arc<VaultCopy>>, D::Error> {
        Option::<VaultCopy>::deserialize(input).map(|value| value.map(Arc::new))
    }
}

/// The cache file of one hub. It keeps no state: callers decide when to read and write.
#[derive(Clone, Debug)]
pub(crate) struct CacheFile {
    path: PathBuf,
    work: Arc<CacheWork>,
}

/// One installation shares two physical filesystem jobs. A cancelled waiter
/// releases admission; a running job retains its permit until the I/O finishes.
#[derive(Debug)]
pub(crate) struct CacheWork {
    runtime: Option<tokio::runtime::Handle>,
    slots: Arc<tokio::sync::Semaphore>,
    operations: Arc<tokio::sync::Semaphore>,
}

impl CacheWork {
    pub(crate) fn new(runtime: Option<tokio::runtime::Handle>) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
            operations: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }

    pub(crate) async fn operation(&self) -> Result<tokio::sync::OwnedSemaphorePermit, CacheError> {
        self.operations
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| CacheError::Worker)
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, CacheError> + Send + 'static,
    ) -> Result<T, CacheError> {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| CacheError::Worker)?;
        let runtime = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
            .ok_or(CacheError::Worker)?;
        runtime
            .spawn_blocking(move || {
                let _permit = permit;
                work()
            })
            .await?
    }
}

impl CacheFile {
    pub(crate) fn new(root: &std::path::Path, address: &HubAddress) -> Result<Self, CacheError> {
        let stem = file_stem(address);
        if stem.len() > MAX_NAME_BYTES {
            return Err(CacheError::NameTooLong);
        }
        Ok(Self {
            path: root.join(format!("{stem}.cache")),
            work: CacheWork::new(None),
        })
    }

    pub(crate) fn with_work(mut self, work: Arc<CacheWork>) -> Self {
        self.work = work;
        self
    }

    pub(crate) fn work(&self) -> &Arc<CacheWork> {
        &self.work
    }

    /// The cached account; `None` when nothing was kept for the hub.
    pub(crate) fn load(&self) -> Result<Option<AccountCache>, CacheError> {
        let bytes = match read_regular_bounded(&self.path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        #[derive(Deserialize)]
        struct Header {
            v: u32,
        }
        let header: Header = serde_json::from_slice(&bytes).map_err(CacheError::Format)?;
        if header.v > FORMAT {
            return Err(CacheError::Newer);
        }
        let cache: AccountCache = serde_json::from_slice(&bytes).map_err(CacheError::Format)?;
        Ok(Some(cache))
    }

    /// Replaces the file atomically; it is created readable by the user only.
    #[cfg(test)]
    pub(crate) fn save(&self, cache: &AccountCache) -> Result<(), CacheError> {
        self.stage(cache)?.publish()?;
        Ok(())
    }

    pub(crate) fn stage(&self, cache: &AccountCache) -> Result<AtomicPublication, CacheError> {
        if let Some(parent) = self.path.parent() {
            prepare_private_directory(parent).map_err(CacheError::Prepare)?;
        }
        let mut bytes = BoundedBuffer::new(MAX_FILE_BYTES);
        if let Err(error) = serde_json::to_writer(&mut bytes, cache) {
            return Err(if bytes.exceeded() {
                CacheError::Limit
            } else {
                CacheError::Format(error)
            });
        }
        Ok(AtomicPublication::stage_private(
            &self.path,
            &bytes.into_inner(),
        )?)
    }

    /// Removes the cache; no file is not an error.
    pub(crate) fn erase(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
