use fs2::FileExt as _;
use nocterm_vault::{DeviceCancellation, DeviceRegistration, PreparedWrite, VaultError};
use std::{
    fs::{self, File, OpenOptions},
    io::Read as _,
    path::{Path, PathBuf},
};
const MAX_ENVELOPE: usize = 108 + 48 + 1_048_576 + 16;
pub(crate) fn file_error(error: nocterm_core::fs::FileError) -> VaultError {
    VaultError::Io(error.into_io_error())
}
pub(crate) struct FileStore {
    path: PathBuf,
}
impl FileStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub(crate) fn exists(&self) -> bool {
        self.path.exists()
    }
    pub(crate) fn read_envelope(&self) -> Result<Vec<u8>, VaultError> {
        read_file(&self.path)
    }
    pub(crate) fn registration_path(&self) -> PathBuf {
        self.path.with_extension("device-unlock")
    }
    pub(crate) fn lock(&self) -> Result<File, VaultError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options.open(self.path.with_extension("lock"))?;
        lock.try_lock_exclusive().map_err(|_| VaultError::Busy)?;
        Ok(lock)
    }
    pub(crate) fn registration(&self) -> Result<DeviceRegistration, VaultError> {
        let file = File::open(self.registration_path())?;
        let mut bytes = Vec::new();
        file.take(32769).read_to_end(&mut bytes)?;
        DeviceRegistration::parse(&bytes)
    }
    pub(crate) fn commit(
        &self,
        candidate: &PreparedWrite,
        cancel: &DeviceCancellation,
    ) -> Result<(), VaultError> {
        let encrypted = candidate.bytes();
        let expected = candidate.expected_revision();
        let directory = self.path.parent().ok_or(VaultError::Format)?;
        fs::create_dir_all(directory)?;
        let _lock = self.lock()?;
        let current = match read_file(&self.path) {
            Ok(current) => Some(current),
            Err(VaultError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        match (expected, current.as_deref()) {
            (None, None) => {}
            (None, Some(_)) => return Err(VaultError::AlreadyExists),
            (Some(expected), Some(current)) if expected == current => {}
            _ => return Err(VaultError::Conflict),
        }
        let publication = nocterm_core::fs::AtomicPublication::stage_private(&self.path, encrypted)
            .map_err(file_error)?;
        if cancel.is_cancelled() {
            return Err(VaultError::Cancelled);
        }
        publication.publish().map_err(file_error)?;
        Ok(())
    }
}
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>, VaultError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(VaultError::Format);
    }
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_ENVELOPE as u64 {
        return Err(VaultError::Limit);
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_ENVELOPE as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ENVELOPE {
        return Err(VaultError::Limit);
    }
    Ok(bytes)
}
