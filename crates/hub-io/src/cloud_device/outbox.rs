//! The exact publications of commands on their way, kept so that a command sealed before the
//! application quit is published again, with the same bytes, after it starts.
//!
//! A `Deliver` flow seals a command once and publishes exactly those bytes; the hub delivers a
//! publication id at most once. Each publication is written here (ciphertext sealed to the
//! runner, never plaintext) before it is first sent and removed once its command settled.
//! When a chat is read again, what is left of it is published again until the hub confirms it
//! or its lifetime ends.
//!
//! TODO(contract): the core offers no way to resume a `Deliver` flow from its sealed command,
//! so a resumed publication gets no receipt probe and is not shown as pending (see the P6
//! report); the runner's accepted event still shows the prompt.
use super::checkpoints::AccountPlace;
use nocterm_cloud_device::command::Publication;
use nocterm_core::fs::{read_regular_bounded, write_private_atomic};
use nocterm_hub_model::cloud::MAX_COMMAND_LIFETIME_MS;
use nocterm_hub_model::cloud::relay::RelayBinding;
use std::io;
use std::path::PathBuf;

const VERSION: u8 = 1;
const MAX_FILE_BYTES: usize = 64 * 1024;
const LAYOUT: &str = "device-v1";

/// One publication as it is published again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredPublication {
    pub(crate) publication_id: [u8; 16],
    pub(crate) binding: Vec<u8>,
    pub(crate) total_bytes: u32,
    pub(crate) ciphertext_sha256: [u8; 32],
    pub(crate) ciphertext: Vec<u8>,
    /// After this time the runner refuses the command; it is not published any more.
    pub(crate) horizon_ms: i64,
}
impl StoredPublication {
    pub(crate) fn of(publication: &Publication, now_ms: i64) -> Option<Self> {
        Some(Self {
            publication_id: publication.publication_id(),
            binding: publication.binding().ok()?,
            total_bytes: publication.total_bytes(),
            ciphertext_sha256: publication.ciphertext_sha256(),
            ciphertext: publication.ciphertext().to_vec(),
            horizon_ms: now_ms.saturating_add(MAX_COMMAND_LIFETIME_MS),
        })
    }
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(70 + self.binding.len() + self.ciphertext.len());
        bytes.push(VERSION);
        bytes.extend_from_slice(&self.horizon_ms.to_be_bytes());
        bytes.extend_from_slice(&self.publication_id);
        bytes.extend_from_slice(&self.total_bytes.to_be_bytes());
        bytes.extend_from_slice(&self.ciphertext_sha256);
        bytes.extend_from_slice(
            &u32::try_from(self.binding.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        bytes.extend_from_slice(&self.binding);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        let (&version, rest) = bytes.split_first()?;
        let (horizon, rest) = rest.split_first_chunk::<8>()?;
        let (publication_id, rest) = rest.split_first_chunk::<16>()?;
        let (total, rest) = rest.split_first_chunk::<4>()?;
        let (digest, rest) = rest.split_first_chunk::<32>()?;
        let (length, rest) = rest.split_first_chunk::<4>()?;
        let length = usize::try_from(u32::from_be_bytes(*length)).ok()?;
        if version != VERSION || rest.len() < length {
            return None;
        }
        let (binding, ciphertext) = rest.split_at(length);
        let total_bytes = u32::from_be_bytes(*total);
        (usize::try_from(total_bytes).ok()? == ciphertext.len()).then(|| Self {
            publication_id: *publication_id,
            binding: binding.to_vec(),
            total_bytes,
            ciphertext_sha256: *digest,
            ciphertext: ciphertext.to_vec(),
            horizon_ms: i64::from_be_bytes(*horizon),
        })
    }
    /// The chat the publication is a command of.
    pub(crate) fn chat_id(&self) -> Option<[u8; 16]> {
        RelayBinding::from_canonical_bytes(&self.binding)
            .ok()
            .map(|binding| binding.chat_id())
    }
}

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
    fn directory(&self, account: &AccountPlace) -> PathBuf {
        self.root.join(account.as_str()).join("outbox")
    }
    fn file(&self, account: &AccountPlace, publication_id: &[u8; 16]) -> PathBuf {
        let hex: String = publication_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.directory(account).join(format!("{hex}.publication"))
    }
    /// Keeps `publication` before it is first sent.
    pub(crate) fn stage(
        &self,
        account: &AccountPlace,
        publication: &StoredPublication,
    ) -> io::Result<()> {
        let directory = self.directory(account);
        nocterm_core::paths::ensure_private_dir(&self.root)?;
        nocterm_core::paths::ensure_private_dir(&directory)?;
        write_private_atomic(
            &self.file(account, &publication.publication_id),
            &publication.encode(),
        )
        .map_err(nocterm_core::fs::FileError::into_io_error)
    }
    /// Forgets a publication whose command settled.
    pub(crate) fn settled(&self, account: &AccountPlace, publication_id: &[u8; 16]) {
        let file = self.file(account, publication_id);
        if let Err(error) = std::fs::remove_file(file)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "a settled Cloud command could not be forgotten");
        }
    }
    /// The publications of `chat_id` left from an earlier run. Unreadable files are skipped.
    pub(crate) fn pending(
        &self,
        account: &AccountPlace,
        chat_id: &[u8; 16],
    ) -> Vec<StoredPublication> {
        let Ok(entries) = std::fs::read_dir(self.directory(account)) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| read_regular_bounded(&entry.path(), MAX_FILE_BYTES).ok())
            .filter_map(|bytes| StoredPublication::decode(&bytes))
            .filter(|publication| publication.chat_id().as_ref() == Some(chat_id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(id: u8) -> StoredPublication {
        StoredPublication {
            publication_id: [id; 16],
            binding: vec![1, 2, 3],
            total_bytes: 4,
            ciphertext_sha256: [9; 32],
            ciphertext: vec![5; 4],
            horizon_ms: 77,
        }
    }

    #[test]
    fn a_publication_reads_back_with_exactly_its_bytes() {
        let publication = stored(1);
        assert_eq!(
            StoredPublication::decode(&publication.encode()),
            Some(publication.clone())
        );
        let mut damaged = publication.encode();
        damaged.pop();
        assert_eq!(StoredPublication::decode(&damaged), None);
    }

    #[test]
    fn a_settled_publication_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Outbox::new(dir.path().to_path_buf());
        let account = AccountPlace::of_hex("ab");
        outbox.stage(&account, &stored(2)).unwrap();
        let file = outbox.file(&account, &[2; 16]);
        assert!(file.exists());
        outbox.settled(&account, &[2; 16]);
        assert!(!file.exists());
        outbox.settled(&account, &[2; 16]);
    }
}
