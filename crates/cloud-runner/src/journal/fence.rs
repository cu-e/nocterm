//! A separate existing-file read connection and shared monotonic refusal marks.
use super::{Driver, catalog::CatalogOwner, head, head_cells::HeadCells, storage};
use crate::storage::Database;
use nocterm_cloud_model::{authority::StreamBinding, journal::*};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Clone, Copy)]
struct HeadMark {
    epoch: u64,
    position: JournalPosition,
    retained: u64,
    deleted: bool,
}
impl HeadMark {
    fn from(head: &head::Head) -> Self {
        Self {
            epoch: head.lease().epoch().get(),
            position: head.position(),
            retained: head.retained().get(),
            deleted: head.deleted(),
        }
    }
    fn permits(&self, next: &Self) -> bool {
        next.epoch >= self.epoch
            && next.position.revision() >= self.position.revision()
            && next.position.control() >= self.position.control()
            && next.position.records() >= self.position.records()
            && next.position.events() >= self.position.events()
            && next.retained >= self.retained
            && (!self.deleted || next.deleted)
    }
}
pub(super) struct HeadMarks {
    heads: BTreeMap<StreamBinding, HeadMark>,
    capacity: usize,
}
impl HeadMarks {
    pub(super) fn new(capacity: u64) -> Self {
        Self {
            heads: BTreeMap::new(),
            capacity: capacity as usize,
        }
    }
    fn observe(
        &mut self,
        stream: &StreamBinding,
        head: Option<&head::Head>,
    ) -> Result<(), JournalError> {
        let Some(head) = head else {
            return if self.heads.contains_key(stream) {
                Err(JournalError::Corrupt)
            } else {
                Ok(())
            };
        };
        let next = HeadMark::from(head);
        if let Some(previous) = self.heads.get(stream) {
            if !previous.permits(&next) {
                return Err(JournalError::Corrupt);
            }
        } else if self.heads.len() >= self.capacity {
            return Err(JournalError::Capacity);
        }
        self.heads.insert(stream.clone(), next);
        Ok(())
    }
}
/// All SQLite, filesystem and cipher operations finish before locking marks.
pub(super) fn observe_driver(driver: &Driver, stream: &StreamBinding) -> Result<(), JournalError> {
    driver.catalog.require_stream(stream)?;
    let tx = driver.connection.unchecked_transaction().map_err(storage)?;
    let catalog = driver.catalog.load(&tx, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&tx, &catalog)?;
    let head = head::load(&tx, driver.cipher.as_ref(), stream)?;
    let cell = if let Some(head) = &head {
        let cell = driver.heads.required(&tx, stream)?;
        driver.heads.check_content(head, &cell)?;
        Some(cell)
    } else {
        None
    };
    tx.commit().map_err(storage)?;
    driver.catalog.observe(&catalog)?;
    if let Some(cell) = cell {
        driver.heads.observe(stream, &cell)?;
    }
    driver.highwater.lock().observe(stream, head.as_ref())
}

pub(super) struct ReadFence {
    database: Database,
    connection: Mutex<Connection>,
    catalog: CatalogOwner,
    cipher: Arc<dyn JournalCipher>,
    catalog_cipher: Arc<dyn JournalCatalogCipher>,
    highwater: Arc<Mutex<HeadMarks>>,
    heads: HeadCells,
}
impl ReadFence {
    pub(super) fn binding(&self) -> &JournalCatalogBinding {
        self.catalog.binding()
    }
    pub(super) fn new(
        root: &Path,
        catalog: CatalogOwner,
        cipher: Arc<dyn JournalCipher>,
        catalog_cipher: Arc<dyn JournalCatalogCipher>,
        highwater: Arc<Mutex<HeadMarks>>,
        heads: HeadCells,
    ) -> Result<Self, JournalError> {
        let database = Database::open_existing(root)?;
        let connection = Mutex::new(database.connect_readonly()?);
        Ok(Self {
            database,
            connection,
            catalog,
            cipher,
            catalog_cipher,
            highwater,
            heads,
        })
    }
    pub(super) fn require_current(&self, lease: &WriterLease) -> Result<(), JournalError> {
        self.catalog.require_stream(lease.stream())?;
        let db = self.connection.try_lock().ok_or(JournalError::Busy)?;
        self.database.validate_leaves()?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(read_error)?;
        if version != super::SCHEMA_VERSION {
            return Err(JournalError::UnsupportedFormat);
        }
        let tx = db.unchecked_transaction().map_err(read_error)?;
        let catalog = self
            .catalog
            .load(&tx, self.catalog_cipher.as_ref())
            .map_err(classify_busy)?;
        self.catalog
            .verify_rows(&tx, &catalog)
            .map_err(classify_busy)?;
        let current =
            head::required(&tx, self.cipher.as_ref(), lease.stream()).map_err(classify_busy)?;
        let cell = self
            .heads
            .required(&tx, lease.stream())
            .map_err(classify_busy)?;
        self.heads.check_content(&current, &cell)?;
        tx.commit().map_err(read_error)?;
        self.database.validate_leaves()?;
        self.catalog.observe(&catalog)?;
        self.heads.observe(lease.stream(), &cell)?;
        self.highwater
            .lock()
            .observe(lease.stream(), Some(&current))?;
        current.require_writer(lease)
    }
}
pub(super) fn read_error(error: rusqlite::Error) -> JournalError {
    if matches!(error, rusqlite::Error::SqliteFailure(cause, _) if matches!(cause.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
    {
        JournalError::Busy
    } else {
        storage(error)
    }
}
pub(super) fn classify_busy(error: JournalError) -> JournalError {
    if let JournalError::Storage(cause) = &error
        && let Some(cause) = cause.downcast_ref::<rusqlite::Error>()
        && matches!(cause, rusqlite::Error::SqliteFailure(cause, _) if matches!(cause.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
    {
        return JournalError::Busy;
    }
    error
}
