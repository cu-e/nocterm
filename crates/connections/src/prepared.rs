//! Prepared documents and injected collaborators, installed without foreground native acquisition.
use crate::{Connections, FactsPrepared, ObservationDeps};
use nocterm_connections_model::{
    draft::EditorDefaults,
    facts::Known,
    profiles::{ProfileId, Profiles},
    recents::Recents,
    repository::LoadedConnections,
    value_ports::{ConnectionClock, ProfileIds, SuppliedValues},
};
use nocterm_store::SnapshotWriter;
use std::sync::Arc;
#[derive(Clone)]
pub struct EditorInputs {
    defaults: EditorDefaults,
    ids: Arc<dyn ProfileIds>,
}
impl EditorInputs {
    pub fn new(defaults: EditorDefaults, ids: Arc<dyn ProfileIds>) -> Self {
        Self { defaults, ids }
    }
    pub fn defaults(&self) -> &EditorDefaults {
        &self.defaults
    }
    pub(crate) fn next_id(&self) -> ProfileId {
        self.ids.next()
    }
}
pub struct ConnectionWriters {
    profiles: SnapshotWriter<Profiles>,
    recents: SnapshotWriter<Recents>,
    facts: SnapshotWriter<Known>,
}
impl ConnectionWriters {
    pub fn new(
        profiles: SnapshotWriter<Profiles>,
        recents: SnapshotWriter<Recents>,
        facts: SnapshotWriter<Known>,
    ) -> Self {
        Self {
            profiles,
            recents,
            facts,
        }
    }
}
pub struct PreparedConnections {
    documents: LoadedConnections,
    writers: Option<ConnectionWriters>,
    inputs: EditorInputs,
    clock: Arc<dyn ConnectionClock>,
    observations: ObservationDeps,
}
impl PreparedConnections {
    pub fn new(
        documents: LoadedConnections,
        writers: ConnectionWriters,
        inputs: EditorInputs,
        clock: Arc<dyn ConnectionClock>,
        observations: ObservationDeps,
    ) -> Self {
        Self {
            documents,
            writers: Some(writers),
            inputs,
            clock,
            observations,
        }
    }
    pub fn memory(defaults: EditorDefaults) -> Self {
        let values = Arc::new(SuppliedValues::new(1, 0));
        Self {
            documents: LoadedConnections::default(),
            writers: None,
            inputs: EditorInputs::new(defaults, values.clone()),
            clock: values,
            observations: ObservationDeps::offline(),
        }
    }
    pub(crate) fn into_owners(self) -> (Connections, EditorInputs, FactsPrepared) {
        let profiles = self.documents.profiles;
        let recents = self.documents.recents;
        let (connections, facts) = match self.writers {
            Some(writers) => (
                Connections::prepared(
                    profiles,
                    recents,
                    writers.profiles,
                    writers.recents,
                    self.documents.read_only,
                    self.clock,
                ),
                FactsPrepared::new(self.documents.known, writers.facts, self.observations),
            ),
            None => (
                Connections::memory_with_clock(profiles, recents, self.clock),
                FactsPrepared::in_memory(self.observations),
            ),
        };
        (connections, self.inputs, facts)
    }
}
