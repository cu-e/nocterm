//! The file that remembers what the saved profiles and their replica last agreed on.

use std::io;
use std::path::PathBuf;

use nocterm_connections_model::replica::ReplicaState;
use nocterm_core::fs::{BoundedReadError, FileError, read_regular_bounded, write_private_atomic};

/// The agreement holds copies of the saved profiles; anything larger is not ours.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Why the agreed state could not be kept or read.
#[derive(Debug, thiserror::Error)]
pub enum ReplicaStateError {
    #[error("the replica state cannot be read")]
    Read(#[from] BoundedReadError),
    #[error("the replica state is not valid")]
    Format(#[source] serde_json::Error),
    #[error("the replica state cannot be written")]
    Write(#[from] FileError),
}

/// A private JSON file, replaced atomically on every write.
pub struct FileReplicaState {
    path: PathBuf,
}

impl FileReplicaState {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The state of the last run. A file that is missing starts from nothing; one that cannot
    /// be read does too, and is logged: the profiles are then reconciled again from both sides
    /// and nothing is deleted for lack of a record.
    pub fn load(&self) -> ReplicaState {
        match self.read() {
            Ok(state) => state,
            Err(ReplicaStateError::Read(BoundedReadError::Io { source, .. }))
                if source.kind() == io::ErrorKind::NotFound =>
            {
                ReplicaState::default()
            }
            Err(error) => {
                tracing::warn!(%error, "the profile replica state starts again");
                ReplicaState::default()
            }
        }
    }

    fn read(&self) -> Result<ReplicaState, ReplicaStateError> {
        let bytes = read_regular_bounded(&self.path, MAX_BYTES)?;
        serde_json::from_slice(&bytes).map_err(ReplicaStateError::Format)
    }

    pub fn save(&self, state: &ReplicaState) -> Result<(), ReplicaStateError> {
        let bytes = serde_json::to_vec(state).map_err(ReplicaStateError::Format)?;
        write_private_atomic(&self.path, &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
