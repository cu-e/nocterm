//! Prepared documents and injected collaborators, installed without foreground native acquisition.
use crate::{
    Connections, FactsPrepared, NoSavedPasswords, ObservationDeps, SavedPasswords,
    ServerAgentAccess,
};
use nocterm_connections_model::{
    draft::EditorDefaults,
    facts::Known,
    profiles::{ProfileId, Profiles},
    recents::Recents,
    repository::LoadedConnections,
    value_ports::{ConnectionClock, ProfileIds, SuppliedValues},
};
use nocterm_ssh_keys::{NoVaultSshKeys, VaultSshKeys};
use nocterm_store::SnapshotWriter;
use std::sync::Arc;
#[derive(Clone)]
pub struct EditorInputs {
    defaults: EditorDefaults,
    ids: Arc<dyn ProfileIds>,
    keys: Arc<dyn VaultSshKeys>,
    passwords: Arc<dyn SavedPasswords>,
    agent_access: Option<Arc<dyn ServerAgentAccess>>,
}
impl EditorInputs {
    /// Inputs of a window without a vault: its key and password choices report that.
    pub fn new(defaults: EditorDefaults, ids: Arc<dyn ProfileIds>) -> Self {
        Self {
            defaults,
            ids,
            keys: Arc::new(NoVaultSshKeys),
            passwords: Arc::new(NoSavedPasswords),
            agent_access: None,
        }
    }
    /// A password typed in the editor is kept in `passwords`.
    pub fn with_passwords(mut self, passwords: Arc<dyn SavedPasswords>) -> Self {
        self.passwords = passwords;
        self
    }
    pub(crate) fn passwords(&self) -> &Arc<dyn SavedPasswords> {
        &self.passwords
    }
    /// The keys the editor offers are those of `keys`.
    pub fn with_keys(mut self, keys: Arc<dyn VaultSshKeys>) -> Self {
        self.keys = keys;
        self
    }
    pub(crate) fn keys(&self) -> &Arc<dyn VaultSshKeys> {
        &self.keys
    }
    /// The editor offers Agent access of a saved server through `access`.
    pub fn with_agent_access(mut self, access: Arc<dyn ServerAgentAccess>) -> Self {
        self.agent_access = Some(access);
        self
    }
    pub(crate) fn agent_access(&self) -> Option<&Arc<dyn ServerAgentAccess>> {
        self.agent_access.as_ref()
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
/// What the connections need besides documents and writers: the same for the saved list of
/// this computer and for a list kept only in memory, so a window can be given either.
#[derive(Clone)]
pub struct ConnectionInputs {
    editor: EditorInputs,
    clock: Arc<dyn ConnectionClock>,
    observations: ObservationDeps,
}
impl ConnectionInputs {
    pub fn new(
        editor: EditorInputs,
        clock: Arc<dyn ConnectionClock>,
        observations: ObservationDeps,
    ) -> Self {
        Self {
            editor,
            clock,
            observations,
        }
    }
    /// The editor offers the keys of `keys` instead of reporting no vault.
    pub fn with_keys(mut self, keys: Arc<dyn VaultSshKeys>) -> Self {
        self.editor = self.editor.with_keys(keys);
        self
    }
    /// The editor keeps a typed password in `passwords` instead of reporting no vault.
    pub fn with_passwords(mut self, passwords: Arc<dyn SavedPasswords>) -> Self {
        self.editor = self.editor.with_passwords(passwords);
        self
    }
    /// The editor offers Agent access of a saved server through `access`.
    pub fn with_agent_access(mut self, access: Arc<dyn ServerAgentAccess>) -> Self {
        self.editor = self.editor.with_agent_access(access);
        self
    }
}
pub struct PreparedConnections {
    documents: LoadedConnections,
    writers: Option<ConnectionWriters>,
    inputs: ConnectionInputs,
}
impl PreparedConnections {
    pub fn new(
        documents: LoadedConnections,
        writers: ConnectionWriters,
        inputs: ConnectionInputs,
    ) -> Self {
        Self {
            documents,
            writers: Some(writers),
            inputs,
        }
    }
    /// An empty list kept only in memory, for a list that something else keeps: it writes
    /// nothing, and new connections get the identifiers `inputs` supplies.
    pub fn volatile(inputs: ConnectionInputs) -> Self {
        Self {
            documents: LoadedConnections::default(),
            writers: None,
            inputs,
        }
    }
    /// The inputs these connections were prepared with, to prepare another list the same way.
    pub fn inputs(&self) -> ConnectionInputs {
        self.inputs.clone()
    }
    /// The editor offers the keys of `keys` instead of reporting no vault.
    pub fn with_keys(mut self, keys: Arc<dyn VaultSshKeys>) -> Self {
        self.inputs = self.inputs.with_keys(keys);
        self
    }
    /// The editor keeps a typed password in `passwords` instead of reporting no vault.
    pub fn with_passwords(mut self, passwords: Arc<dyn SavedPasswords>) -> Self {
        self.inputs = self.inputs.with_passwords(passwords);
        self
    }
    pub fn memory(defaults: EditorDefaults) -> Self {
        Self::memory_with_keys(defaults, Arc::new(NoVaultSshKeys))
    }
    pub fn memory_with_keys(defaults: EditorDefaults, keys: Arc<dyn VaultSshKeys>) -> Self {
        let values = Arc::new(SuppliedValues::new(1, 0));
        Self::volatile(ConnectionInputs::new(
            EditorInputs::new(defaults, values.clone()).with_keys(keys),
            values,
            ObservationDeps::offline(),
        ))
    }
    pub(crate) fn into_owners(self) -> (Connections, EditorInputs, FactsPrepared) {
        let profiles = self.documents.profiles;
        let recents = self.documents.recents;
        let ConnectionInputs {
            editor,
            clock,
            observations,
        } = self.inputs;
        let (connections, facts) = match self.writers {
            Some(writers) => (
                Connections::prepared(
                    profiles,
                    recents,
                    writers.profiles,
                    writers.recents,
                    self.documents.read_only,
                    clock,
                ),
                FactsPrepared::new(self.documents.known, writers.facts, observations),
            ),
            None => (
                Connections::memory_with_clock(profiles, recents, clock),
                FactsPrepared::in_memory(observations),
            ),
        };
        (connections, editor, facts)
    }
}
