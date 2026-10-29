//! Bounded native secret persistence; never a public token or wire codec.
use super::runner_peer::{RunnerHubError as Error, remote};
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_core::{
    SharedCause,
    fs::{AtomicPublication, create_private, open_private_regular, require_private_regular},
};
use nocterm_hub_client::cloud::RunnerSession;
use nocterm_hub_model::cloud::runner_auth::RunnerProofSubject;
use nocterm_hub_proto::v1::CloudRunnerTokens;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"NCRSE001";
const BYTES: usize = 572;
const TOKEN_BYTES: usize = 256;

pub(super) struct SessionRecord(Zeroizing<[u8; BYTES]>);
impl SessionRecord {
    pub(super) fn from_session(
        session: RunnerSession,
        file: &RunnerSessionFile,
    ) -> Result<Self, Error> {
        let mut tokens = session.into_response();
        let access = Zeroizing::new(std::mem::take(&mut tokens.access_token));
        let refresh = Zeroizing::new(std::mem::take(&mut tokens.refresh_token));
        if access.is_empty()
            || refresh.is_empty()
            || access.len() > TOKEN_BYTES
            || refresh.len() > TOKEN_BYTES
        {
            return Err(Error::Format);
        }
        let mut bytes = Zeroizing::new([0; BYTES]);
        bytes[..8].copy_from_slice(MAGIC);
        bytes[40..48].copy_from_slice(&tokens.access_expires_at_ms.to_be_bytes());
        bytes[48..56].copy_from_slice(&tokens.refresh_expires_at_ms.to_be_bytes());
        bytes[56..58].copy_from_slice(&(access.len() as u16).to_be_bytes());
        bytes[58..60].copy_from_slice(&(refresh.len() as u16).to_be_bytes());
        bytes[60..60 + access.len()].copy_from_slice(access.as_bytes());
        bytes[316..316 + refresh.len()].copy_from_slice(refresh.as_bytes());
        let digest = file.digest(&bytes[40..]);
        bytes[8..40].copy_from_slice(&digest);
        Ok(Self(bytes))
    }
    fn session(&self, subject: &RunnerProofSubject) -> Result<RunnerSession, Error> {
        let bytes = &self.0;
        let access_len =
            u16::from_be_bytes(bytes[56..58].try_into().map_err(|_| Error::Format)?) as usize;
        let refresh_len =
            u16::from_be_bytes(bytes[58..60].try_into().map_err(|_| Error::Format)?) as usize;
        if access_len == 0
            || refresh_len == 0
            || access_len > TOKEN_BYTES
            || refresh_len > TOKEN_BYTES
            || bytes[60 + access_len..316].iter().any(|v| *v != 0)
            || bytes[316 + refresh_len..].iter().any(|v| *v != 0)
        {
            return Err(Error::Format);
        }
        let access = std::str::from_utf8(&bytes[60..60 + access_len]).map_err(|_| Error::Format)?;
        let refresh =
            std::str::from_utf8(&bytes[316..316 + refresh_len]).map_err(|_| Error::Format)?;
        let tokens = CloudRunnerTokens {
            account_id: subject.namespace.account_id.as_str().to_owned(),
            runner_id: subject.runner_id.to_vec(),
            access_token: access.to_owned(),
            access_expires_at_ms: i64::from_be_bytes(
                bytes[40..48].try_into().map_err(|_| Error::Format)?,
            ),
            refresh_token: refresh.to_owned(),
            refresh_expires_at_ms: i64::from_be_bytes(
                bytes[48..56].try_into().map_err(|_| Error::Format)?,
            ),
            capability_version: 1,
        };
        RunnerSession::from_stored_response(tokens, subject).map_err(remote)
    }
}

pub(super) struct RunnerSessionFile {
    path: PathBuf,
    parent: File,
    _lock: File,
    published: bool,
    source: [u8; 32],
}
impl RunnerSessionFile {
    pub(super) fn acquire(
        root: &Path,
        binding: &JournalCatalogBinding,
        subject: &RunnerProofSubject,
    ) -> Result<Self, Error> {
        nocterm_core::paths::ensure_private_dir(root).map_err(storage)?;
        let parent = File::open(root).map_err(storage)?;
        let lock_path = root.join("runner-session.lock");
        let lock = match create_private(&lock_path) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_private_regular(&lock_path).map_err(storage)?
            }
            Err(error) => return Err(storage(error)),
        };
        require_private_regular(&lock).map_err(storage)?;
        lock.try_lock()
            .map_err(|error| storage(io::Error::other(error)))?;
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-runner-session-source-v1");
        let epoch = binding.epoch().get().to_be_bytes();
        for field in [
            binding.scope().hub().instance().as_str().as_bytes(),
            binding.scope().hub().authority().as_bytes(),
            binding.scope().account().id().as_str().as_bytes(),
            binding.scope().account().key().as_bytes(),
            subject.raw_hub_id.as_str().as_bytes(),
            subject.proof_key.as_slice(),
            binding.runner().as_bytes(),
            epoch.as_slice(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        Ok(Self {
            path: root.join("runner-session.v1"),
            parent,
            _lock: lock,
            published: false,
            source: hash.finalize().into(),
        })
    }
    pub(super) fn require_absent(&self) -> Result<(), Error> {
        match open_private_regular(&self.path) {
            Ok(_) => Err(Error::Existing),
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => Ok(()),
            Err(error) => Err(storage(error)),
        }
    }
    pub(super) fn read(&mut self, subject: &RunnerProofSubject) -> Result<RunnerSession, Error> {
        let mut file = open_private_regular(&self.path).map_err(storage)?;
        let mut bytes = Zeroizing::new([0u8; BYTES + 1]);
        let mut used = 0;
        loop {
            if used == bytes.len() {
                return Err(Error::Format);
            }
            let count = file.read(&mut bytes[used..]).map_err(storage)?;
            if count == 0 {
                break;
            }
            used += count;
        }
        if used != BYTES || &bytes[..8] != MAGIC || bytes[8..40] != self.digest(&bytes[40..BYTES]) {
            return Err(Error::Format);
        }
        let mut record = SessionRecord(Zeroizing::new([0; BYTES]));
        record.0.copy_from_slice(&bytes[..BYTES]);
        let session = record.session(subject)?;
        self.confirm(&file)?;
        self.published = true;
        Ok(session)
    }
    pub(super) fn save(&mut self, record: &SessionRecord) -> Result<(), Error> {
        let staged =
            AtomicPublication::stage_private(&self.path, record.0.as_ref()).map_err(storage)?;
        let file = if self.published {
            // Refuse an unsafe existing file before replacing the private session.
            let existing = open_private_regular(&self.path).map_err(storage)?;
            require_private_regular(&existing).map_err(storage)?;
            staged.publish().map_err(storage)?;
            self.published = true;
            open_private_regular(&self.path).map_err(stored)?
        } else {
            let file = staged.publish_new().map_err(storage)?;
            self.published = true;
            file
        };
        self.confirm(&file)
    }
    fn confirm(&self, file: &File) -> Result<(), Error> {
        require_private_regular(file).map_err(stored)?;
        file.sync_all().map_err(stored)?;
        self.parent.sync_all().map_err(stored)
    }
    fn digest(&self, bytes: &[u8]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-runner-session-record-v1");
        hash.update(self.source);
        hash.update(bytes);
        hash.update([0u8; 64]);
        hash.finalize().into()
    }
}
fn storage(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Storage(SharedCause::new(error))
}
fn stored(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::StoredButUnavailable(SharedCause::new(error))
}

#[cfg(test)]
mod tests;
