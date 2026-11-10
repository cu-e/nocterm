//! The containers of the active host, kept current.
//!
//! The model follows the workspace's active host ([`nocterm_workspace::host`]),
//! like the resource monitor, over the connection that host already has. Its
//! containers come from the application's [`ContainerHosts`], which watches each
//! host connection once for every window showing it. The model holds that watch
//! while the panel is hidden, so the footer's count stays right.

use std::sync::Arc;

use gpui_kit::{App, Context, Entity, Subscription, Task};
use nocterm_containers::{Action, Changes, ContainersError, Engine, Snapshot};
use nocterm_containers_service::{ContainerHosts, HostContainers, ListingStatus};
use nocterm_session::{ExecRequest, HostExec};
use nocterm_workspace::{
    Host, ProgramSpec, SessionContext, Workspace, host::follow_active_session,
};

/// What a host shows before its engine has answered.
static NOTHING: Snapshot = Snapshot {
    containers: Vec::new(),
    images: Vec::new(),
};

/// Where the listing of the current host stands.
#[derive(Debug)]
pub(crate) enum Status<'a> {
    /// There is no host to list.
    Off,
    /// The host's session is not connected.
    Offline,
    Loading,
    Ready,
    /// The last attempt failed; unless the host has no engine, another
    /// follows shortly.
    Failed(&'a ContainersError),
}

/// A confirmed operation belongs to one observed host connection. Moving
/// away, disconnecting or reconnecting invalidates the intent permanently.
#[derive(Clone)]
pub(crate) struct ActionIntent {
    host: Host,
    engine: Engine,
    epoch: Arc<()>,
    action: Action,
    ids: Vec<String>,
}

impl ActionIntent {
    pub(crate) fn action(&self) -> Action {
        self.action
    }
}

pub(crate) struct ContainersModel {
    epoch: Arc<()>,
    local: Option<Arc<dyn HostExec>>,
    hosts: ContainerHosts,
    host: Option<Host>,
    /// The shared watch of a connected host.
    watched: Option<Watched>,
    _follow: Subscription,
}

/// A host's shared watch, redrawn by this model when it changes.
struct Watched {
    containers: Entity<HostContainers>,
    _observe: Subscription,
}

impl ContainersModel {
    pub(crate) fn new(
        workspace: &Entity<Workspace>,
        session: Option<SessionContext>,
        local: Option<Arc<dyn HostExec>>,
        hosts: ContainerHosts,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            epoch: Arc::new(()),
            local,
            hosts,
            host: None,
            watched: None,
            _follow: follow_active_session(workspace, cx, Self::follow),
        };
        this.follow(session, cx);
        this
    }

    pub(crate) fn host(&self) -> Option<&Host> {
        self.host.as_ref()
    }

    fn containers<'a>(&self, cx: &'a App) -> Option<&'a HostContainers> {
        self.watched
            .as_ref()
            .map(|watched| watched.containers.read(cx))
    }

    pub(crate) fn engine(&self, cx: &App) -> Option<Engine> {
        self.containers(cx)?.engine()
    }

    pub(crate) fn snapshot<'a>(&self, cx: &'a App) -> &'a Snapshot {
        self.containers(cx)
            .map_or(&NOTHING, HostContainers::snapshot)
    }

    pub(crate) fn status<'a>(&self, cx: &'a App) -> Status<'a> {
        let Some(host) = &self.host else {
            return Status::Off;
        };
        if !host.connected {
            return Status::Offline;
        }
        match self.containers(cx).map(HostContainers::status) {
            Some(ListingStatus::Ready) => Status::Ready,
            Some(ListingStatus::Failed(error)) => Status::Failed(error),
            Some(ListingStatus::Loading) | None => Status::Loading,
        }
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
        self.epoch = Arc::new(());
        self.watched = host
            .as_ref()
            .filter(|host| host.connected)
            .map(|host| self.watch(&host.exec, cx));
        self.host = host;
        cx.notify();
    }

    fn watch(&self, exec: &Arc<dyn HostExec>, cx: &mut Context<Self>) -> Watched {
        let containers = self.hosts.watch(exec, cx);
        Watched {
            _observe: cx.observe(&containers, |_, _, cx| cx.notify()),
            containers,
        }
    }

    /// Lists the host again, as the person asked.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.relist(Changes::ALL, cx);
    }

    fn relist(&self, changes: Changes, cx: &mut Context<Self>) {
        if let Some(watched) = &self.watched {
            watched
                .containers
                .update(cx, |containers, cx| containers.relist(changes, cx));
        }
    }

    pub(crate) fn intent(
        &self,
        action: Action,
        ids: Vec<String>,
        cx: &App,
    ) -> Option<ActionIntent> {
        let host = self.host.clone()?;
        if !host.connected {
            return None;
        }
        Some(ActionIntent {
            host,
            engine: self.engine(cx)?,
            epoch: self.epoch.clone(),
            action,
            ids,
        })
    }

    fn accepts(&self, intent: &ActionIntent, cx: &App) -> bool {
        Arc::ptr_eq(&self.epoch, &intent.epoch)
            && self.engine(cx) == Some(intent.engine)
            && self
                .host
                .as_ref()
                .is_some_and(|host| host.connected && host.same_as(&intent.host))
    }

    /// Freezes the current host for an operation that needs no confirmation.
    pub(crate) fn perform(
        &mut self,
        action: Action,
        ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ContainersError>> {
        let Some(intent) = self.intent(action, ids, cx) else {
            return Task::ready(Err(ContainersError::Disconnected));
        };
        self.perform_intent(intent, cx)
    }

    /// Executes a previously captured intent only while its connection is
    /// still current. The executor never comes from the newly active host.
    pub(crate) fn perform_intent(
        &mut self,
        intent: ActionIntent,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ContainersError>> {
        cx.spawn(async move |this, cx| {
            let accepted = this.update(cx, |this, cx| this.accepts(&intent, cx));
            if !matches!(accepted, Ok(true)) {
                return Err(ContainersError::Disconnected);
            }
            let deadline = cx
                .background_executor()
                .timer(std::time::Duration::from_secs(75));
            let action =
                intent
                    .engine
                    .perform(intent.host.exec.as_ref(), intent.action, &intent.ids);
            futures::pin_mut!(deadline, action);
            let done = match futures::future::select(action, deadline).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => Err(ContainersError::TimedOut),
            };
            // Its events say so too, but not every engine reports them. Finishing on A
            // must not relist B.
            let _ = this.update(cx, |this, cx| {
                if this.accepts(&intent, cx) {
                    this.relist(intent.action.changes(), cx);
                }
            });
            done
        })
    }

    /// A program on the current host for a tab of its own, made by the
    /// host's engine.
    pub(crate) fn program(
        &self,
        title: String,
        program: impl FnOnce(Engine) -> ExecRequest,
        cx: &App,
    ) -> Option<ProgramSpec> {
        Some(ProgramSpec {
            title: title.into(),
            host: self.host.clone()?,
            program: program(self.engine(cx)?),
            shell_syntax: None,
        })
    }
}
