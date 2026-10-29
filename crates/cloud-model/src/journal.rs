//! Atomic command/effect/checkpoint/outbox contracts. Value construction never proves fsync.
mod batch;
mod catalog;
mod cleanup;
mod control;
mod effects;
mod error;
mod heads;
mod position;
mod publication;
mod reader;
mod record;
mod snapshot_publication;
mod storage;
mod writer;

pub use crate::authority::CommandIssuer;
pub use batch::{
    CommandAdmission, CommandReceipt, CommitOutcome, JournalBatch, JournalCommitReceipt,
    OutboxEntry, classify_command,
};
pub use catalog::{
    JournalCatalogBinding, JournalCatalogCipher, JournalCatalogContext, JournalCatalogCursor,
    JournalCatalogPage, JournalCatalogReader, MAX_JOURNAL_CATALOG_CHATS,
    MAX_JOURNAL_CATALOG_CIPHERTEXT, MAX_JOURNAL_CATALOG_PAGE_CHATS, MAX_JOURNAL_CATALOG_PLAINTEXT,
};
pub use cleanup::{CleanupDeclaration, CleanupLease, CleanupReconciliation, RunJournalCleanup};
pub use control::{EngineCheckpointReader, JournalControlReader, JournalHead, JournalWriterFence};
pub use effects::{
    BackgroundRecordOutcome, JournalEventEncoder, TargetEffectReader, UnresolvedTargetEffect,
};
pub use error::JournalError;
pub use heads::{
    CurrentChatHead, CurrentCloudHeads, CurrentHeadFacts, CurrentHeadsCursor, CurrentHeadsPage,
    HeadAnchor, HeadAnchorKind, HeadRecordPosition, JournalChatHeadCipher, JournalChatHeadContext,
    MAX_CURRENT_HEADS_PAGE, MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT, MAX_JOURNAL_CHAT_HEAD_PLAINTEXT,
    OriginalCommandReceiptObservation, OriginalCommandReceiptQuery,
};
pub use position::JournalPosition;
pub use publication::*;
pub use reader::{
    JournalPage, JournalReadLimits, JournalReader, JournalSnapshot, JournalSnapshotWriter,
    StoredRecord,
};
pub use record::{CommandAcceptance, CommandKind, JournalRecord, JournalRecordKind, RecordContext};
pub use snapshot_publication::*;
pub use storage::{
    JournalCipher, JournalCipherContext, JournalStorageClass, MAX_STORED_JOURNAL_BYTES,
};
pub use writer::{RunJournalWriter, WriterClaim, WriterLease};

pub const MAX_JOURNAL_RECORDS: usize = 256;
pub const MAX_TRANSACTION_BYTES: usize = 128 * 1024 * 1024;
/// Conservative space charge for bounded metadata, not a second wire serializer.
const RECORD_METADATA_CHARGE: usize = 16 * 1024;
pub type JournalFuture<T> = futures::future::BoxFuture<'static, Result<T, JournalError>>;
