//! Narrow reads for command admission, recovery and current native writer fencing.
use super::{CommandReceipt, JournalError, JournalFuture, JournalPosition, WriterLease};
use crate::{
    authority::StreamBinding,
    engine::EngineCheckpoint,
    identity::{CommandId, JournalSequence},
};

/// Descriptive observation; neither construction nor restoration acquires authority.
#[derive(Clone, Debug)]
pub struct JournalHead {
    lease: WriterLease,
    position: JournalPosition,
    retained: JournalSequence,
    deleted: bool,
}
impl JournalHead {
    pub fn new(
        lease: WriterLease,
        position: JournalPosition,
        retained: JournalSequence,
        deleted: bool,
    ) -> Result<Self, JournalError> {
        if retained > position.records()
            || position.control().get() > position.revision().get()
            || position.revision().get() > position.records().get()
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            lease,
            position,
            retained,
            deleted,
        })
    }
    pub fn lease(&self) -> &WriterLease {
        &self.lease
    }
    pub fn position(&self) -> JournalPosition {
        self.position
    }
    pub fn retained(&self) -> JournalSequence {
        self.retained
    }
    pub fn deleted(&self) -> bool {
        self.deleted
    }
}

pub trait JournalControlReader: Send + Sync {
    fn head(&self, stream: StreamBinding) -> JournalFuture<Option<JournalHead>>;
    /// The original immutable receipt, even after writer/control renewal or a
    /// tombstone. Its digest/issuer must still be classified before acceptance.
    fn command_receipt(
        &self,
        stream: StreamBinding,
        id: CommandId,
    ) -> JournalFuture<Option<CommandReceipt>>;
}
pub trait EngineCheckpointReader: Send + Sync {
    fn checkpoint(&self, stream: StreamBinding) -> JournalFuture<Option<EngineCheckpoint>>;
}
pub trait JournalWriterFence: Send + Sync {
    /// Synchronous native check immediately before I/O. Unavailable storage is
    /// a refusal, never permission inferred from an old descriptive lease.
    fn require_current(&self, lease: &WriterLease) -> Result<(), JournalError>;
}
