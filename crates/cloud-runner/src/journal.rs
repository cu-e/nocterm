mod append;
mod catalog;
mod cleanup;
pub use cleanup::DurableJournalCleanup;
mod current_heads;
mod effects;
mod fence;
mod head;
mod head_cells;
pub(crate) mod header_link;
mod metadata;
pub use current_heads::DurableCurrentCloudHeads;
mod replay;
mod retention;
mod snapshot_publication;
pub use snapshot_publication::DurableSnapshotPublication;
mod run_heads;
mod session_loss;

use crate::{
    codec,
    storage::{Database, storage},
};
use nocterm_cloud_model::{
    authority::{StreamBinding, TargetBinding},
    engine::EngineCheckpoint,
    identity::*,
    journal::*,
    tool::{
        BackgroundCommand, BackgroundCommandJournal, BackgroundCommandOutcome, ToolExecutionError,
    },
};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::{path::Path, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct JournalLimits {
    database_bytes: u64,
    chat_bytes: u64,
    chat_records: u64,
    chats: u64,
    catalog_bytes: usize,
}
impl JournalLimits {
    pub fn new(
        database_bytes: u64,
        chat_bytes: u64,
        chat_records: u64,
        chats: u64,
    ) -> Result<Self, JournalError> {
        if !(1024 * 1024..=16 * 1024 * 1024 * 1024).contains(&database_bytes)
            || chat_bytes == 0
            || chat_bytes > database_bytes
            || chat_records == 0
            || chat_records > 1_000_000
            || chats == 0
            || chats > MAX_JOURNAL_CATALOG_CHATS as u64
        {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            database_bytes,
            chat_bytes,
            chat_records,
            chats,
            catalog_bytes: MAX_JOURNAL_CATALOG_PLAINTEXT,
        })
    }
    /// Lower deployment ceiling for authenticated catalog plaintext.
    pub fn with_catalog_bytes(mut self, bytes: usize) -> Result<Self, JournalError> {
        if !(1024..=MAX_JOURNAL_CATALOG_PLAINTEXT).contains(&bytes) {
            return Err(JournalError::Capacity);
        }
        self.catalog_bytes = bytes;
        Ok(self)
    }
}
impl Default for JournalLimits {
    fn default() -> Self {
        Self {
            database_bytes: 1024 * 1024 * 1024,
            chat_bytes: 256 * 1024 * 1024,
            chat_records: 100_000,
            chats: 10_000,
            catalog_bytes: MAX_JOURNAL_CATALOG_PLAINTEXT,
        }
    }
}

const SCHEMA_VERSION: i64 = 5;

struct Driver {
    connection: Connection,
    database: Database,
    cipher: Arc<dyn JournalCipher>,
    limits: JournalLimits,
    events: Option<(
        Arc<dyn JournalEventEncoder>,
        crate::CoordinatorPublicationBudgets,
    )>,
    catalog: catalog::CatalogOwner,
    catalog_cipher: Arc<dyn JournalCatalogCipher>,
    highwater: Arc<Mutex<fence::HeadMarks>>,
    heads: head_cells::HeadCells,
}

