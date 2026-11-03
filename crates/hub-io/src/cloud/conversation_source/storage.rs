//! Fixed-count private encrypted cells; durability is confirmed before delivery.
use nocterm_cloud_model::{
    identity::CommandId,
    projection::{ProjectionError, RecordKind},
};
use nocterm_core::{
    fs::{AtomicPublication, create_private, open_private_regular, require_private_regular},
    paths::ensure_private_dir,
};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::PathBuf,
};

pub(super) const MAX_PENDING: usize = 32;
const HEADER: usize = 16;
const MAGIC: &[u8; 8] = b"NCDOUT01";

/// Caller pre-admits the complete bounded input buffer before invoking a read.
/// The app supplies a trusted private parent; this is not a directory traversal API.
pub(super) struct OutboxFile {
    root: PathBuf,
    parent: File,
    completed: PathBuf,
    completed_parent: File,
    _lock: File,
    maximum: usize,
}
pub(super) struct SealedCell {
    pub writer: u64,
    pub ciphertext: Vec<u8>,
}
impl OutboxFile {
    pub(super) fn open(root: PathBuf, maximum: usize) -> Result<Self, ProjectionError> {
        if maximum == 0 || maximum > nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES + 16 * 1024 {
            return Err(ProjectionError::Capacity);
        }
        ensure_private_dir(&root).map_err(|_| ProjectionError::Persistence)?;
        let metadata = fs::symlink_metadata(&root).map_err(|_| ProjectionError::Persistence)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ProjectionError::Persistence);
        }
        let parent = File::open(&root).map_err(|_| ProjectionError::Persistence)?;
        let path = root.join("outbox.lock");
        let lock = match create_private(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_private_regular(&path).map_err(|_| ProjectionError::Persistence)?
            }
            Err(_) => return Err(ProjectionError::Persistence),
        };
        require_private_regular(&lock).map_err(|_| ProjectionError::Persistence)?;
        lock.try_lock().map_err(|_| ProjectionError::Unavailable)?;
        let completed = root.join("completed");
        ensure_private_dir(&completed).map_err(|_| ProjectionError::Persistence)?;
        if fs::symlink_metadata(&completed)
            .map_err(|_| ProjectionError::Persistence)?
            .file_type()
            .is_symlink()
        {
            return Err(ProjectionError::Persistence);
        }
        let completed_parent = File::open(&completed).map_err(|_| ProjectionError::Persistence)?;
        let result = Self {
            root,
            parent,
            completed,
            completed_parent,
            _lock: lock,
            maximum,
        };
        result.pending()?;
        Ok(result)
    }
    pub(super) fn pending(&self) -> Result<Vec<CommandId>, ProjectionError> {
        let mut ids = Vec::with_capacity(MAX_PENDING);
        for entry in fs::read_dir(&self.root).map_err(|_| ProjectionError::Persistence)? {
            let entry = entry.map_err(|_| ProjectionError::Persistence)?;
            let name = entry.file_name();
            if name == "outbox.lock" || name == "completed" {
                continue;
            }
            if ids.len() == MAX_PENDING
                || !entry
                    .file_type()
                    .map_err(|_| ProjectionError::Persistence)?
                    .is_file()
            {
                return Err(ProjectionError::Capacity);
            }
            let text = name.to_str().ok_or(ProjectionError::Persistence)?;
            let hex = text
                .strip_suffix(".outbox")
                .ok_or(ProjectionError::Persistence)?;
            if hex.len() != 32 {
                return Err(ProjectionError::Persistence);
            }
            let mut id = [0u8; 16];
            for (slot, pair) in id.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
                *slot = u8::from_str_radix(
                    std::str::from_utf8(pair).map_err(|_| ProjectionError::Persistence)?,
                    16,
                )
                .map_err(|_| ProjectionError::Persistence)?;
            }
            ids.push(CommandId::from_bytes(id));
        }
        Ok(ids)
    }
    pub(super) fn read(
        &self,
        id: CommandId,
        maximum: usize,
    ) -> Result<Option<SealedCell>, ProjectionError> {
        let maximum = maximum.min(self.maximum);
        let mut file = match open_private_regular(&self.path(id)) {
            Ok(file) => file,
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => {
                match open_private_regular(
                    &self.completed.join(
                        self.path(id)
                            .file_name()
                            .ok_or(ProjectionError::Persistence)?,
                    ),
                ) {
                    Ok(file) => file,
                    Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => {
                        return Ok(None);
                    }
                    Err(_) => return Err(ProjectionError::Persistence),
                }
            }
            Err(_) => return Err(ProjectionError::Persistence),
        };
        require_private_regular(&file).map_err(|_| ProjectionError::Persistence)?;
        let length = usize::try_from(
            file.metadata()
                .map_err(|_| ProjectionError::Persistence)?
                .len(),
        )
        .map_err(|_| ProjectionError::Capacity)?;
        if length < HEADER + 42 || length > maximum + HEADER {
            return Err(ProjectionError::Capacity);
        }
        let mut header = [0; HEADER];
        file.read_exact(&mut header)
            .map_err(|_| ProjectionError::Persistence)?;
        if &header[..8] != MAGIC {
            return Err(ProjectionError::InvalidRecord(RecordKind::Storage));
        }
        let writer = u64::from_be_bytes(
            header[8..]
                .try_into()
                .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Storage))?,
        );
        if writer == 0 {
            return Err(ProjectionError::InvalidRecord(RecordKind::Storage));
        }
        let mut ciphertext = vec![0; length - HEADER];
        file.read_exact(&mut ciphertext)
            .map_err(|_| ProjectionError::Persistence)?;
        let mut extra = [0; 1];
        if file
            .read(&mut extra)
            .map_err(|_| ProjectionError::Persistence)?
            != 0
        {
            return Err(ProjectionError::Persistence);
        }
        self.confirm(&file)?;
        Ok(Some(SealedCell { writer, ciphertext }))
    }
    pub(super) fn stage(
        &self,
        id: CommandId,
        bytes: &[u8],
        writer: u64,
        urgent: bool,
    ) -> Result<AtomicPublication, ProjectionError> {
        if writer == 0 || bytes.len() > self.maximum {
            return Err(ProjectionError::Capacity);
        }
        if self.pending()?.len() >= if urgent { MAX_PENDING } else { MAX_PENDING - 1 } {
            return Err(ProjectionError::Capacity);
        }
        let mut file = Vec::with_capacity(HEADER + bytes.len());
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&writer.to_be_bytes());
        file.extend_from_slice(bytes);
        AtomicPublication::stage_private(&self.path(id), &file)
            .map_err(|_| ProjectionError::Persistence)
    }
    /// The exclusive owner moves opaque bytes only after a pinned R admission
    /// fact. No message is re-signed, discarded or overwritten. Archive fullness
    /// leaves the pending original intact and is reported as maintenance.
    pub(super) fn archive(&self, id: CommandId) -> Result<(), ProjectionError> {
        let source = self.path(id);
        let opened = match open_private_regular(&source) {
            Ok(file) => file,
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => return Ok(()),
            Err(_) => return Err(ProjectionError::Persistence),
        };
        let destination = self
            .completed
            .join(source.file_name().ok_or(ProjectionError::Persistence)?);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(ProjectionError::Persistence);
        }
        let mut count = 0usize;
        for entry in fs::read_dir(&self.completed).map_err(|_| ProjectionError::Persistence)? {
            entry.map_err(|_| ProjectionError::Persistence)?;
            count += 1;
            if count >= 256 {
                return Err(ProjectionError::Capacity);
            }
        }
        // Trusted private ancestors and the held exclusive owner serialize all
        // writers to this directory, including another process's capture.
        fs::rename(&source, &destination).map_err(|_| ProjectionError::Persistence)?;
        require_private_regular(&opened).map_err(|_| ProjectionError::Persistence)?;
        opened
            .sync_all()
            .and_then(|()| self.completed_parent.sync_all())
            .and_then(|()| self.parent.sync_all())
            .and_then(|()| self.completed_parent.sync_all())
            .map_err(|_| ProjectionError::Persistence)
    }
    pub(super) fn confirm(&self, file: &File) -> Result<(), ProjectionError> {
        require_private_regular(file).map_err(|_| ProjectionError::Persistence)?;
        file.sync_all()
            .and_then(|()| self.parent.sync_all())
            .and_then(|()| self.completed_parent.sync_all())
            .map_err(|_| ProjectionError::Persistence)
    }
    fn path(&self, id: CommandId) -> PathBuf {
        let hex: String = id
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.root.join(format!("{hex}.outbox"))
    }
}
