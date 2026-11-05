//! The commands of a chat on their way, kept so that a command signed before the application
//! quit is resumed after it starts.
//!
//! A `Deliver` flow seals a command once and publishes exactly those bytes; the hub delivers a
//! publication id at most once. When a delivery first asks to publish, the core hands out the
//! command as a `StoredCommand` (sealed with a key of the account and chat, so a file holds no
//! prompt text), which is written here under the chat and the command id and removed once the
//! delivery is done or failed. When the chat is read again, `OpenChat::resume` tracks each
//! stored command's pending row again and resumes its delivery with the same bytes and a
//! receipt probe; a command that does not open, or has nothing left to deliver, is forgotten.
use super::checkpoints::AccountPlace;
use super::storage_work::{StorageWork, StorageWorkError};
use nocterm_cloud_device::command::StoredCommand;
use nocterm_cloud_device::command::stored::MAX_STORED_COMMAND_PLAINTEXT_BYTES;
use nocterm_cloud_device::context::ChatRef;
use nocterm_core::fs::{
    BoundedReadError, lock_private, read_regular_bounded, write_private_atomic,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

/// The sealed form adds a nonce and a tag to the plaintext.
const MAX_FILE_BYTES: usize = MAX_STORED_COMMAND_PLAINTEXT_BYTES + 64;
const LAYOUT: &str = "device-v1";
const EXTENSION: &str = "command";
const MAX_RESUMED_COMMANDS: usize = 32;
const MAX_RESUMED_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 256;

/// A stored command operation did not complete; publication must not bypass it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OutboxError {
    #[error(transparent)]
    Worker(#[from] StorageWorkError),
    #[error("the Cloud command outbox could not be read or changed")]
    Io(#[from] io::Error),
    #[error("the Cloud command could not be published in its private outbox file")]
    Write(#[source] nocterm_core::fs::FileError),
    #[error("the Cloud command outbox could not take its private publication lock")]
    Lock(#[source] nocterm_core::fs::FileError),
    #[error("the Cloud command outbox exceeds its bounded resume capacity")]
    Capacity,
}

/// The outbox files of an installation.
pub(crate) struct Outbox {
    root: PathBuf,
    work: Arc<StorageWork>,
}
impl Outbox {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root: root.join(LAYOUT),
            work: StorageWork::new(),
        }
    }
    pub(crate) fn with_work(mut self, work: Arc<StorageWork>) -> Self {
        self.work = work;
        self
    }
    pub(crate) async fn keep_async(
        self: &Arc<Self>,
        account: AccountPlace,
        chat: ChatRef,
        command_id: [u8; 16],
        command: StoredCommand,
    ) -> Result<(), OutboxError> {
        if command.as_bytes().len() > MAX_FILE_BYTES.min(MAX_RESUMED_BYTES) {
            return Err(OutboxError::Capacity);
        }
        let bytes = command.into_bytes().into_boxed_slice();
        let owner = self.clone();
        self.work
            .run(move || owner.keep(&account, &chat, &command_id, &bytes))
            .await?
    }
    pub(crate) async fn settled_async(
        self: &Arc<Self>,
        account: AccountPlace,
        chat: ChatRef,
        command_id: [u8; 16],
    ) -> Result<(), OutboxError> {
        let owner = self.clone();
        self.work
            .run(move || owner.settled(&account, &chat, &command_id))
            .await?
    }
    pub(crate) async fn left_async(
        self: &Arc<Self>,
        account: AccountPlace,
        chat: ChatRef,
    ) -> Result<Vec<([u8; 16], StoredCommand)>, OutboxError> {
        let owner = self.clone();
        self.work.run(move || owner.left(&account, &chat)).await?
    }
    fn directory(&self, account: &AccountPlace, chat: &ChatRef) -> PathBuf {
        self.root
            .join(account.as_str())
            .join("outbox")
            .join(chat.to_string())
    }
    fn file(&self, account: &AccountPlace, chat: &ChatRef, command_id: &[u8; 16]) -> PathBuf {
        let hex: String = command_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.directory(account, chat)
            .join(format!("{hex}.{EXTENSION}"))
    }
    /// Keeps the signed command `command_id` of `chat` before it is first published.
    fn keep(
        &self,
        account: &AccountPlace,
        chat: &ChatRef,
        command_id: &[u8; 16],
        bytes: &[u8],
    ) -> Result<(), OutboxError> {
        if bytes.len() > MAX_FILE_BYTES.min(MAX_RESUMED_BYTES) {
            return Err(OutboxError::Capacity);
        }
        let _lock = self.lock(account, chat)?;
        self.admit(account, chat, command_id, bytes.len())?;
        write_private_atomic(&self.file(account, chat, command_id), bytes)
            .map_err(OutboxError::Write)
    }
    fn lock(&self, account: &AccountPlace, chat: &ChatRef) -> Result<std::fs::File, OutboxError> {
        nocterm_core::paths::ensure_private_dir(&self.root)?;
        let directory = self.directory(account, chat);
        nocterm_core::paths::ensure_private_dir(&directory)?;
        lock_private(&directory.join("outbox.lock")).map_err(OutboxError::Lock)
    }
    /// Files remain the only byte/count authority; the retained per-chat OS lock spans this
    /// bounded scan and atomic publication, including independently constructed installations.
    fn admit(
        &self,
        account: &AccountPlace,
        chat: &ChatRef,
        replacement: &[u8; 16],
        incoming: usize,
    ) -> Result<(), OutboxError> {
        let mut bytes = incoming;
        let mut commands = 1usize;
        let destination = self.file(account, chat, replacement);
        let destination_name = destination.file_name().ok_or(OutboxError::Capacity)?;
        for (index, entry) in std::fs::read_dir(self.directory(account, chat))?.enumerate() {
            if index >= MAX_DIRECTORY_ENTRIES {
                return Err(OutboxError::Capacity);
            }
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            if name.to_str().and_then(command_id).is_none() {
                continue;
            }
            if name.as_os_str() == destination_name {
                continue;
            }
            let length =
                usize::try_from(entry.metadata()?.len()).map_err(|_| OutboxError::Capacity)?;
            bytes = bytes.checked_add(length).ok_or(OutboxError::Capacity)?;
            commands += 1;
            if bytes > MAX_RESUMED_BYTES || commands > MAX_RESUMED_COMMANDS {
                return Err(OutboxError::Capacity);
            }
        }
        Ok(())
    }
    /// Forgets a command whose delivery ended; absence is already settled.
    fn settled(
        &self,
        account: &AccountPlace,
        chat: &ChatRef,
        command_id: &[u8; 16],
    ) -> Result<(), OutboxError> {
        let _lock = self.lock(account, chat)?;
        match std::fs::remove_file(self.file(account, chat, command_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    /// Reads a finite directory and byte budget. Refusal leaves every file in
    /// place, so a retry never silently forgets an admitted signed command.
    fn left(
        &self,
        account: &AccountPlace,
        chat: &ChatRef,
    ) -> Result<Vec<([u8; 16], StoredCommand)>, OutboxError> {
        let _lock = self.lock(account, chat)?;
        let entries = match std::fs::read_dir(self.directory(account, chat)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut commands = Vec::new();
        let mut bytes = 0usize;
        for (index, entry) in entries.enumerate() {
            if index >= MAX_DIRECTORY_ENTRIES {
                return Err(OutboxError::Capacity);
            }
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(command_id)
            else {
                continue;
            };
            if commands.len() >= MAX_RESUMED_COMMANDS {
                return Err(OutboxError::Capacity);
            }
            let cap = MAX_FILE_BYTES.min(MAX_RESUMED_BYTES.saturating_sub(bytes));
            let sealed = match read_regular_bounded(&path, cap) {
                Ok(sealed) => sealed,
                Err(BoundedReadError::TooLarge { .. }) => return Err(OutboxError::Capacity),
                Err(_) => continue,
            };
            bytes = bytes.saturating_add(sealed.capacity());
            if bytes > MAX_RESUMED_BYTES {
                return Err(OutboxError::Capacity);
            }
            commands.push((id, StoredCommand::from_bytes(sealed)));
        }
        Ok(commands)
    }
}

/// The command id of an outbox file name (`<32 hex>.command`).
fn command_id(name: &str) -> Option<[u8; 16]> {
    let hex = name.strip_suffix(EXTENSION)?.strip_suffix('.')?;
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: u8) -> ChatRef {
        format!("{}-{}-1", "ab".repeat(16), format!("{id:02x}").repeat(16))
            .parse()
            .unwrap()
    }

    #[tokio::test]
    async fn a_kept_command_reads_back_under_its_chat_with_exactly_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Arc::new(Outbox::new(dir.path().to_path_buf()));
        let account = AccountPlace::of_hex("ab");
        let command = StoredCommand::from_bytes(vec![1, 2, 3]);
        outbox
            .keep_async(account.clone(), chat(1), [7; 16], command)
            .await
            .unwrap();
        let left = outbox.left_async(account.clone(), chat(1)).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, [7; 16]);
        assert_eq!(left[0].1.as_bytes(), [1, 2, 3]);
        assert!(
            outbox
                .left_async(account.clone(), chat(2))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_settled_command_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Arc::new(Outbox::new(dir.path().to_path_buf()));
        let account = AccountPlace::of_hex("ab");
        let command = StoredCommand::from_bytes(vec![4]);
        outbox
            .keep_async(account.clone(), chat(1), [2; 16], command)
            .await
            .unwrap();
        outbox
            .settled_async(account.clone(), chat(1), [2; 16])
            .await
            .unwrap();
        assert!(
            outbox
                .left_async(account.clone(), chat(1))
                .await
                .unwrap()
                .is_empty()
        );
        outbox
            .settled_async(account.clone(), chat(1), [2; 16])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_extra_command_is_refused_before_publication_and_settling_releases_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Arc::new(Outbox::new(dir.path().to_path_buf()));
        let account = AccountPlace::of_hex("ab");
        for id in 0..MAX_RESUMED_COMMANDS {
            outbox
                .keep_async(
                    account.clone(),
                    chat(1),
                    [id as u8; 16],
                    StoredCommand::from_bytes(vec![id as u8]),
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            outbox
                .keep_async(
                    account.clone(),
                    chat(1),
                    [32; 16],
                    StoredCommand::from_bytes(vec![32])
                )
                .await,
            Err(OutboxError::Capacity)
        ));
        outbox
            .keep_async(
                account.clone(),
                chat(1),
                [0; 16],
                StoredCommand::from_bytes(vec![7, 7]),
            )
            .await
            .unwrap();
        assert_eq!(
            outbox
                .left_async(account.clone(), chat(1))
                .await
                .unwrap()
                .len(),
            MAX_RESUMED_COMMANDS
        );
        outbox
            .settled_async(account.clone(), chat(1), [0; 16])
            .await
            .unwrap();
        outbox
            .keep_async(
                account.clone(),
                chat(1),
                [32; 16],
                StoredCommand::from_bytes(vec![32]),
            )
            .await
            .unwrap();
        assert_eq!(
            outbox.left_async(account, chat(1)).await.unwrap().len(),
            MAX_RESUMED_COMMANDS
        );
    }

    #[tokio::test]
    async fn large_command_files_share_the_existing_resume_byte_budget_and_survive_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Arc::new(Outbox::new(dir.path().to_path_buf()));
        let account = AccountPlace::of_hex("ab");
        let size = MAX_RESUMED_BYTES / 2 + 1;
        outbox
            .keep_async(
                account.clone(),
                chat(1),
                [1; 16],
                StoredCommand::from_bytes(vec![1; size]),
            )
            .await
            .unwrap();
        // Replacing the same command is charged once.
        outbox
            .keep_async(
                account.clone(),
                chat(1),
                [1; 16],
                StoredCommand::from_bytes(vec![1; size]),
            )
            .await
            .unwrap();
        assert!(matches!(
            outbox
                .keep_async(
                    account.clone(),
                    chat(1),
                    [2; 16],
                    StoredCommand::from_bytes(vec![2; size])
                )
                .await,
            Err(OutboxError::Capacity)
        ));
        outbox
            .settled_async(account.clone(), chat(1), [1; 16])
            .await
            .unwrap();
        outbox
            .keep_async(
                account.clone(),
                chat(1),
                [2; 16],
                StoredCommand::from_bytes(vec![2; size]),
            )
            .await
            .unwrap();
        let left = outbox.left_async(account.clone(), chat(1)).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, [2; 16]);
        assert_eq!(left[0].1.as_bytes().len(), size);
        assert!(matches!(
            outbox
                .keep_async(
                    account,
                    chat(1),
                    [3; 16],
                    StoredCommand::from_bytes(vec![0; MAX_RESUMED_BYTES + 1])
                )
                .await,
            Err(OutboxError::Capacity)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn separate_outbox_installations_cannot_race_past_the_shared_byte_limit() {
        let directory = tempfile::tempdir().unwrap();
        let first = Arc::new(Outbox::new(directory.path().to_path_buf()));
        let second = Arc::new(Outbox::new(directory.path().to_path_buf()));
        let account = AccountPlace::of_hex("ab");
        let size = MAX_RESUMED_BYTES / 2 + 1;
        let (a, b) = tokio::join!(
            first.keep_async(
                account.clone(),
                chat(1),
                [1; 16],
                StoredCommand::from_bytes(vec![1; size])
            ),
            second.keep_async(
                account.clone(),
                chat(1),
                [2; 16],
                StoredCommand::from_bytes(vec![2; size])
            )
        );
        assert_ne!(a.is_ok(), b.is_ok());
        assert!(matches!(a, Ok(()) | Err(OutboxError::Capacity)));
        assert!(matches!(b, Ok(()) | Err(OutboxError::Capacity)));
        let left = first.left_async(account.clone(), chat(1)).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].1.as_bytes().len(), size);
        assert_eq!(second.left_async(account, chat(1)).await.unwrap().len(), 1);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_legacy_case_variant_counts_separately_from_the_exact_replacement_path() {
        let root = tempfile::tempdir().unwrap();
        let outbox = Arc::new(Outbox::new(root.path().to_path_buf()));
        let account = AccountPlace::of_hex("ab");
        let size = MAX_RESUMED_BYTES / 2 + 1;
        outbox
            .keep_async(
                account.clone(),
                chat(1),
                [0xab; 16],
                StoredCommand::from_bytes(vec![1; size]),
            )
            .await
            .unwrap();
        // A legacy accepted name differs from the native writer's lowercase destination.
        let directory = root
            .path()
            .join("device-v1")
            .join("ab")
            .join("outbox")
            .join(chat(1).to_string());
        let original = directory.join(format!("{}.command", "ab".repeat(16)));
        let legacy = directory.join(format!("{}.command", "AB".repeat(16)));
        std::fs::copy(&original, &legacy).unwrap();
        assert!(matches!(
            outbox
                .keep_async(
                    account.clone(),
                    chat(1),
                    [0xab; 16],
                    StoredCommand::from_bytes(vec![2; size])
                )
                .await,
            Err(OutboxError::Capacity)
        ));
        assert_eq!(
            std::fs::metadata(&original).unwrap().len(),
            u64::try_from(size).unwrap()
        );
        assert_eq!(
            std::fs::metadata(&legacy).unwrap().len(),
            u64::try_from(size).unwrap()
        );
        std::fs::remove_file(legacy).unwrap();
        let left = outbox.left_async(account, chat(1)).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].1.as_bytes().first(), Some(&1));
    }

    #[tokio::test]
    async fn an_unwritable_outbox_reports_its_native_cause_instead_of_admitting_a_command() {
        use std::error::Error as _;
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let outbox = Arc::new(Outbox::new(blocked));
        let error = outbox
            .keep_async(
                AccountPlace::of_hex("ab"),
                chat(1),
                [3; 16],
                StoredCommand::from_bytes(vec![4]),
            )
            .await
            .unwrap_err();
        assert!(error.source().is_some());
        assert!(matches!(error, OutboxError::Io(_) | OutboxError::Write(_)));
    }

    #[test]
    fn only_command_files_name_a_command() {
        assert_eq!(
            command_id(&format!("{}.command", "0a".repeat(16))),
            Some([10; 16])
        );
        assert_eq!(
            command_id(&format!("{}.publication", "0a".repeat(16))),
            None
        );
        assert_eq!(command_id("0a.command"), None);
    }
}
