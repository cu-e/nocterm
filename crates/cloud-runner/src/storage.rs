mod budget;
pub(crate) use budget::StorageBudget;

use nocterm_cloud_model::journal::{JournalError, MAX_STORED_JOURNAL_BYTES};
use nocterm_core::{SharedCause, fs::create_private};
use rusqlite::{Connection, OpenFlags, limits::Limit};
use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
    time::Duration,
};

const DATABASE: &str = "cloud-runs.sqlite3";

/// SQLite's pathname VFS requires supervisor-owned trusted ancestry. This owner
/// refuses static links and unsafe leaves; it does not claim held-directory race
/// protection against a process able to replace that trusted ancestry.
pub(crate) struct Database {
    path: PathBuf,
    root: File,
}
impl Database {
    pub(crate) fn create_new(root: &Path) -> Result<Self, JournalError> {
        Self::prepare(root, true)
    }
    pub(crate) fn open_existing(root: &Path) -> Result<Self, JournalError> {
        Self::prepare(root, false)
    }
    fn prepare(root: &Path, create: bool) -> Result<Self, JournalError> {
        validate_ancestry(root)?;
        let metadata = fs::symlink_metadata(root).map_err(storage)?;
        validate_directory(&metadata)?;
        let root_file = File::open(root).map_err(storage)?;
        let path = root.join(DATABASE);
        if create {
            match fs::symlink_metadata(&path) {
                Ok(metadata) => validate_file(&metadata)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(storage(error)),
            }
            let file = create_private(&path).map_err(storage)?;
            file.sync_all().map_err(storage)?;
            root_file.sync_all().map_err(storage)?;
        }
        let database = Self {
            path,
            root: root_file,
        };
        database.validate_leaves()?;
        Ok(database)
    }
    pub(crate) fn connect_readonly(&self) -> Result<Connection, JournalError> {
        self.validate_leaves()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let db = Connection::open_with_flags(&self.path, flags).map_err(storage)?;
        db.busy_timeout(Duration::from_millis(5)).map_err(storage)?;
        db.set_limit(
            Limit::SQLITE_LIMIT_LENGTH,
            (MAX_STORED_JOURNAL_BYTES + 4096) as i32,
        )
        .map_err(storage)?;
        db.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 64 * 1024)
            .map_err(storage)?;
        db.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
            .map_err(storage)?;
        db.execute_batch(
            "PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY;",
        )
        .map_err(storage)?;
        self.validate_leaves()?;
        Ok(db)
    }
    pub(crate) fn connect(&self) -> Result<Connection, JournalError> {
        self.validate_leaves()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let db = Connection::open_with_flags(&self.path, flags).map_err(storage)?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(storage)?;
        db.set_limit(
            Limit::SQLITE_LIMIT_LENGTH,
            (MAX_STORED_JOURNAL_BYTES + 4096) as i32,
        )
        .map_err(storage)?;
        db.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 64 * 1024)
            .map_err(storage)?;
        db.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
            .map_err(storage)?;
        db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA secure_delete=ON;").map_err(storage)?;
        let mode: String = db
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(storage)?;
        let sync: i64 = db
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .map_err(storage)?;
        if mode != "delete" || sync != 3 {
            return Err(JournalError::Corrupt);
        }
        self.validate_leaves()?;
        Ok(db)
    }
    pub(crate) fn sync_directory(&self) -> Result<(), JournalError> {
        self.root.sync_all().map_err(storage)
    }
    pub(crate) fn validate_leaves(&self) -> Result<(), JournalError> {
        let root_path = self.path.parent().ok_or(JournalError::Corrupt)?;
        let root_metadata = fs::symlink_metadata(root_path).map_err(storage)?;
        validate_directory(&root_metadata)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let held = self.root.metadata().map_err(storage)?;
            if held.dev() != root_metadata.dev() || held.ino() != root_metadata.ino() {
                return Err(JournalError::Corrupt);
            }
        }
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let mut path = self.path.as_os_str().to_owned();
            path.push(suffix);
            match fs::symlink_metadata(Path::new(&path)) {
                Ok(metadata) => {
                    validate_file(&metadata)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt as _;
                        if metadata.uid() != root_metadata.uid() {
                            return Err(JournalError::Corrupt);
                        }
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && !suffix.is_empty() => {}
                Err(error) => return Err(storage(error)),
            }
        }
        Ok(())
    }
}

fn validate_ancestry(path: &Path) -> Result<(), JournalError> {
    if !path.is_absolute() {
        return Err(JournalError::Corrupt);
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(JournalError::Corrupt);
        }
        prefix.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&prefix).map_err(storage)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(JournalError::Corrupt);
        }
    }
    Ok(())
}
fn validate_directory(metadata: &fs::Metadata) -> Result<(), JournalError> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(JournalError::Corrupt);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o777 != 0o700 {
            return Err(JournalError::Corrupt);
        }
    }
    Ok(())
}
fn validate_file(metadata: &fs::Metadata) -> Result<(), JournalError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(JournalError::Corrupt);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o777 != 0o600 || metadata.nlink() != 1 {
            return Err(JournalError::Corrupt);
        }
    }
    Ok(())
}
pub(crate) fn storage(error: impl std::error::Error + Send + Sync + 'static) -> JournalError {
    JournalError::Storage(SharedCause::new(error))
}
