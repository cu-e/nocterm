//! The sealed checkpoints of the device core, one private file each: a chat's verified reader
//! (`VerifiedChat`) and a runner's verified directory (`VerifiedDirectory`).
//!
//! The core seals every checkpoint with a key derived from the account key, so a file holds
//! ciphertext only; one that does not open is ignored by the core and read again from the hub.
//! Files are published with `write_private_atomic` under a mode-0700 root, and a write never
//! replaces a newer one of the same file.
use nocterm_cloud_device::AccountScope;
use nocterm_core::fs::{BoundedReadError, FileError, read_regular_bounded, write_private_atomic};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// The largest checkpoint read back.
const MAX_CHECKPOINT_BYTES: usize = 32 * 1024 * 1024;
/// The directory of this layout under the chats root.
const LAYOUT: &str = "device-v1";

/// Why a checkpoint could not be read or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CheckpointError {
    #[error("the checkpoint directory could not be prepared")]
    Directory(#[source] io::Error),
    #[error("the checkpoint could not be written")]
    Write(#[source] FileError),
    #[error("the checkpoint could not be read")]
    Read(#[source] BoundedReadError),
}

/// Where one account's checkpoints live: a directory named by a digest of its scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AccountPlace(String);
impl AccountPlace {
    pub(crate) fn of(scope: &AccountScope) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"nocterm-desktop-cloud-checkpoints-v1");
        for part in [scope.hub_instance(), scope.account_id()] {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part.as_bytes());
        }
        hash.update(scope.account_key_digest());
        Self(
            hash.finalize()[..16]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }
}

impl AccountPlace {
    /// The directory name.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
    #[cfg(test)]
    pub(crate) fn of_hex(name: &str) -> Self {
        Self(name.to_owned())
    }
}

/// The checkpoint files of an installation.
pub(crate) struct Checkpoints {
    root: PathBuf,
    /// The newest generation written per file.
    written: Mutex<HashMap<PathBuf, u64>>,
}
impl Checkpoints {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root: root.join(LAYOUT),
            written: Mutex::new(HashMap::new()),
        }
    }
    fn path(&self, account: &AccountPlace, file: &str) -> PathBuf {
        self.root.join(&account.0).join(file)
    }
    /// The bytes of `file`, or `None` when it was never written.
    pub(crate) fn load(
        &self,
        account: &AccountPlace,
        file: &str,
    ) -> Result<Option<Vec<u8>>, CheckpointError> {
        match read_regular_bounded(&self.path(account, file), MAX_CHECKPOINT_BYTES) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => Ok(None),
            Err(error) => Err(CheckpointError::Read(error)),
        }
    }
    /// Publishes `bytes` as `file` unless a newer generation was written already.
    pub(crate) fn store(
        &self,
        account: &AccountPlace,
        file: &str,
        generation: u64,
        bytes: &[u8],
    ) -> Result<(), CheckpointError> {
        let path = self.path(account, file);
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if written
            .get(&path)
            .is_some_and(|newest| *newest >= generation)
        {
            return Ok(());
        }
        nocterm_core::paths::ensure_private_dir(&self.root).map_err(CheckpointError::Directory)?;
        if let Some(parent) = path.parent() {
            nocterm_core::paths::ensure_private_dir(parent).map_err(CheckpointError::Directory)?;
        }
        write_private_atomic(&path, bytes).map_err(CheckpointError::Write)?;
        written.insert(path, generation);
        Ok(())
    }
}

/// The file of a chat's checkpoint.
pub(crate) fn chat_file(chat: &nocterm_cloud_device::context::ChatRef) -> String {
    format!("{chat}.chat")
}

/// The file of a runner's directory checkpoint.
pub(crate) fn directory_file(runner: &[u8; 16]) -> String {
    let hex: String = runner.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex}.directory")
}

/// Removes the verified-page caches of the earlier layout (`<64 hex>/replay/verified-page.v1`):
/// the device core reads every chat into its own checkpoints instead. Links are not followed.
pub(crate) fn remove_earlier_pages(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let earlier = name.to_str().is_some_and(|name| {
            name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let directory = entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink());
        if !(earlier && directory) {
            continue;
        }
        let page = entry.path().join("replay").join("verified-page.v1");
        if std::fs::symlink_metadata(&page).is_ok_and(|meta| meta.is_file())
            && let Err(error) = std::fs::remove_file(&page)
        {
            tracing::warn!(%error, "an earlier Cloud chat page could not be removed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkpoint_is_read_back_and_an_older_write_never_replaces_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::new(dir.path().to_path_buf());
        let account = AccountPlace("a".repeat(32));
        assert!(checkpoints.load(&account, "c.chat").unwrap().is_none());
        checkpoints.store(&account, "c.chat", 2, b"newer").unwrap();
        checkpoints.store(&account, "c.chat", 1, b"older").unwrap();
        assert_eq!(
            checkpoints.load(&account, "c.chat").unwrap().as_deref(),
            Some(&b"newer"[..])
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(LAYOUT))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn earlier_verified_pages_are_removed_and_other_files_stay() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = dir.path().join("f".repeat(64)).join("replay");
        std::fs::create_dir_all(&earlier).unwrap();
        std::fs::write(earlier.join("verified-page.v1"), b"old").unwrap();
        std::fs::write(earlier.join("other"), b"kept").unwrap();
        remove_earlier_pages(dir.path());
        assert!(!earlier.join("verified-page.v1").exists());
        assert!(earlier.join("other").exists());
    }
}
