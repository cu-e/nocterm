//! The sealed checkpoints of the device core, one private file each: a chat's verified reader
//! (`VerifiedChat`) and a runner's verified directory (`VerifiedDirectory`).
//!
//! The core seals every checkpoint with a key derived from the account key, so a file holds
//! ciphertext only; one that does not open is ignored by the core and read again from the hub.
//! Files are published with `write_private_atomic` under a mode-0700 root, and a write never
//! replaces a newer one of the same file.
use super::storage_work::{StorageWork, StorageWorkError};
use nocterm_cloud_device::AccountScope;
use nocterm_core::fs::{BoundedReadError, FileError, read_regular_bounded, write_private_atomic};
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

#[path = "checkpoint_queue.rs"]
mod queue;
use queue::{MAX_CHECKPOINT_BYTES, PendingCheckpoints, QueueError};

/// The directory of this layout under the chats root.
const LAYOUT: &str = "device-v1";

/// Why a checkpoint could not be read or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CheckpointError {
    #[error(transparent)]
    Queue(#[from] QueueError),
    #[error(transparent)]
    Worker(#[from] StorageWorkError),
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

/// One installation's latest pending checkpoints and one ordered physical writer.
/// Pending work is bounded by files and bytes; accepted work retains this owner
/// through physical completion even after the flow that offered it closes.
pub(crate) struct Checkpoints {
    root: PathBuf,
    pending: Mutex<PendingCheckpoints>,
    drained: tokio::sync::Notify,
    work: Arc<StorageWork>,
}
impl Checkpoints {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root: root.join(LAYOUT),
            pending: Mutex::default(),
            drained: tokio::sync::Notify::new(),
            work: StorageWork::new(),
        }
    }
    pub(crate) fn with_work(mut self, work: Arc<StorageWork>) -> Self {
        self.work = work;
        self
    }
    fn path(&self, account: &AccountPlace, file: &str) -> PathBuf {
        self.root.join(&account.0).join(file)
    }
    /// Loads a bounded file without occupying a Tokio worker.
    pub(crate) async fn load_async(
        &self,
        account: &AccountPlace,
        file: &str,
    ) -> Result<Option<Vec<u8>>, CheckpointError> {
        let path = self.path(account, file);
        self.work.run(move || load(&path)).await?
    }
    /// Coalesces the newest snapshot for this exact account/file. A queue that is
    /// full refuses explicitly; no unbounded waiter or task retains rejected bytes.
    /// Submission order, rather than a flow-local counter, orders reopened flows.
    pub(crate) fn offer(
        self: &Arc<Self>,
        account: &AccountPlace,
        file: &str,
        bytes: Vec<u8>,
    ) -> Result<(), CheckpointError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| StorageWorkError::Unavailable)?;
        let start = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .offer(self.path(account, file), bytes)?;
        if start {
            let owner = self.clone();
            runtime.spawn_blocking(move || owner.drain());
        }
        Ok(())
    }
    fn drain(&self) {
        loop {
            let next = self
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let Some((path, bytes, writing)) = next else {
                self.drained.notify_waiters();
                return;
            };
            if let Err(error) = write(&self.root, &path, &bytes) {
                tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud checkpoint was not saved");
            }
            drop(writing);
            self.drained.notify_waiters();
        }
    }
    /// Waits for accepted disk work, including its failures, without preventing new
    /// offers. Actual I/O remains owned by the writer if this waiter is cancelled.
    #[cfg(test)]
    pub(crate) async fn flush(&self) {
        loop {
            let drained = self.drained.notified();
            tokio::pin!(drained);
            drained.as_mut().enable();
            if self
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .idle()
            {
                return;
            }
            drained.await;
        }
    }
}

fn load(path: &Path) -> Result<Option<Vec<u8>>, CheckpointError> {
    match read_regular_bounded(path, MAX_CHECKPOINT_BYTES) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => Ok(None),
        Err(error) => Err(CheckpointError::Read(error)),
    }
}
fn write(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), CheckpointError> {
    nocterm_core::paths::ensure_private_dir(root).map_err(CheckpointError::Directory)?;
    if let Some(parent) = path.parent() {
        nocterm_core::paths::ensure_private_dir(parent).map_err(CheckpointError::Directory)?;
    }
    write_private_atomic(path, bytes).map_err(CheckpointError::Write)
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

    #[tokio::test]
    async fn a_reopened_flow_can_keep_a_new_checkpoint_and_accounts_remain_separate() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoints = Arc::new(Checkpoints::new(dir.path().to_path_buf()));
        let account = AccountPlace("a".repeat(32));
        let other = AccountPlace("b".repeat(32));
        assert!(
            checkpoints
                .load_async(&account, "c.chat")
                .await
                .unwrap()
                .is_none()
        );
        checkpoints
            .offer(&account, "c.chat", b"earlier flow".to_vec())
            .unwrap();
        checkpoints.flush().await;
        checkpoints
            .offer(&account, "c.chat", b"reopened flow".to_vec())
            .unwrap();
        checkpoints
            .offer(&other, "c.chat", b"other account".to_vec())
            .unwrap();
        checkpoints.flush().await;
        assert_eq!(
            checkpoints
                .load_async(&account, "c.chat")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"reopened flow"[..])
        );
        assert_eq!(
            checkpoints
                .load_async(&other, "c.chat")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"other account"[..])
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

    #[tokio::test]
    async fn an_oversized_offer_does_not_replace_the_accepted_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoints = Arc::new(Checkpoints::new(dir.path().to_path_buf()));
        let account = AccountPlace::of_hex("a");
        checkpoints
            .offer(&account, "c.chat", b"kept".to_vec())
            .unwrap();
        checkpoints.flush().await;
        assert!(matches!(
            checkpoints.offer(&account, "c.chat", vec![0; MAX_CHECKPOINT_BYTES + 1]),
            Err(CheckpointError::Queue(QueueError::Oversized))
        ));
        checkpoints.flush().await;
        assert_eq!(
            checkpoints
                .load_async(&account, "c.chat")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"kept"[..])
        );
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