/// Cloneable handle to one bounded serial SQLite owner. Methods do blocking disk
/// work when polled; composition must poll them on its blocking/background pool.
/// Concurrent use of the same handle refuses Capacity rather than queueing
/// unbounded work. Independent processes/connections use SQLite's bounded lock.
#[derive(Clone)]
pub struct DurableRunJournal {
    driver: Arc<Mutex<Driver>>,
    fence: Arc<fence::ReadFence>,
    current_heads: DurableCurrentCloudHeads,
}
impl DurableRunJournal {
    pub fn binding(&self) -> &JournalCatalogBinding {
        self.fence.binding()
    }
    /// A narrow terminal-fact/declaration facade; it cannot append new effects.
    pub fn cleanup(&self) -> DurableJournalCleanup {
        DurableJournalCleanup::new(self.clone())
    }
    /// Creates a private database with O_EXCL and durably initializes its empty
    /// authenticated catalog. Existing files are never replaced or repaired.
    pub fn create_new(
        root: &Path,
        binding: JournalCatalogBinding,
        cipher: Arc<dyn JournalCipher>,
        catalog_cipher: Arc<dyn JournalCatalogCipher>,
        head_cipher: Arc<dyn JournalChatHeadCipher>,
        limits: JournalLimits,
    ) -> Result<Self, JournalError> {
        let database = Database::create_new(root)?;
        let mut connection = database.connect()?;
        initialize(&connection, limits, true)?;
        let mut catalog = catalog::CatalogOwner::new(binding, limits.chats, limits.catalog_bytes);
        database.sync_directory()?;
        catalog.create(&mut connection, catalog_cipher.as_ref())?;
        let heads = head_cells::HeadCells::new(head_cipher, limits.chats);
        let current_heads = DurableCurrentCloudHeads::shared(
            root,
            catalog.clone(),
            catalog_cipher.clone(),
            heads.clone(),
            Some(Arc::downgrade(&cipher)),
        )?;
        let highwater = Arc::new(Mutex::new(fence::HeadMarks::new(limits.chats)));
        let fence = Arc::new(fence::ReadFence::new(
            root,
            catalog.clone(),
            cipher.clone(),
            catalog_cipher.clone(),
            highwater.clone(),
            heads.clone(),
        )?);
        Ok(Self {
            fence,
            current_heads,
            driver: Arc::new(Mutex::new(Driver {
                connection,
                database,
                cipher,
                limits,
                events: None,
                catalog,
                catalog_cipher,
                highwater,
                heads,
            })),
        })
    }
    /// Opens only an existing account/worker-bound database. Missing/old catalog
    /// data requires explicit recovery; it is never inferred from SQL indexes.
    pub fn open_existing(
        root: &Path,
        binding: JournalCatalogBinding,
        cipher: Arc<dyn JournalCipher>,
        catalog_cipher: Arc<dyn JournalCatalogCipher>,
        head_cipher: Arc<dyn JournalChatHeadCipher>,
        limits: JournalLimits,
    ) -> Result<Self, JournalError> {
        let database = Database::open_existing(root)?;
        let connection = database.connect()?;
        initialize(&connection, limits, false)?;
        let catalog = catalog::CatalogOwner::new(binding, limits.chats, limits.catalog_bytes);
        let heads = head_cells::HeadCells::new(head_cipher, limits.chats);
        let tx = connection.unchecked_transaction().map_err(storage)?;
        let current = catalog.load(&tx, catalog_cipher.as_ref())?;
        catalog.verify_rows(&tx, &current)?;
        // Every required head must authenticate before reopening succeeds.
        for stream in current.streams(catalog.binding()) {
            let native = head::required(&tx, cipher.as_ref(), &stream)?;
            let cell = heads.required(&tx, &stream)?;
            heads.check_content(&native, &cell)?;
        }
        tx.commit().map_err(storage)?;
        catalog.observe(&current)?;
        let current_heads = DurableCurrentCloudHeads::shared(
            root,
            catalog.clone(),
            catalog_cipher.clone(),
            heads.clone(),
            Some(Arc::downgrade(&cipher)),
        )?;
        let highwater = Arc::new(Mutex::new(fence::HeadMarks::new(limits.chats)));
        let fence = Arc::new(fence::ReadFence::new(
            root,
            catalog.clone(),
            cipher.clone(),
            catalog_cipher.clone(),
            highwater.clone(),
            heads.clone(),
        )?);
        Ok(Self {
            fence,
            current_heads,
            driver: Arc::new(Mutex::new(Driver {
                connection,
                database,
                cipher,
                limits,
                events: None,
                catalog,
                catalog_cipher,
                highwater,
                heads,
            })),
        })
    }
    pub fn current_heads(&self) -> DurableCurrentCloudHeads {
        self.current_heads.clone()
    }
    /// Background jobs require the injected encoder so each lifecycle observation
    /// commits its encrypted event in the same transaction as its original effect.
    pub fn with_background_events(
        self,
        events: Arc<dyn JournalEventEncoder>,
        work: crate::CoordinatorPublicationBudgets,
    ) -> Result<Self, JournalError> {
        self.with_driver(|driver| {
            driver.events = Some((events, work));
            Ok(())
        })?;
        Ok(self)
    }
    fn with_driver<T>(
        &self,
        operation: impl FnOnce(&mut Driver) -> Result<T, JournalError>,
    ) -> Result<T, JournalError> {
        let mut driver = self.driver.try_lock().ok_or(JournalError::Capacity)?;
        driver.database.validate_leaves()?;
        let catalog = driver
            .catalog
            .read_checked(&driver.connection, driver.catalog_cipher.as_ref())?;
        driver.catalog.observe(&catalog)?;
        operation(&mut driver)
    }
    fn with_stream<T>(
        &self,
        stream: &StreamBinding,
        operation: impl FnOnce(&mut Driver) -> Result<T, JournalError>,
    ) -> Result<T, JournalError> {
        self.with_driver(|driver| {
            fence::observe_driver(driver, stream)?;
            let result = operation(driver);
            fence::observe_driver(driver, stream)?;
            result
        })
    }
    /// Native derived checkpoint, never a second command ledger.
    pub fn checkpoint(&self, stream: StreamBinding) -> JournalFuture<Option<EngineCheckpoint>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(&stream, |driver| {
                driver.catalog.require_stream(&stream)?;
                replay::checkpoint(driver, &stream)
            })
        })
    }
    /// Read bounded ciphertext outbox entries. Delivery acknowledgement has no
    /// effect on command admission, journal ownership or target execution.
    pub fn outbox(
        &self,
        stream: StreamBinding,
        after: EventSequence,
        limits: JournalReadLimits,
    ) -> JournalFuture<Vec<OutboxEntry>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(&stream, |driver| {
                driver.catalog.require_stream(&stream)?;
                replay::outbox(driver, &stream, after, limits)
            })
        })
    }
    pub fn acknowledge_outbox(
        &self,
        lease: WriterLease,
        through: EventSequence,
    ) -> JournalFuture<()> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(lease.stream(), |driver| {
                driver.catalog.require_stream(lease.stream())?;
                retention::acknowledge(driver, &lease, through)
            })
        })
    }
    /// Atomically install a bounded authenticated snapshot and compact only the
    /// covered prefix. Immutable command receipts and tombstones are retained.
    pub fn compact(
        &self,
        lease: WriterLease,
        expected: JournalPosition,
        snapshot: JournalSnapshot,
    ) -> JournalFuture<()> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(lease.stream(), |driver| {
                driver.catalog.require_stream(lease.stream())?;
                retention::compact(driver, &lease, expected, &snapshot)
            })
        })
    }
}
impl RunJournalWriter for DurableRunJournal {
    fn claim_writer(&self, claim: WriterClaim) -> JournalFuture<WriterLease> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(claim.stream(), |driver| head::claim(driver, &claim))
        })
    }
    fn commit(&self, batch: JournalBatch) -> JournalFuture<CommitOutcome> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(batch.lease().stream(), |driver| {
                driver.catalog.require_stream(batch.lease().stream())?;
                append::commit(driver, &batch)
            })
        })
    }
}
impl JournalSnapshotWriter for DurableRunJournal {
    fn compact(
        &self,
        lease: WriterLease,
        expected: JournalPosition,
        snapshot: JournalSnapshot,
    ) -> JournalFuture<()> {
        DurableRunJournal::compact(self, lease, expected, snapshot)
    }
}
impl JournalReader for DurableRunJournal {
    fn read(
        &self,
        stream: StreamBinding,
        after: JournalSequence,
        limits: JournalReadLimits,
    ) -> JournalFuture<JournalPage> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(&stream, |driver| {
                driver.catalog.require_stream(&stream)?;
                replay::read(driver, &stream, after, limits)
            })
        })
    }
    fn snapshot(&self, stream: StreamBinding) -> JournalFuture<Option<JournalSnapshot>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(&stream, |driver| {
                driver.catalog.require_stream(&stream)?;
                replay::snapshot(driver, &stream)
            })
        })
    }
}

