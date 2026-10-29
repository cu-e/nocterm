//! Existing-file root-only consistent observations, with no writer capability.
use super::{
    append,
    catalog::{Catalog, CatalogOwner},
    fence::{classify_busy, read_error},
    head_cells::{Cell, HeadCells},
};
use crate::storage::Database;
use nocterm_cloud_model::{authority::StreamBinding, journal::*};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Weak},
};

#[derive(Clone)]
pub struct DurableCurrentCloudHeads(Arc<Reader>);
struct Reader {
    database: Database,
    connection: Mutex<Connection>,
    catalog: CatalogOwner,
    catalog_cipher: Arc<dyn JournalCatalogCipher>,
    heads: HeadCells,
    receipts: Option<Weak<dyn JournalCipher>>,
}
impl DurableCurrentCloudHeads {
    /// Requires no content key and does not open the encrypted native Header.
    pub fn open_existing(
        root: &Path,
        binding: JournalCatalogBinding,
        catalog_cipher: Arc<dyn JournalCatalogCipher>,
        head_cipher: Arc<dyn JournalChatHeadCipher>,
        limits: super::JournalLimits,
    ) -> Result<Self, JournalError> {
        Self::shared(
            root,
            CatalogOwner::new(binding, limits.chats, limits.catalog_bytes),
            catalog_cipher,
            HeadCells::new(head_cipher, limits.chats),
            None,
        )
    }
    pub(super) fn shared(
        root: &Path,
        catalog: CatalogOwner,
        catalog_cipher: Arc<dyn JournalCatalogCipher>,
        heads: HeadCells,
        receipts: Option<Weak<dyn JournalCipher>>,
    ) -> Result<Self, JournalError> {
        let database = Database::open_existing(root)?;
        let connection = Mutex::new(database.connect_readonly()?);
        let value = Self(Arc::new(Reader {
            database,
            connection,
            catalog,
            catalog_cipher,
            heads,
            receipts,
        }));
        value.0.read(|_, _| Ok(((), Vec::new())))?;
        Ok(value)
    }
}
impl Reader {
    fn read<T>(
        &self,
        operation: impl FnOnce(&Connection, &Catalog) -> Result<(T, Vec<Cell>), JournalError>,
    ) -> Result<T, JournalError> {
        let connection = self.connection.try_lock().ok_or(JournalError::Busy)?;
        self.database.validate_leaves()?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(read_error)?;
        if version != super::SCHEMA_VERSION {
            return Err(JournalError::UnsupportedFormat);
        }
        let tx = connection.unchecked_transaction().map_err(read_error)?;
        let catalog = self
            .catalog
            .load(&tx, self.catalog_cipher.as_ref())
            .map_err(classify_busy)?;
        self.catalog
            .verify_rows(&tx, &catalog)
            .map_err(classify_busy)?;
        let (value, cells) = operation(&tx, &catalog).map_err(classify_busy)?;
        tx.commit().map_err(read_error)?;
        self.database.validate_leaves()?;
        // No filesystem/SQLite/cipher operation occurs under refusal-mark locks.
        self.catalog.observe(&catalog)?;
        for cell in cells {
            self.heads.observe(cell.value().stream(), &cell)?;
        }
        Ok(value)
    }
    fn catalog(
        &self,
        after: Option<CurrentHeadsCursor>,
        limit: u16,
    ) -> Result<CurrentHeadsPage, JournalError> {
        if limit == 0 || limit as usize > MAX_CURRENT_HEADS_PAGE {
            return Err(JournalError::Capacity);
        }
        self.read(|db, catalog| {
            if after.as_ref().is_some_and(|cursor| {
                cursor.binding() != self.catalog.binding()
                    || cursor.revision() != catalog.revision()
                    || cursor.root() != catalog.hash()
                    || catalog
                        .chats()
                        .nth(cursor.seen().saturating_sub(1) as usize)
                        != Some(cursor.after())
            }) {
                return Err(JournalError::Conflict);
            }
            let skip = after.as_ref().map_or(0, |cursor| cursor.seen() as usize);
            let cells = catalog
                .chats()
                .skip(skip)
                .take(limit as usize)
                .map(|id| self.heads.required(db, &self.catalog.binding().stream(id)))
                .collect::<Result<Vec<_>, _>>()?;
            let more = skip + cells.len() < catalog.len();
            let value = CurrentHeadsPage::new(
                self.catalog.binding().clone(),
                catalog.revision(),
                catalog.hash(),
                catalog.len() as u32,
                after,
                cells.iter().map(|cell| cell.value().clone()).collect(),
                more,
            )?;
            Ok((value, cells))
        })
    }
    fn chat(&self, stream: StreamBinding) -> Result<Option<CurrentChatHead>, JournalError> {
        self.catalog.require_stream(&stream)?;
        self.read(|db, catalog| {
            if !catalog.chats().any(|id| id == stream.chat()) {
                return Ok((None, Vec::new()));
            }
            let cell = self.heads.required(db, &stream)?;
            Ok((Some(cell.value().clone()), vec![cell]))
        })
    }
    fn receipt(
        &self,
        query: OriginalCommandReceiptQuery,
    ) -> Result<OriginalCommandReceiptObservation, JournalError> {
        self.catalog.require_stream(&query.stream)?;
        self.read(|db, catalog| {
            use OriginalCommandReceiptObservation as Result;
            if !catalog.chats().any(|id| id == query.stream.chat()) {
                return Ok((Result::NotFound, Vec::new()));
            }
            let cell = self.heads.required(db, &query.stream)?;
            if cell.value().key_epoch() != query.key_epoch {
                return Ok((Result::Conflict, vec![cell]));
            }
            let result = if let Some(cipher) = self.receipts.as_ref().and_then(Weak::upgrade) {
                match append::receipt(db, cipher.as_ref(), &query.stream, query.command) {
                    Ok(Some(receipt))
                        if receipt.acceptance().digest() == query.digest
                            && receipt.acceptance().expected_control()
                                == query.expected_control =>
                    {
                        Result::Accepted(receipt.position().control())
                    }
                    Ok(Some(_)) => Result::Conflict,
                    Ok(None) if cell.value().facts().control <= query.expected_control => {
                        Result::NotFound
                    }
                    Ok(None) | Err(JournalError::KeyUnavailable) => Result::Unknown,
                    Err(error) => return Err(error),
                }
            } else {
                Result::Unknown
            };
            Ok((result, vec![cell]))
        })
    }
}
impl CurrentCloudHeads for DurableCurrentCloudHeads {
    fn catalog(
        &self,
        after: Option<CurrentHeadsCursor>,
        limit: u16,
    ) -> JournalFuture<CurrentHeadsPage> {
        let this = self.clone();
        Box::pin(async move { this.0.catalog(after, limit) })
    }
    fn chat(&self, stream: StreamBinding) -> JournalFuture<Option<CurrentChatHead>> {
        let this = self.clone();
        Box::pin(async move { this.0.chat(stream) })
    }
    fn receipt(
        &self,
        query: OriginalCommandReceiptQuery,
    ) -> JournalFuture<OriginalCommandReceiptObservation> {
        let this = self.clone();
        Box::pin(async move { this.0.receipt(query) })
    }
}
