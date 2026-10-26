//! One local agent connection retains its original conversation and prepared directory.
use nocterm_ai::{
    AgentLaunch,
    execution_origin::{ChatKey, ExecutionOrigin},
};
use std::path::{Path, PathBuf};

pub(super) fn admitted_origin(
    origin: &ExecutionOrigin,
    requested_id: &str,
) -> Result<ExecutionOrigin, crate::AgentActivationError> {
    if origin.key().id() != requested_id {
        return Err(crate::AgentActivationError::Storage(
            nocterm_ai::history::HistoryCodecError::IdentityMismatch.into(),
        ));
    }
    Ok(origin.clone())
}

#[derive(Hash)]
pub(super) struct ExecutionIdentity {
    origin: ExecutionOrigin,
    launch: AgentLaunch,
    workdir: PathBuf,
    isolated: bool,
}
impl ExecutionIdentity {
    pub(super) fn new(
        origin: ExecutionOrigin,
        launch: AgentLaunch,
        workdir: PathBuf,
        isolated: bool,
    ) -> Self {
        Self {
            origin,
            launch,
            workdir,
            isolated,
        }
    }
    pub(super) fn key(&self) -> &ChatKey {
        self.origin.key()
    }
    pub(super) fn launch(&self) -> &AgentLaunch {
        &self.launch
    }
    pub(super) fn workdir(&self) -> &Path {
        &self.workdir
    }
    pub(super) fn prepared_directory(&mut self, workdir: PathBuf) {
        self.workdir = workdir;
    }
    pub(super) fn isolation_changed(&self, isolated: bool) -> bool {
        self.isolated != isolated
    }
}