impl JournalCatalogReader for DurableRunJournal {
    fn catalog(
        &self,
        cursor: Option<JournalCatalogCursor>,
        limit: u16,
    ) -> JournalFuture<JournalCatalogPage> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                if limit == 0 || usize::from(limit) > MAX_JOURNAL_CATALOG_PAGE_CHATS {
                    return Err(JournalError::Capacity);
                }
                let catalog = driver
                    .catalog
                    .read_checked(&driver.connection, driver.catalog_cipher.as_ref())?;
                if cursor.as_ref().is_some_and(|cursor| {
                    cursor.binding() != driver.catalog.binding()
                        || cursor.revision() != catalog.revision()
                }) {
                    return Err(JournalError::Conflict);
                }
                let mut chats = catalog
                    .streams(driver.catalog.binding())
                    .map(|stream| stream.chat())
                    .filter(|id| cursor.as_ref().is_none_or(|cursor| *id > cursor.after()))
                    .take(usize::from(limit) + 1)
                    .collect::<Vec<_>>();
                let more = chats.len() > usize::from(limit);
                if more {
                    chats.pop();
                }
                driver.catalog.observe(&catalog)?;
                JournalCatalogPage::new(
                    driver.catalog.binding().clone(),
                    catalog.revision(),
                    chats,
                    more,
                )
            })
        })
    }
}

