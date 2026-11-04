//! Contiguous locally durable inbox prefix; never inferred from a scan watermark.
use crate::cloud::private_snapshot::{PrivateSnapshot, SnapshotError};
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_core::SharedCause;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

#[derive(Debug, thiserror::Error)]
pub(super) enum CursorError {
    #[error("the original private inbox cursor is malformed or moved")]
    Format,
    #[error("inbox prefix cannot skip a hole or replace pending durability")]
    Conflict,
    #[error("private inbox cursor access failed")]
    Storage(#[source] SharedCause),
    #[error("the inbox cursor was published but durability is unconfirmed")]
    Stored(#[source] SharedCause),
}
const MAGIC: &[u8; 8] = b"NCRIC001";
const BYTES: usize = 48;

pub(super) struct InboxCursor {
    file: PrivateSnapshot,
    source: [u8; 32],
    confirmed: u64,
    pending: Option<u64>,
}
impl InboxCursor {
    pub(super) fn open(
        root: &Path,
        binding: &JournalCatalogBinding,
        encryption: &[u8; 32],
        create: bool,
    ) -> Result<Self, CursorError> {
        let file = PrivateSnapshot::acquire(root, "runner-inbox.lock", "runner-inbox.v1")
            .map_err(snapshot)?;
        let runner = binding.runner();
        let epoch = binding.epoch().get().to_be_bytes();
        let mut hash = Sha256::new();
        hash.update(b"nocterm-private-inbox-source-v1");
        for field in [
            binding.scope().hub().instance().as_str().as_bytes(),
            binding.scope().hub().authority().as_bytes(),
            binding.scope().account().id().as_str().as_bytes(),
            binding.scope().account().key().as_bytes(),
            runner.as_bytes(),
            epoch.as_slice(),
            encryption.as_slice(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        let mut owner = Self {
            file,
            source: hash.finalize().into(),
            confirmed: 0,
            pending: None,
        };
        if create {
            owner.write(0)?;
        } else {
            owner.read()?;
        }
        Ok(owner)
    }
    pub(super) fn confirmed(&self) -> u64 {
        self.confirmed
    }
    pub(super) fn advance(&mut self, cursor: u64) -> Result<(), CursorError> {
        if cursor <= self.confirmed {
            return Ok(());
        }
        if self.confirmed.checked_add(1) != Some(cursor)
            || self.pending.is_some_and(|old| old != cursor)
        {
            return Err(CursorError::Conflict);
        }
        self.write(cursor)
    }
    fn read(&mut self) -> Result<(), CursorError> {
        let mut file = self.file.open().map_err(storage)?;
        let mut bytes = [0u8; BYTES + 1];
        let mut used = 0;
        loop {
            if used == bytes.len() {
                return Err(CursorError::Format);
            }
            let count = file.read(&mut bytes[used..]).map_err(storage)?;
            if count == 0 {
                break;
            }
            used += count;
        }
        if used != BYTES || &bytes[..8] != MAGIC || bytes[8..40] != self.digest(&bytes[40..48]) {
            return Err(CursorError::Format);
        }
        self.file.accept_read(&file).map_err(snapshot)?;
        self.confirmed =
            u64::from_be_bytes(bytes[40..48].try_into().map_err(|_| CursorError::Format)?);
        Ok(())
    }
    fn write(&mut self, cursor: u64) -> Result<(), CursorError> {
        self.pending = Some(cursor);
        let mut bytes = [0u8; BYTES];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[40..48].copy_from_slice(&cursor.to_be_bytes());
        let digest = self.digest(&bytes[40..48]);
        bytes[8..40].copy_from_slice(&digest);
        self.file.save(&bytes).map_err(snapshot)?;
        self.confirmed = cursor;
        self.pending = None;
        Ok(())
    }
    fn digest(&self, cursor: &[u8]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"nocterm-private-inbox-prefix-v1");
        hash.update(self.source);
        hash.update(cursor);
        hash.finalize().into()
    }
}
fn storage(error: impl std::error::Error + Send + Sync + 'static) -> CursorError {
    CursorError::Storage(SharedCause::new(error))
}
fn snapshot(error: SnapshotError) -> CursorError {
    match error {
        SnapshotError::Stored(cause) => CursorError::Stored(cause),
        error => storage(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::identity::*;

    fn binding(account: &str) -> JournalCatalogBinding {
        JournalCatalogBinding::new(
            AccountScope::new(
                HubIdentity::new(
                    NamespaceId::new("hub").unwrap(),
                    Fingerprint::from_bytes([1; 32]),
                ),
                AccountIdentity::new(
                    NamespaceId::new(account).unwrap(),
                    Fingerprint::from_bytes([2; 32]),
                ),
            ),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(4).unwrap(),
        )
    }

    #[test]
    fn inbox_prefix_survives_restart_without_crossing_holes_or_changing_original_source() {
        let directory = tempfile::tempdir().unwrap();
        let original = binding("original");
        let mut cursor = InboxCursor::open(directory.path(), &original, &[5; 32], true).unwrap();
        assert!(cursor.advance(2).is_err());
        assert_eq!(cursor.confirmed(), 0);
        cursor.advance(1).unwrap();
        assert!(InboxCursor::open(directory.path(), &original, &[5; 32], false).is_err());
        drop(cursor);
        let mut restored = InboxCursor::open(directory.path(), &original, &[5; 32], false).unwrap();
        assert_eq!(restored.confirmed(), 1);
        restored.advance(1).unwrap();
        assert!(restored.advance(3).is_err());
        restored.advance(2).unwrap();
        drop(restored);
        assert!(InboxCursor::open(directory.path(), &binding("other"), &[5; 32], false).is_err());
        assert!(InboxCursor::open(directory.path(), &original, &[6; 32], false).is_err());
        assert!(InboxCursor::open(directory.path(), &original, &[5; 32], true).is_err());
        assert_eq!(
            InboxCursor::open(directory.path(), &original, &[5; 32], false)
                .unwrap()
                .confirmed(),
            2
        );
    }
    #[test]
    fn explicit_identity_inbox_initialization_never_resets_an_existing_prefix() {
        use crate::cloud::{CloudRunnerIdentity, RunnerInbox};
        let directory = tempfile::tempdir().unwrap();
        let binding = binding("original");
        let identity =
            CloudRunnerIdentity::create_new(&directory.path().join("identity"), binding.clone())
                .unwrap();
        let inbox = directory.path().join("inbox");
        RunnerInbox::initialize_storage(&inbox, &identity).unwrap();
        let public = identity.public_identity().unwrap();
        let mut cursor =
            InboxCursor::open(&inbox, &binding, public.encryption_key(), false).unwrap();
        cursor.advance(1).unwrap();
        drop(cursor);
        assert!(RunnerInbox::initialize_storage(&inbox, &identity).is_err());
        assert_eq!(
            InboxCursor::open(&inbox, &binding, public.encryption_key(), false)
                .unwrap()
                .confirmed(),
            1
        );
    }
}
