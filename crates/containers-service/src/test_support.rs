//! A host with a scripted Docker, for tests of everything that watches containers.

use futures::future::BoxFuture;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use futures::FutureExt as _;
use nocterm_session::{ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};

/// One running container named `web`.
pub const RUNNING: &str =
    r#"{"ID":"a1","Image":"nginx","Names":"web","State":"running","Status":"Up 2 hours"}"#;
/// The same container, exited.
pub const EXITED: &str =
    r#"{"ID":"a1","Image":"nginx","Names":"web","State":"exited","Status":"Exited (0)"}"#;
/// One image.
pub const IMAGE: &str = r#"{"ID":"sha256:1","Repository":"nginx","Tag":"1","Size":"7MB"}"#;

/// A host whose `docker` lists what it is told to, keeps `docker events` running until the
/// test prints to it or lets it end, and does every other command it is asked. Without
/// Docker, every command is missing.
pub struct FakeDockerHost {
    docker: bool,
    containers: Mutex<String>,
    images: Mutex<String>,
    ran: Mutex<Vec<String>>,
    events: Mutex<Vec<ExecSink>>,
    listings: Mutex<VecDeque<BoxFuture<'static, ()>>>,
}

impl FakeDockerHost {
    pub fn new(docker: bool) -> Arc<Self> {
        Arc::new(Self {
            docker,
            containers: Mutex::new(RUNNING.into()),
            images: Mutex::new(String::new()),
            ran: Mutex::default(),
            events: Mutex::default(),
            listings: Mutex::default(),
        })
    }

    /// Suspends the next container listing after capturing its reply.
    pub fn suspend_listing(&self, gate: BoxFuture<'static, ()>) {
        lock(&self.listings).push_back(gate);
    }

    /// Every command line run so far.
    pub fn ran(&self) -> Vec<String> {
        lock(&self.ran).clone()
    }

    /// How many command lines run so far start with `prefix`.
    pub fn count(&self, prefix: &str) -> usize {
        lock(&self.ran)
            .iter()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    /// What `docker ps` prints from now on.
    pub fn set_containers(&self, listing: &str) {
        *lock(&self.containers) = listing.into();
    }

    /// What `docker images` prints from now on.
    pub fn set_images(&self, listing: &str) {
        *lock(&self.images) = listing.into();
    }

    /// Prints `line` and a newline to the latest event stream.
    pub fn announce(&self, line: &str) {
        let events = lock(&self.events);
        let sink = events.last().expect("events are followed");
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        assert_eq!(sink.send(bytes).now_or_never(), Some(true));
    }

    /// Ends the latest event stream, as an engine whose events are unavailable does.
    pub fn end_events(&self) {
        let sink = lock(&self.events).pop().expect("events are followed");
        sink.finish(ExecExit::default());
    }

    /// Whether the event stream started `index`th is still being read.
    pub fn following(&self, index: usize) -> bool {
        !lock(&self.events)[index].is_closed()
    }

    /// How many event streams are still being read.
    pub fn followers(&self) -> usize {
        lock(&self.events)
            .iter()
            .filter(|sink| !sink.is_closed())
            .count()
    }
}

/// A Docker event line of `kind` (`container` or `image`) for `action` on `id`.
pub fn docker_event(kind: &str, action: &str, id: &str) -> String {
    format!(r#"{{"Type":"{kind}","Action":"{action}","Actor":{{"ID":"{id}"}}}}"#)
}

impl HostExec for FakeDockerHost {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let line = request.command_line();
        lock(&self.ran).push(line.clone());
        let (sink, output) = ExecOutput::channel();
        if self.docker && line.starts_with("docker events") {
            lock(&self.events).push(sink);
            return Box::pin(async { Ok(output) });
        }
        let (stdout, status) = match () {
            _ if !self.docker => (String::new(), 127),
            _ if line.starts_with("docker ps") => (lock(&self.containers).clone(), 0),
            _ if line.starts_with("docker images") => (lock(&self.images).clone(), 0),
            _ if line.starts_with("docker") => (String::new(), 0),
            _ => (String::new(), 127),
        };
        let gate = line
            .starts_with("docker ps")
            .then(|| lock(&self.listings).pop_front())
            .flatten();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.await;
            }
            assert!(sink.send(stdout.into_bytes()).await);
            sink.finish(ExecExit {
                status: Some(status),
                stderr: String::new(),
            });
            Ok(output)
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
