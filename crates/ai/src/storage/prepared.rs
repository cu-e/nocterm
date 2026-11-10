//! Owned bounded source bytes are decoded only after receiving the caller's lease.
use super::{AgentStorageError, StorageFuture, require_local_scope};
use crate::{
    history::{self, SavedChat},
    local_environment::ChatScope,
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::fmt;

/// An original source prepared by a repository, not a fresh pathname lookup.
/// The estimate covers decoded retained objects; separately bounded native input
/// scratch remains owned by this handle until decoding or cancellation finishes.
pub trait PreparedChatLoad: Send {
    fn resident_bytes(&self) -> usize;
    fn decode(self: Box<Self>, allocation: AllocationLease) -> StorageFuture<Allocated<SavedChat>>;
}

/// Pure shared implementation for native repositories and their canonical fake.
/// Creation preflights bytes without constructing a decoded SavedChat. Matching
/// descriptive coordinates confer no filesystem or account authority.
pub struct PreparedChatBytes {
    bytes: Vec<u8>,
    scope: ChatScope,
    id: String,
    expected_version: Option<u32>,
    resident: usize,
}
impl PreparedChatBytes {
    pub fn new(
        bytes: Vec<u8>,
        scope: ChatScope,
        id: String,
        expected_version: Option<u32>,
    ) -> Result<Self, AgentStorageError> {
        if bytes.len() as u64 > history::MAX_FILE_BYTES {
            return Err(history::HistoryCodecError::EncodedLimit.into());
        }
        if let Some(version) = expected_version
            && !(1..=3).contains(&version)
        {
            return Err(history::HistoryCodecError::UnsupportedVersion(version).into());
        }
        let resident = history::structural_bytes(&bytes)?;
        Ok(Self {
            bytes,
            scope,
            id,
            expected_version,
            resident,
        })
    }
}
impl fmt::Debug for PreparedChatBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedChatBytes")
            .field("resident_bytes", &self.resident)
            .field("expected_version", &self.expected_version)
            .finish_non_exhaustive()
    }
}
impl PreparedChatLoad for PreparedChatBytes {
    fn resident_bytes(&self) -> usize {
        self.resident
    }
    fn decode(self: Box<Self>, allocation: AllocationLease) -> StorageFuture<Allocated<SavedChat>> {
        Box::pin(async move {
            if allocation.payloads() == 0 {
                return Err(AgentStorageError::PayloadCapacity);
            }
            if allocation.bytes() < self.resident {
                return Err(AgentStorageError::ResidentCapacity {
                    retained: 0,
                    incoming: self.resident,
                    limit: allocation.bytes(),
                });
            }
            let chat = history::decode(&self.bytes, &self.id)?;
            require_local_scope(&self.scope, &chat.header.chat_scope())?;
            if self
                .expected_version
                .is_some_and(|version| version != chat.version)
            {
                return Err(history::HistoryCodecError::InvalidHeader.into());
            }
            Ok(Allocated::new(chat, allocation))
        })
    }
}
