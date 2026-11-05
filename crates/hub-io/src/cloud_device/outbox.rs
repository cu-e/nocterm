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
use nocterm_cloud_device::command::StoredCommand;
use nocterm_cloud_device::command::stored::MAX_STORED_COMMAND_PLAINTEXT_BYTES;
use nocterm_cloud_device::context::ChatRef;
use nocterm_core::fs::{read_regular_bounded, write_private_atomic};
use std::io;
use std::path::PathBuf;

/// The sealed form adds a nonce and a tag to the plaintext.
const MAX_FILE_BYTES: usize = MAX_STORED_COMMAND_PLAINTEXT_BYTES + 64;
const LAYOUT: &str = "device-v1";
const EXTENSION: &str = "command";

/// The outbox files of an installation.
pub(crate) struct Outbox {
    root: PathBuf,
}
impl Outbox {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root: root.join(LAYOUT),
        }
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
    pub(crate) fn keep(
        &self,
        account: &AccountPlace,
        chat: &ChatRef,
        command_id: &[u8; 16],
        command: &StoredCommand,
    ) -> io::Result<()> {
        nocterm_core::paths::ensure_private_dir(&self.root)?;
        nocterm_core::paths::ensure_private_dir(&self.directory(account, chat))?;
        write_private_atomic(&self.file(account, chat, command_id), command.as_bytes())
            .map_err(nocterm_core::fs::FileError::into_io_error)
    }
    /// Forgets a command whose delivery ended.
    pub(crate) fn settled(&self, account: &AccountPlace, chat: &ChatRef, command_id: &[u8; 16]) {
        let file = self.file(account, chat, command_id);
        if let Err(error) = std::fs::remove_file(file)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "a settled Cloud command could not be forgotten");
        }
    }
    /// The commands of `chat` left from an earlier run, by command id. Unreadable files and
    /// other names are skipped.
    pub(crate) fn left(
        &self,
        account: &AccountPlace,
        chat: &ChatRef,
    ) -> Vec<([u8; 16], StoredCommand)> {
        let Ok(entries) = std::fs::read_dir(self.directory(account, chat)) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let path = entry.path();
                let command_id = command_id(path.file_name()?.to_str()?)?;
                let bytes = read_regular_bounded(&path, MAX_FILE_BYTES).ok()?;
                Some((command_id, StoredCommand::from_bytes(bytes)))
            })
            .collect()
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

    #[test]
    fn a_kept_command_reads_back_under_its_chat_with_exactly_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Outbox::new(dir.path().to_path_buf());
        let account = AccountPlace::of_hex("ab");
        let command = StoredCommand::from_bytes(vec![1, 2, 3]);
        outbox.keep(&account, &chat(1), &[7; 16], &command).unwrap();
        let left = outbox.left(&account, &chat(1));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, [7; 16]);
        assert_eq!(left[0].1.as_bytes(), [1, 2, 3]);
        assert!(outbox.left(&account, &chat(2)).is_empty());
    }

    #[test]
    fn a_settled_command_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Outbox::new(dir.path().to_path_buf());
        let account = AccountPlace::of_hex("ab");
        let command = StoredCommand::from_bytes(vec![4]);
        outbox.keep(&account, &chat(1), &[2; 16], &command).unwrap();
        outbox.settled(&account, &chat(1), &[2; 16]);
        assert!(outbox.left(&account, &chat(1)).is_empty());
        outbox.settled(&account, &chat(1), &[2; 16]);
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
