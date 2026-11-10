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
    pending: VecDeque<(Session, crate::ConnectionSource)>,
    requests: VecDeque<ConnectRequest>,
}

impl ScriptedTransport {
    /// Queue one session; the caller drives its progress, prompts and output.
    pub fn queue(
        &self,
        fs: Option<Arc<dyn RemoteFs>>,
        exec: Option<Arc<dyn HostExec>>,
    ) -> SessionDriver {
        self.queue_with_source(fs, exec, Default::default())
    }
    /// The caller clones the same immutable source into open options. A live
    /// queued session is never rebound to a source chosen by a later open.
    pub fn queue_with_source(
        &self,
        fs: Option<Arc<dyn RemoteFs>>,
        exec: Option<Arc<dyn HostExec>>,
        source: crate::ConnectionSource,
    ) -> SessionDriver {
        let (mut session, driver) = crate::channel_with_source(fs, source.clone());
        if let Some(exec) = exec {
            session = session.with_exec(exec);
        }
        lock(&self.state).pending.push_back((session, source));
        driver
    }

    /// Queue an already prepared channel, retaining its original driver identity.
    pub fn enqueue_session(&self, session: Session) {
        lock(&self.state)
            .pending
            .push_back((session, Default::default()));
    }

    /// The last 4096 open requests, oldest first.
    pub fn requests(&self) -> Vec<ConnectRequest> {
        lock(&self.state).requests.iter().cloned().collect()
    }
}

impl Transport for ScriptedTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        self.open_with(request, Default::default())
    }
    fn open_with(&self, request: ConnectRequest, options: crate::ConnectionOpenOptions) -> Session {
        if let Err(error) = options.source().check() {
            return Session::failed(error.into());
        }

        let mut script = lock(&self.state);
        if script.requests.len() == 4096 {
            script.requests.pop_front();
        }
        script.requests.push_back(request);
        match script.pending.pop_front() {
            Some((session, source))
                if !options.source().is_restricted() || options.source().same_owner(&source) =>
            {
                session
            }
            Some(_) => Session::failed(crate::ConnectionSourceError::Unsupported.into()),
            None => Session::failed(SessionError::Cancelled),
        }
    }
}
