//! Native files; application dispatches blocking capability calls on background workers.
use nocterm_connections_model::{
    facts::Known,
    profiles::Profiles,
    recents::Recents,
    repository::{
        ConnectionDocument, ConnectionRepository, ConnectionStorageError, LoadedConnections,
        StorageOperation,
    },
};
use nocterm_core::persist;
use std::{path::PathBuf, sync::Arc};
pub struct FileConnectionRepository {
    profiles: PathBuf,
    recents: PathBuf,
    facts: PathBuf,
}
impl FileConnectionRepository {
    pub fn new(profiles: PathBuf, recents: PathBuf, facts: PathBuf) -> Self {
        Self {
            profiles,
            recents,
            facts,
        }
    }
}
impl ConnectionRepository for FileConnectionRepository {
    fn load(&self) -> LoadedConnections {
        let (profiles, read_only) = match persist::load::<Profiles>(&self.profiles) {
            Ok(value) => (value.unwrap_or_default(), None),
            Err(error) => {
                tracing::error!(%error, "could not read saved connections");
                (
                    Profiles::default(),
                    Some(Arc::new(ConnectionStorageError::new(
                        ConnectionDocument::Profiles,
                        StorageOperation::Read,
                        error,
                    ))),
                )
            }
        };
        let recents = persist::load(&self.recents)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "could not read recent connections");
                None
            })
            .unwrap_or_default();
        let known = persist::load(&self.facts)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "could not read detected server details");
                None
            })
            .unwrap_or_default();
        LoadedConnections {
            profiles,
            recents,
            known,
            read_only,
        }
    }
    fn save_profiles(&self, profiles: &Profiles) -> Result<(), ConnectionStorageError> {
        persist::save_preserving(&self.profiles, profiles).map_err(|error| {
            ConnectionStorageError::new(
                ConnectionDocument::Profiles,
                StorageOperation::Write,
                error,
            )
        })
    }
    fn save_recents(&self, recents: &Recents) -> Result<(), ConnectionStorageError> {
        persist::save(&self.recents, recents).map_err(|error| {
            ConnectionStorageError::new(ConnectionDocument::Recents, StorageOperation::Write, error)
        })
    }
    fn save_facts(&self, known: &Known) -> Result<(), ConnectionStorageError> {
        persist::save(&self.facts, known).map_err(|error| {
            ConnectionStorageError::new(ConnectionDocument::Facts, StorageOperation::Write, error)
        })
    }
}
#[cfg(test)]
mod tests;
