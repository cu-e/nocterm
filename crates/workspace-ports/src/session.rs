//! The remote session behind a tab, as consumers see it.
use nocterm_session::{HostAddress, HostExec, RemoteFs, Target};
use std::sync::Arc;

/// The remote session behind a tab.
#[derive(Clone)]
pub struct SessionContext {
    pub target: Target,
    /// The jump hosts the session was reached through, in order. Two sessions
    /// to equal targets are different machines unless their routes are equal.
    pub via: Vec<Target>,
    /// The host's file system, when the transport offers one.
    pub fs: Option<Arc<dyn RemoteFs>>,
    /// Runs programs on the host, when the transport can.
    pub exec: Option<Arc<dyn HostExec>>,
    /// Whether the session is up. A disconnected session keeps its context,
    /// so views can show what they showed before, greyed out.
    pub connected: bool,
}

impl SessionContext {
    pub fn new(target: Target, fs: Option<Arc<dyn RemoteFs>>, connected: bool) -> Self {
        Self {
            target,
            via: Vec::new(),
            fs,
            exec: None,
            connected,
        }
    }

    /// The same context, reached through `via`.
    pub fn with_via(mut self, via: Vec<Target>) -> Self {
        self.via = via;
        self
    }

    /// The machine this session reaches, with the route it took.
    pub fn address(&self) -> HostAddress {
        HostAddress::new(self.target.clone(), self.via.clone())
    }

    /// Whether this session reaches `host` through exactly its route.
    pub fn reaches(&self, host: &HostAddress) -> bool {
        self.target == host.target && self.via == host.via
    }

    /// The same context, with the host's program runner.
    pub fn with_exec(mut self, exec: Option<Arc<dyn HostExec>>) -> Self {
        self.exec = exec;
        self
    }

    /// Whether two contexts are the same session in the same state.
    pub fn same_as(&self, other: &Self) -> bool {
        self.target == other.target
            && self.via == other.via
            && self.connected == other.connected
            && same_handle(&self.fs, &other.fs)
            && same_handle(&self.exec, &other.exec)
    }
}

fn same_handle<T: ?Sized>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}
