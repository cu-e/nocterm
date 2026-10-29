//! Installation-owned physical drivers shared across original login captures.
//! Weak entries never pin keys, memberships or file locks. Actual I/O retains a
//! strong driver and its pre-admitted metadata through physical completion.
use super::persistence::{Drivers, Files};
use nocterm_cloud_model::projection::ProjectionError;
use nocterm_core::allocation::AllocationLease;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

pub struct ConversationStorage {
    root: PathBuf,
    maximum: usize,
    drivers: Mutex<BTreeMap<PathBuf, Weak<Drivers>>>,
    allocation: AllocationLease,
}
impl ConversationStorage {
    /// Reserve all registry/driver metadata before constructing this installation
    /// owner. Payloads and actual blocking/RPC work have separate allowances.
    pub fn resident_bytes(maximum: usize) -> Result<usize, ProjectionError> {
        if !(1..=128).contains(&maximum) {
            return Err(ProjectionError::Capacity);
        }
        maximum
            .checked_mul(64 * 1024)
            .and_then(|n| n.checked_add(8192))
            .ok_or(ProjectionError::Capacity)
    }
    pub fn new(
        root: PathBuf,
        maximum: usize,
        allocation: AllocationLease,
    ) -> Result<Arc<Self>, ProjectionError> {
        if root.as_os_str().len() > 4096
            || allocation.payloads() == 0
            || allocation.bytes() < Self::resident_bytes(maximum)?
        {
            return Err(ProjectionError::Capacity);
        }
        Ok(Arc::new(Self {
            root,
            maximum,
            drivers: Mutex::new(BTreeMap::new()),
            allocation,
        }))
    }
    pub(super) fn root(&self) -> &Path {
        &self.root
    }
    /// Called only in the bounded blocking lane. Holding the registry lock
    /// serializes first-open vs relogin; a live old view upgrades the same driver.
    pub(super) fn open(&self, root: PathBuf) -> Result<Files, ProjectionError> {
        if root.parent() != Some(self.root.as_path()) {
            return Err(ProjectionError::WrongSource);
        }
        let mut drivers = self
            .drivers
            .lock()
            .map_err(|_| ProjectionError::Unavailable)?;
        if let Some(files) = drivers.get(&root).and_then(Files::upgrade) {
            return Ok(files);
        }
        drivers.retain(|_, driver| driver.strong_count() != 0);
        if drivers.len() >= self.maximum {
            return Err(ProjectionError::Capacity);
        }
        let files = Files::create(&root, self.allocation.clone())?;
        drivers.insert(root, files.weak());
        Ok(files)
    }
}
