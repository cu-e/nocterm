//! The containers of the active host, kept current.
//!
//! The model follows the workspace's active host ([`nocterm_workspace::host`]),
//! like the resource monitor, over the connection that host already has. It
//! keeps watching while the panel is hidden, so the footer's count stays
//! right. The engine's event stream says when to list again; where there is
//! none, or it ends, the listing is repeated every [`RETRY`].

use std::{sync::Arc, time::Duration};

use futures::FutureExt as _;
use gpui_kit::{AsyncApp, Context, Entity, Subscription, Task, WeakEntity};
use nocterm_containers::{Action, ContainersError, Engine, Snapshot, detect};
use nocterm_session::{ExecRequest, HostExec};
use nocterm_workspace::{
    Host, ProgramSpec, SessionContext, Workspace, host::follow_active_session,
};

/// How long a failed listing, or an ended event stream, waits before the
/// host is listed again.
const RETRY: Duration = Duration::from_secs(10);
/// How long a burst of events (a Compose project starting) may take before
/// the host is listed once for all of it.
const SETTLE: Duration = Duration::from_millis(300);

/// Where the listing of the current host stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// There is no host to list.
    Off,
    /// The host's session is not connected.
    Offline,
    Loading,
    Ready,
    /// The last attempt failed; unless the host has no engine, another
    /// follows shortly.
    Failed(ContainersError),
}

pub(crate) struct ContainersModel {
    local: Option<Arc<dyn HostExec>>,
    host: Option<Host>,
    /// The engine the host was found to have.
    engine: Option<Engine>,
    snapshot: Snapshot,
    status: Status,
    task: Option<Task<()>>,
    _follow: Subscription,
}

impl ContainersModel {
    pub(crate) fn new(
        workspace: &Entity<Workspace>,
        session: Option<SessionContext>,
        local: Option<Arc<dyn HostExec>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            local,
            host: None,
            engine: None,
            snapshot: Snapshot::default(),
            status: Status::Off,
            task: None,
            _follow: follow_active_session(workspace, cx, Self::follow),
        };
        this.follow(session, cx);
        this
    }

    pub(crate) fn host(&self) -> Option<&Host> {
        self.host.as_ref()
    }

    pub(crate) fn engine(&self) -> Option<Engine> {
        self.engine
    }

    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    fn follow(&mut self, session: Option<SessionContext>, cx: &mut Context<Self>) {
        let host = Host::resolve(session.as_ref(), self.local.as_ref());
        let unchanged = match (&self.host, &host) {
            (Some(was), Some(host)) => was.same_as(host),
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        let key = |host: &Option<Host>| host.as_ref().map(|host| host.key.clone());
        if key(&self.host) != key(&host) {
            self.engine = None;
            self.snapshot = Snapshot::default();
        }
        self.host = host;
        self.refresh(cx);
    }

    /// Lists the host again, and follows it from there.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        let Some(host) = self.host.clone() else {
            self.set_status(Status::Off, cx);
            return;
        };
        if !host.connected {
            self.set_status(Status::Offline, cx);
            return;
        }
        self.set_status(Status::Loading, cx);
        let engine = self.engine;
        self.task = Some(cx.spawn(async move |this, cx| watch(this, host, engine, cx).await));
    }

    /// Does `action` to `ids` on the current host, then lists it again.
    pub(crate) fn perform(
        &mut self,
        action: Action,
        ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ContainersError>> {
        let (Some(host), Some(engine)) = (self.host.clone(), self.engine) else {
            return Task::ready(Err(ContainersError::Disconnected));
        };
        cx.spawn(async move |this, cx| {
            let done = engine.perform(host.exec.as_ref(), action, &ids).await;
            // Not every engine reports the change itself.
            let _ = this.update(cx, |this, cx| this.refresh(cx));
            done
        })
    }

    /// A program on the current host for a tab of its own, made by the
    /// host's engine.
    pub(crate) fn program(
        &self,
        title: String,
        program: impl FnOnce(Engine) -> ExecRequest,
    ) -> Option<ProgramSpec> {
        Some(ProgramSpec {
            title: title.into(),
            host: self.host.clone()?,
            program: program(self.engine?),
            shell_syntax: None,
        })
    }

    fn apply(&mut self, engine: Engine, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.engine = Some(engine);
        if self.snapshot != snapshot {
            self.snapshot = snapshot;
            cx.notify();
        }
        self.set_status(Status::Ready, cx);
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }
}

/// Lists the host and follows its changes, again after failures, until the
/// host has no engine or the model moves on.
async fn watch(
    this: WeakEntity<ContainersModel>,
    host: Host,
    mut engine: Option<Engine>,
    cx: &mut AsyncApp,
) {
    loop {
        let outcome = follow_engine(&this, &host, &mut engine, cx).await;
        let retry = this.update(cx, |this, cx| match outcome {
            // The event stream ended; the listing still stands.
            Ok(()) => true,
            Err(error) => {
                let retry = !matches!(
                    error,
                    ContainersError::NotInstalled | ContainersError::Unsupported
                );
                this.set_status(Status::Failed(error), cx);
                retry
            }
        });
        if !matches!(retry, Ok(true)) {
            return;
        }
        cx.background_executor().timer(RETRY).await;
    }
}

/// One attempt: a listing, then another after each change the engine
/// reports, until its event stream ends.
async fn follow_engine(
    this: &WeakEntity<ContainersModel>,
    host: &Host,
    engine: &mut Option<Engine>,
    cx: &mut AsyncApp,
) -> Result<(), ContainersError> {
    let exec = host.exec.as_ref();
    let (found, snapshot) = match *engine {
        Some(known) => (known, known.snapshot(exec).await?),
        None => detect(exec).await?,
    };
    *engine = Some(found);
    let apply = |snapshot, cx: &mut AsyncApp| {
        this.update(cx, |this, cx| this.apply(found, snapshot, cx))
            .map_err(|_| ContainersError::Disconnected)
    };
    apply(snapshot, cx)?;
    let mut events = found.events(exec).await?;
    while events.next().await.is_some() {
        cx.background_executor().timer(SETTLE).await;
        while let Some(Some(_)) = events.next().now_or_never() {}
        apply(found.snapshot(exec).await?, cx)?;
    }
    Ok(())
}
