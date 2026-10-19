//! The managed connector consumes an original fixture FIFO and records each workspace.
use crate::{
    AgentConnector,
    connection::{ManagedAgentConnection, ManagedConnectRequest, ManagedWorkspaceHandle},
    history::source::{ManagedCommandError, ManagedSourceFuture},
    model::json::deferred::NativeAllocationError,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
pub type ManagedConnectFuture = Result<
    ManagedSourceFuture<Result<ManagedAgentConnection, ManagedCommandError>>,
    NativeAllocationError,
>;
type Factory = Box<dyn FnOnce(ManagedConnectRequest) -> ManagedConnectFuture + Send>;
#[derive(Default, Clone)]
pub struct ScriptedConnector {
    connections: Arc<Mutex<VecDeque<Factory>>>,
    requests: Arc<Mutex<Vec<ManagedWorkspaceHandle>>>,
}
impl ScriptedConnector {
    /// Queues one connection of `commands`; connections of one script share its handle.
    pub fn push_scripted(&self, commands: &super::ScriptedCommands) {
        let commands = commands.clone();
        self.push_managed(move |request| commands.connect(request));
    }
    /// The next connection fails with a typed fixture error, not a substituted success.
    pub fn push_failed(&self) {
        self.push_managed(|request| {
            let slot = request.context().prepare_error::<FixtureError>(0)?;
            ManagedSourceFuture::new(request.lifetime(), async move {
                Err(ManagedCommandError::Native(
                    slot.capture(FixtureError::Scripted),
                ))
            })
        });
    }
    /// Fixture construction is explicit; the supplied factory must admit its
    /// original managed graph using this request's actual Source before boxing.
    pub fn push_managed<F>(&self, factory: F)
    where
        F: FnOnce(ManagedConnectRequest) -> ManagedConnectFuture + Send + 'static,
    {
        self.connections
            .lock()
            .expect("test queue")
            .push_back(Box::new(factory));
    }
    /// The prepared workspace root of every connection request, in order.
    pub fn working_directories(&self) -> Vec<PathBuf> {
        self.requests
            .lock()
            .expect("test requests")
            .iter()
            .map(|workspace| workspace.workspace().root.clone())
            .collect()
    }
    pub(crate) fn managed(&self, request: ManagedConnectRequest) -> ManagedConnectFuture {
        self.requests
            .lock()
            .expect("test requests")
            .push(request.workspace_handle());
        let entry = self.connections.lock().expect("test queue").pop_front();
        match entry {
            Some(factory) => factory(request),
            None => {
                let slot = request.context().prepare_error::<FixtureError>(0)?;
                ManagedSourceFuture::new(request.lifetime(), async move {
                    Err(ManagedCommandError::Native(
                        slot.capture(FixtureError::Empty),
                    ))
                })
            }
        }
    }
}
impl AgentConnector for ScriptedConnector {
    fn connect_managed(&self, request: ManagedConnectRequest) -> ManagedConnectFuture {
        self.managed(request)
    }
}
#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("No scripted connection")]
    Empty,
    #[error("Scripted connection failure")]
    Scripted,
}
