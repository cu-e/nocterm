use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use super::lock;
use crate::{ConnectRequest, HostExec, RemoteFs, Session, SessionDriver, SessionError, Transport};

/// A transport with sessions prepared by a test. The returned drivers expose the
/// real command/event channels, including independent shutdown notification.
#[derive(Clone, Default)]
pub struct ScriptedTransport {
    state: Arc<Mutex<TransportScript>>,
}

#[derive(Default)]
struct TransportScript {
    pending: VecDeque<Session>,
    requests: VecDeque<ConnectRequest>,
}

impl ScriptedTransport {
    /// Queue one session; the caller drives its progress, prompts and output.
    pub fn queue(
        &self,
        fs: Option<Arc<dyn RemoteFs>>,
        exec: Option<Arc<dyn HostExec>>,
    ) -> SessionDriver {
        let (mut session, driver) = crate::channel(fs);
        if let Some(exec) = exec {
            session = session.with_exec(exec);
        }
        lock(&self.state).pending.push_back(session);
        driver
    }

    /// Queue an already prepared channel, retaining its original driver identity.
    pub fn enqueue_session(&self, session: Session) {
        lock(&self.state).pending.push_back(session);
    }

    /// The last 4096 open requests, oldest first.
    pub fn requests(&self) -> Vec<ConnectRequest> {
        lock(&self.state).requests.iter().cloned().collect()
    }
}

impl Transport for ScriptedTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        let mut script = lock(&self.state);
        if script.requests.len() == 4096 {
            script.requests.pop_front();
        }
        script.requests.push_back(request);
        script
            .pending
            .pop_front()
            .unwrap_or_else(|| Session::failed(SessionError::Cancelled))
    }
}
