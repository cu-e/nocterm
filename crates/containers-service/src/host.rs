//! What one host connection's engine last reported, and the watch that keeps it current.

use std::sync::Arc;

use futures::channel::mpsc;
use gpui::{Context, Task};
use nocterm_containers::{Changes, Container, ContainersError, Engine, Image, Snapshot};
use nocterm_session::HostExec;

use crate::watch;

/// Where the listing of a host stands.
#[derive(Debug)]
pub enum ListingStatus {
    /// The first listing, or one after a failure, is under way.
    Loading,
    Ready,
    /// The last attempt failed; unless the host has no engine, another follows shortly.
    Failed(ContainersError),
}

impl ListingStatus {
    /// Whether no watch follows: the host has no engine, or cannot run programs.
    fn is_final(&self) -> bool {
        matches!(
            self,
            Self::Failed(ContainersError::NotInstalled | ContainersError::Unsupported)
        )
    }
}

/// A listing that replaces one or both lists of the snapshot.
pub(crate) enum Listed {
    Both(Snapshot),
    Containers(Vec<Container>),
    Images(Vec<Image>),
}

/// The containers and images of one host connection, shared by everyone watching it.
pub struct HostContainers {
    exec: Arc<dyn HostExec>,
    engine: Option<Engine>,
    snapshot: Snapshot,
    status: ListingStatus,
    /// What the running watch is asked to list again; closed once it has ended.
    requests: mpsc::UnboundedSender<Changes>,
    _watch: Task<()>,
}

impl HostContainers {
    pub(crate) fn new(exec: Arc<dyn HostExec>, cx: &mut Context<Self>) -> Self {
        let (requests, watch) = Self::spawn(exec.clone(), None, cx);
        Self {
            exec,
            engine: None,
            snapshot: Snapshot::default(),
            status: ListingStatus::Loading,
            requests,
            _watch: watch,
        }
    }

    fn spawn(
        exec: Arc<dyn HostExec>,
        engine: Option<Engine>,
        cx: &mut Context<Self>,
    ) -> (mpsc::UnboundedSender<Changes>, Task<()>) {
        let (requests, asked) = mpsc::unbounded();
        let task =
            cx.spawn(async move |this, cx| watch::watch(this, exec, engine, asked, cx).await);
        (requests, task)
    }

    /// How programs run on this host.
    pub fn exec(&self) -> &Arc<dyn HostExec> {
        &self.exec
    }

    /// The engine the host was found to have.
    pub fn engine(&self) -> Option<Engine> {
        self.engine
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn status(&self) -> &ListingStatus {
        &self.status
    }

    /// Lists `changes` again soon, as after an action on the host or when the person asks;
    /// a watch that has stopped starts again, looking for the engine anew.
    pub fn relist(&mut self, changes: Changes, cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        if self.requests.unbounded_send(changes).is_ok() {
            return;
        }
        self.restart(cx);
    }

    /// Starts again at once a watch that is waiting out a failure, or has stopped.
    pub(crate) fn revive(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, ListingStatus::Failed(_)) {
            self.relist(Changes::ALL, cx);
        }
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        let engine = if self.status.is_final() {
            None
        } else {
            self.engine
        };
        let (requests, watch) = Self::spawn(self.exec.clone(), engine, cx);
        self.requests = requests;
        self._watch = watch;
        self.set_status(ListingStatus::Loading, cx);
    }

    pub(crate) fn listed(&mut self, engine: Engine, listed: Listed, cx: &mut Context<Self>) {
        let changed = self.engine != Some(engine);
        self.engine = Some(engine);
        let changed = changed
            | match listed {
                Listed::Both(snapshot) => replace(&mut self.snapshot, snapshot),
                Listed::Containers(containers) => {
                    replace(&mut self.snapshot.containers, containers)
                }
                Listed::Images(images) => replace(&mut self.snapshot.images, images),
            };
        if changed {
            cx.notify();
        }
        self.set_status(ListingStatus::Ready, cx);
    }

    /// The watch's attempt ended with `outcome`; whether to try again.
    pub(crate) fn ended(
        &mut self,
        outcome: Result<(), ContainersError>,
        cx: &mut Context<Self>,
    ) -> bool {
        // An ended event stream leaves the listing standing.
        let Err(error) = outcome else { return true };
        self.set_status(ListingStatus::Failed(error), cx);
        !self.status.is_final()
    }

    fn set_status(&mut self, status: ListingStatus, cx: &mut Context<Self>) {
        // Native error sources have no equality contract. Deduplicate only the
        // visible status category and text; recovery always matches typed variants.
        let unchanged = match (&self.status, &status) {
            (ListingStatus::Failed(previous), ListingStatus::Failed(next)) => {
                std::mem::discriminant(previous) == std::mem::discriminant(next)
                    && previous.to_string() == next.to_string()
            }
            (previous, next) => std::mem::discriminant(previous) == std::mem::discriminant(next),
        };
        if !unchanged {
            self.status = status;
            cx.notify();
        }
    }
}

fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}
