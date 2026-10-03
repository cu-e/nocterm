//! Deterministic connector for application tests. Each connection is supplied by the test.
use crate::{AgentConnection, AgentConnector, ConnectError, ConnectRequest};
use futures::future::BoxFuture;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default, Clone)]
pub struct ScriptedConnector {
    connections: Arc<Mutex<VecDeque<Result<AgentConnection, ConnectError>>>>,
    requests: Arc<Mutex<Vec<ConnectRequest>>>,
}
impl ScriptedConnector {
    pub fn push(&self, connection: Result<AgentConnection, ConnectError>) {
        self.connections
            .lock()
            .expect("test queue")
            .push_back(connection);
    }
    pub fn requests(&self) -> Vec<ConnectRequest> {
        self.requests.lock().expect("test requests").clone()
    }
}
impl AgentConnector for ScriptedConnector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, ConnectError>> {
        self.requests.lock().expect("test requests").push(request);
        let result = self
            .connections
            .lock()
            .expect("test queue")
            .pop_front()
            .unwrap_or_else(|| Err(ConnectError::Io("No scripted connection".into())));
        Box::pin(async move { result })
    }
}
