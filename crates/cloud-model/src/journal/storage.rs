use super::JournalError;
use crate::{
    authority::StreamBinding,
    identity::{ChatKeyEpoch, JournalSequence, StreamRevision, WriterEpoch},
    payload::MAX_LOGICAL_PAYLOAD_BYTES,
};

/// Native record metadata and purpose-specific authentication overhead are bounded
/// separately from a logical chat payload. This is not a hub envelope limit.
pub const MAX_STORED_JOURNAL_BYTES: usize = MAX_LOGICAL_PAYLOAD_BYTES + 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalStorageClass {
    Header,
    Record,
    CommandReceipt,
    Checkpoint,
    Snapshot,
    Outbox,
    BackgroundCommand,
    Effect,
    RunHead,
    /// A chat's SSH row: subject key seed and access (journal v8, ADR 0145).
    SshAccess,
}

/// Authenticated storage coordinates. A cipher must bind every field and use a
/// journal-only purpose, independent from command, event and key-grant purposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCipherContext {
    stream: StreamBinding,
    class: JournalStorageClass,
    id: [u8; 16],
    sequence: JournalSequence,
    revision: StreamRevision,
    writer: WriterEpoch,
}
impl JournalCipherContext {
    pub fn new(
        stream: StreamBinding,
        class: JournalStorageClass,
        id: [u8; 16],
        sequence: JournalSequence,
        revision: StreamRevision,
        writer: WriterEpoch,
    ) -> Self {
        Self {
            stream,
            class,
            id,
            sequence,
            revision,
            writer,
        }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn class(&self) -> JournalStorageClass {
        self.class
    }
    pub fn id(&self) -> &[u8; 16] {
        &self.id
    }
    pub fn sequence(&self) -> JournalSequence {
        self.sequence
    }
    pub fn revision(&self) -> StreamRevision {
        self.revision
    }
    pub fn writer(&self) -> WriterEpoch {
        self.writer
    }
}

/// The contract-cryptography adapter supplies authenticated encryption for native
/// journal records. No plaintext production implementation or implicit key lookup
/// is permitted. Implementations reject missing/rotated keys and changed context.
/// The native driver validates both returned lengths before storing or decoding.
/// Calls perform only bounded in-memory cryptography, never network or target I/O.
pub trait JournalCipher: Send + Sync {
    /// Actual verified active material from this configured owner, never the R
    /// epoch or a SQL selector. Metadata-only reads do not require this key.
    fn chat_key_epoch(&self, stream: &StreamBinding) -> Result<ChatKeyEpoch, JournalError>;
    /// Native AEAD output overhead is bounded by 512 bytes before allocation.
    fn seal(
        &self,
        context: &JournalCipherContext,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, JournalError>;
    /// Non-compressing native AEAD: plaintext allocation is bounded by sealed
    /// length before allocation; native codecs rely on this bound for permits.
    fn open(&self, context: &JournalCipherContext, sealed: &[u8]) -> Result<Vec<u8>, JournalError>;
}
