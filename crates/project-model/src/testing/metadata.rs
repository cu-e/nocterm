use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{ProjectId, ProjectIdentity, ProjectMetadataError, ProjectMetadataSource};

/// A deterministic supplied sequence and clock, without native entropy or time.
pub struct SequenceProjectMetadata {
    first: u128,
    allocations: AtomicU64,
    timestamp: AtomicU64,
    failures: Mutex<VecDeque<ProjectMetadataError>>,
}

impl SequenceProjectMetadata {
    pub fn new(first: u128, timestamp: u64) -> Self {
        Self {
            first,
            allocations: AtomicU64::new(0),
            timestamp: AtomicU64::new(timestamp),
            failures: Mutex::new(VecDeque::new()),
        }
    }

    pub fn set_timestamp(&self, timestamp: u64) {
        self.timestamp.store(timestamp, Ordering::SeqCst);
    }

    pub fn fail_next(&self, error: ProjectMetadataError) {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(error);
    }
}

impl ProjectMetadataSource for SequenceProjectMetadata {
    fn new_identity(&self) -> Result<ProjectIdentity, ProjectMetadataError> {
        if let Some(error) = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
        {
            return Err(error);
        }
        let offset = self
            .allocations
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |next| {
                next.checked_add(1)
            })
            .map_err(|_| ProjectMetadataError::Exhausted)?;
        let value = self
            .first
            .checked_add(u128::from(offset))
            .ok_or(ProjectMetadataError::Exhausted)?;
        Ok(ProjectIdentity {
            id: ProjectId::from_bytes(value.to_be_bytes()),
            created: self.timestamp(),
        })
    }

    fn timestamp(&self) -> u64 {
        self.timestamp.load(Ordering::SeqCst)
    }
}