impl JournalControlReader for DurableRunJournal {
    fn head(&self, stream: StreamBinding) -> JournalFuture<Option<JournalHead>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(&stream, |driver| {
                driver.catalog.require_stream(&stream)?;
                driver.database.validate_leaves()?;
                head::load(&driver.connection, driver.cipher.as_ref(), &stream)?
                    .map(|head| {
                        JournalHead::new(
                            head.lease().clone(),
                            head.position(),
                            head.retained(),
                            head.deleted(),
                        )
                    })
                    .transpose()
            })
        })
    }
    fn command_receipt(
        &self,
        stream: StreamBinding,
        id: CommandId,
    ) -> JournalFuture<Option<CommandReceipt>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_stream(&stream, |driver| {
                driver.catalog.require_stream(&stream)?;
                driver.database.validate_leaves()?;
                let Some(head) = head::load(&driver.connection, driver.cipher.as_ref(), &stream)?
                else {
                    return Ok(None);
                };
                let receipt =
                    append::receipt(&driver.connection, driver.cipher.as_ref(), &stream, id)?;
                if receipt.as_ref().is_some_and(|receipt| {
                    let original = receipt.position();
                    let current = head.position();
                    receipt.lease().epoch() > head.lease().epoch()
                        || original.revision() > current.revision()
                        || original.records() > current.records()
                        || original.events() > current.events()
                        || original.control() > current.control()
                }) {
                    return Err(JournalError::Corrupt);
                }
                Ok(receipt)
            })
        })
    }
}
impl EngineCheckpointReader for DurableRunJournal {
    fn checkpoint(&self, stream: StreamBinding) -> JournalFuture<Option<EngineCheckpoint>> {
        DurableRunJournal::checkpoint(self, stream)
    }
}
impl JournalWriterFence for DurableRunJournal {
    fn require_current(&self, lease: &WriterLease) -> Result<(), JournalError> {
        self.fence.require_current(lease)
    }
}

impl TargetEffectReader for DurableRunJournal {
    fn unresolved_target(
        &self,
        scope: AccountScope,
        target: TargetBinding,
        limit: usize,
    ) -> JournalFuture<Vec<UnresolvedTargetEffect>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| effects::unresolved(driver, &scope, &target, limit))
        })
    }
}
impl BackgroundCommandJournal for DurableRunJournal {
    fn retain(
        &self,
        command: BackgroundCommand,
    ) -> futures::future::BoxFuture<'static, Result<(), ToolExecutionError>> {
        let journal = self.clone();
        Box::pin(async move {
            let mut driver = journal
                .driver
                .try_lock()
                .ok_or(ToolExecutionError::Capacity)?;
            let stream = command.dispatch().intent().authority().run().stream();
            fence::observe_driver(&driver, &stream).map_err(|cause| {
                ToolExecutionError::Execution(nocterm_core::SharedCause::new(cause))
            })?;
            let result = effects::retain(&mut driver, &command);
            fence::observe_driver(&driver, &stream).map_err(|cause| {
                ToolExecutionError::Execution(nocterm_core::SharedCause::new(cause))
            })?;
            result
        })
    }
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> futures::future::BoxFuture<'static, Result<(), ToolExecutionError>> {
        let journal = self.clone();
        Box::pin(async move {
            let mut driver = journal
                .driver
                .try_lock()
                .ok_or(ToolExecutionError::Capacity)?;
            let stream = command.dispatch().intent().authority().run().stream();
            fence::observe_driver(&driver, &stream).map_err(|cause| {
                ToolExecutionError::Execution(nocterm_core::SharedCause::new(cause))
            })?;
            let result = effects::finish(&mut driver, &command, outcome);
            fence::observe_driver(&driver, &stream).map_err(|cause| {
                ToolExecutionError::Execution(nocterm_core::SharedCause::new(cause))
            })?;
            result
        })
    }
}

fn initialize(db: &Connection, limits: JournalLimits, create: bool) -> Result<(), JournalError> {
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(storage)?;
    if (create && version != 0) || (!create && version != SCHEMA_VERSION) {
        return Err(JournalError::UnsupportedFormat);
    }
    let page_size: i64 = db
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(storage)?;
    if !(512..=65536).contains(&page_size) || !(page_size as u64).is_power_of_two() {
        return Err(JournalError::Corrupt);
    }
    let pages = integer(limits.database_bytes / unsigned(page_size)?)?;
    db.pragma_update(None, "max_page_count", pages)
        .map_err(storage)?;
    let actual_pages: i64 = db
        .query_row("PRAGMA max_page_count", [], |row| row.get(0))
        .map_err(storage)?;
    if actual_pages > pages {
        return Err(JournalError::Capacity);
    }
    if create {
        db.execute_batch(include_str!("journal/schema.sql"))
            .map_err(storage)?;
    }
    // Integrity checking may read index keys too. First exclude malformed root
    // metadata using payload-free type/length passes in this same snapshot.
    let tx = db.unchecked_transaction().map_err(storage)?;
    if !create {
        metadata::catalog(
            &tx,
            (limits.catalog_bytes + 64 * 1024).min(MAX_JOURNAL_CATALOG_CIPHERTEXT),
        )?;
    }
    metadata::rows(&tx, limits.chats)?;
    let integrity: String = tx
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(storage)?;
    if integrity != "ok" {
        return Err(JournalError::Corrupt);
    }
    tx.commit().map_err(storage)
}

fn integer(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::Capacity)
}
fn unsigned(value: i64) -> Result<u64, JournalError> {
    u64::try_from(value).map_err(|_| JournalError::Corrupt)
}
