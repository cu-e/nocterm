//! Fixed bounded private material snapshot; stores sealed grants, never chat keys.
use super::{MAX_MATERIALS, MaterialStoreError as Error, StoredGrant};
use nocterm_cloud_model::{
    identity::{ChatKeyEpoch, CloudChatId},
    journal::JournalCatalogBinding,
};
use nocterm_core::{
    SharedCause,
    fs::{AtomicPublication, create_private, open_private_regular, require_private_regular},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"NCRMG001";
const HEADER: usize = 44;
const ENTRY: usize = 363;
const MAX_BYTES: usize = HEADER + MAX_MATERIALS * ENTRY;

pub(super) struct MaterialFile {
    path: PathBuf,
    parent: File,
    _lock: File,
    source: [u8; 32],
    published: bool,
}
impl MaterialFile {
    pub(super) fn acquire(
        root: &Path,
        binding: &JournalCatalogBinding,
        encryption: &[u8; 32],
    ) -> Result<Self, Error> {
        nocterm_core::paths::ensure_private_dir(root).map_err(storage)?;
        let parent = File::open(root).map_err(storage)?;
        let lock_path = root.join("runner-material.lock");
        let lock = match create_private(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_private_regular(&lock_path).map_err(storage)?
            }
            Err(error) => return Err(storage(error)),
        };
        require_private_regular(&lock).map_err(storage)?;
        lock.try_lock()
            .map_err(|error| storage(io::Error::other(error)))?;
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-material-source-v1");
        let runner = binding.runner();
        let epoch = binding.epoch().get().to_be_bytes();
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
        Ok(Self {
            path: root.join("runner-material.v1"),
            parent,
            _lock: lock,
            source: hash.finalize().into(),
            published: false,
        })
    }
    pub(super) fn read(
        &mut self,
        maximum: usize,
    ) -> Result<BTreeMap<CloudChatId, StoredGrant>, Error> {
        let mut file = open_private_regular(&self.path).map_err(storage)?;
        let size = usize::try_from(file.metadata().map_err(storage)?.len())
            .map_err(|_| Error::Capacity)?;
        if !(HEADER..=MAX_BYTES).contains(&size) || !(size - HEADER).is_multiple_of(ENTRY) {
            return Err(Error::Format);
        }
        let mut bytes = Vec::with_capacity(size + 1);
        file.by_ref()
            .take((size + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        if bytes.len() != size || &bytes[..8] != MAGIC || bytes[8..40] != self.digest(&bytes[40..])
        {
            return Err(Error::Format);
        }
        let count =
            u32::from_be_bytes(bytes[40..44].try_into().map_err(|_| Error::Format)?) as usize;
        if count > maximum || count != (size - HEADER) / ENTRY {
            return Err(Error::Capacity);
        }
        let mut entries = BTreeMap::new();
        for row in bytes[HEADER..].as_chunks::<ENTRY>().0 {
            let (chat, grant) = decode(row)?;
            if entries.insert(chat, grant).is_some() {
                return Err(Error::Format);
            }
        }
        self.confirm(&file)?;
        self.published = true;
        Ok(entries)
    }
    pub(super) fn save(
        &mut self,
        entries: &BTreeMap<CloudChatId, StoredGrant>,
    ) -> Result<(), Error> {
        if entries.len() > MAX_MATERIALS {
            return Err(Error::Capacity);
        }
        let mut bytes = vec![0; HEADER + entries.len() * ENTRY];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[40..44].copy_from_slice(&(entries.len() as u32).to_be_bytes());
        for ((chat, grant), row) in entries
            .iter()
            .zip(bytes[HEADER..].as_chunks_mut::<ENTRY>().0)
        {
            encode(row, *chat, grant)?;
        }
        let digest = self.digest(&bytes[40..]);
        bytes[8..40].copy_from_slice(&digest);
        let stage = AtomicPublication::stage_private(&self.path, &bytes).map_err(storage)?;
        let file = if self.published {
            let previous = open_private_regular(&self.path).map_err(storage)?;
            require_private_regular(&previous).map_err(storage)?;
            stage.publish().map_err(storage)?;
            open_private_regular(&self.path).map_err(stored)?
        } else {
            let file = stage.publish_new().map_err(storage)?;
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
        hash.update(b"nocterm-native-material-record-v1");
        hash.update(self.source);
        hash.update(bytes);
        hash.finalize().into()
    }
}
fn encode(out: &mut [u8], chat: CloudChatId, grant: &StoredGrant) -> Result<(), Error> {
    if grant.sealed.is_empty() || grant.sealed.len() > 256 {
        return Err(Error::Capacity);
    }
    out[0] = u8::from(grant.retired);
    out[1..17].copy_from_slice(chat.as_bytes());
    out[17..25].copy_from_slice(&grant.epoch.get().to_be_bytes());
    out[25..57].copy_from_slice(&grant.account);
    out[57..73].copy_from_slice(&grant.original_delivery);
    out[73..105].copy_from_slice(&grant.digest);
    out[105..107].copy_from_slice(&(grant.sealed.len() as u16).to_be_bytes());
    out[107..107 + grant.sealed.len()].copy_from_slice(&grant.sealed);
    Ok(())
}
fn decode(row: &[u8]) -> Result<(CloudChatId, StoredGrant), Error> {
    let length = u16::from_be_bytes(row[105..107].try_into().map_err(|_| Error::Format)?) as usize;
    if row[0] > 1
        || length == 0
        || length > 256
        || row[107 + length..].iter().any(|byte| *byte != 0)
    {
        return Err(Error::Format);
    }
    let sealed = row[107..107 + length].to_vec();
    let digest: [u8; 32] = row[73..105].try_into().map_err(|_| Error::Format)?;
    if <[u8; 32]>::from(Sha256::digest(&sealed)) != digest {
        return Err(Error::Format);
    }
    Ok((
        CloudChatId::from_bytes(row[1..17].try_into().map_err(|_| Error::Format)?),
        StoredGrant {
            epoch: ChatKeyEpoch::new(u64::from_be_bytes(
                row[17..25].try_into().map_err(|_| Error::Format)?,
            ))
            .map_err(|_| Error::Format)?,
            account: row[25..57].try_into().map_err(|_| Error::Format)?,
            original_delivery: row[57..73].try_into().map_err(|_| Error::Format)?,
            digest,
            sealed,
            retired: row[0] == 1,
        },
    ))
}
fn storage(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Storage(SharedCause::new(error))
}
fn stored(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::StoredButUnavailable(SharedCause::new(error))
}
