//! Scripted document ports; state and operation outcomes are observed through methods.
use crate::{
    facts::Known,
    profiles::Profiles,
    recents::Recents,
    repository::{
        ConnectionDocument, ConnectionRepository, ConnectionStorageError, LoadedConnections,
    },
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryCall {
    Load,
    Save(ConnectionDocument),
}
pub struct InMemoryConnectionRepository {
    documents: Mutex<LoadedConnections>,
    failures: Mutex<VecDeque<ConnectionStorageError>>,
    calls: Mutex<Vec<RepositoryCall>>,
    gates: Mutex<VecDeque<(ConnectionDocument, Arc<RepositoryGate>)>>,
}
impl InMemoryConnectionRepository {
    pub fn new(documents: LoadedConnections) -> Self {
        Self {
            documents: Mutex::new(documents),
            failures: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            gates: Mutex::new(VecDeque::new()),
        }
    }
    pub fn snapshot(&self) -> LoadedConnections {
        self.documents.lock().unwrap().clone()
    }
    pub fn replace(&self, documents: LoadedConnections) {
        *self.documents.lock().unwrap() = documents;
    }
    pub fn fail_next(&self, error: ConnectionStorageError) {
        self.failures.lock().unwrap().push_back(error);
    }
    /// The caller releases this explicit gate; the fake owns no scheduling or native worker.
    pub fn suspend_next(&self, document: ConnectionDocument, gate: Arc<RepositoryGate>) {
        self.gates.lock().unwrap().push_back((document, gate));
    }
    pub fn calls(&self) -> Vec<RepositoryCall> {
        self.calls.lock().unwrap().clone()
    }
    fn save(
        &self,
        document: ConnectionDocument,
        update: impl FnOnce(&mut LoadedConnections),
    ) -> Result<(), ConnectionStorageError> {
        self.calls
            .lock()
            .unwrap()
            .push(RepositoryCall::Save(document));
        let gate = {
            let mut gates = self.gates.lock().unwrap();
            gates
                .front()
                .is_some_and(|(target, _)| *target == document)
                .then(|| gates.pop_front().unwrap().1)
        };
        if let Some(gate) = gate {
            gate.wait();
        }
        let mut failures = self.failures.lock().unwrap();
        if failures
            .front()
            .is_some_and(|error| error.document() == document)
        {
            return Err(failures.pop_front().unwrap());
        }
        update(&mut self.documents.lock().unwrap());
        Ok(())
    }
}
pub use super::gates::BlockingGate as RepositoryGate;
impl ConnectionRepository for InMemoryConnectionRepository {
    fn load(&self) -> LoadedConnections {
        self.calls.lock().unwrap().push(RepositoryCall::Load);
        self.snapshot()
    }
    fn save_profiles(&self, profiles: &Profiles) -> Result<(), ConnectionStorageError> {
        self.save(ConnectionDocument::Profiles, |documents| {
            documents.profiles = profiles.clone()
        })
    }
    fn save_recents(&self, recents: &Recents) -> Result<(), ConnectionStorageError> {
        self.save(ConnectionDocument::Recents, |documents| {
            documents.recents = recents.clone()
        })
    }
    fn save_facts(&self, known: &Known) -> Result<(), ConnectionStorageError> {
        self.save(ConnectionDocument::Facts, |documents| {
            documents.known = known.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::StorageOperation;
    #[test]
    fn rejected_document_write_keeps_snapshot_and_retains_native_source() {
        use std::error::Error as _;
        let repository = InMemoryConnectionRepository::new(LoadedConnections::default());
        repository.fail_next(ConnectionStorageError::new(
            ConnectionDocument::Profiles,
            StorageOperation::Write,
            crate::profiles::ProfileIdParseError::from(
                "invalid".parse::<uuid::Uuid>().unwrap_err(),
            ),
        ));
        let error = repository.save_profiles(&Profiles::default()).unwrap_err();
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<crate::profiles::ProfileIdParseError>()
                .is_some()
        );
        assert!(repository.snapshot().profiles.is_empty());
        assert_eq!(
            repository.calls(),
            vec![RepositoryCall::Save(ConnectionDocument::Profiles)]
        );
    }
}
