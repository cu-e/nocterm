//! The machine the user is looking at, for features that run programs on it.
//!
//! The active tab decides. A remote session that can run programs is reached
//! over its own connection; anything else is this computer, through the
//! runner the application installs with [`set_local_exec`]. Features that
//! follow the active host — the resource monitor, the containers panel —
//! resolve it the same way with [`Host::resolve`] and hear of changes
//! through [`follow_active_session`], so switching tabs means the same thing
//! to all of them and none opens a connection of its own.

use std::sync::Arc;

use gpui_kit::{App, Context, Entity, Global, SharedString, Subscription};
use nocterm_session::{ExecRequest, HostExec, Target};

use crate::{SessionContext, Workspace, WorkspaceEvent};

/// Which machine something is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKey {
    /// The computer nocterm runs on.
    Local,
    Remote(Target),
}

impl HostKey {
    /// The machine `session` runs programs on: its own when it can, this
    /// computer otherwise.
    pub fn of(session: &SessionContext) -> Self {
        match session.exec {
            Some(_) => Self::Remote(session.target.clone()),
            None => Self::Local,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Local => "This computer".into(),
            Self::Remote(target) => target.to_string(),
        }
    }
}

/// A machine and how to run programs on it.
#[derive(Clone)]
pub struct Host {
    pub key: HostKey,
    pub exec: Arc<dyn HostExec>,
    /// Whether its connection is up; this computer always is.
    pub connected: bool,
    /// The session it was reached through; `None` for this computer.
    pub session: Option<SessionContext>,
}

impl Host {
    /// The host behind `session`: its own connection when it can run
    /// programs, otherwise this computer, when `local` is given.
    pub fn resolve(
        session: Option<&SessionContext>,
        local: Option<&Arc<dyn HostExec>>,
    ) -> Option<Self> {
        if let Some((session, exec)) =
            session.and_then(|session| Some((session, session.exec.clone()?)))
        {
            return Some(Self {
                key: HostKey::of(session),
                exec,
                connected: session.connected,
                session: Some(session.clone()),
            });
        }
        Some(Self {
            key: HostKey::Local,
            exec: local?.clone(),
            connected: true,
            session: None,
        })
    }

    /// Whether two hosts are the same machine, reached the same way, in the
    /// same state.
    pub fn same_as(&self, other: &Self) -> bool {
        self.key == other.key
            && self.connected == other.connected
            && Arc::ptr_eq(&self.exec, &other.exec)
    }
}

/// A program shown in a tab of its own, such as a container's log or a shell
/// inside one. It runs over the host's existing connection.
#[derive(Clone)]
pub struct ProgramSpec {
    pub title: SharedString,
    pub host: Host,
    pub program: ExecRequest,
}

struct LocalExec(Arc<dyn HostExec>);

impl Global for LocalExec {}

/// Installs the program runner for this computer.
pub fn set_local_exec(exec: Arc<dyn HostExec>, cx: &mut App) {
    cx.set_global(LocalExec(exec));
}

/// The program runner for this computer, when the application installed one.
pub fn local_exec(cx: &App) -> Option<Arc<dyn HostExec>> {
    cx.try_global::<LocalExec>().map(|local| local.0.clone())
}

/// Calls `follow` with the active session of `workspace` each time it
/// becomes a different one or changes state.
pub fn follow_active_session<T: 'static>(
    workspace: &Entity<Workspace>,
    cx: &mut Context<T>,
    follow: impl Fn(&mut T, Option<SessionContext>, &mut Context<T>) + 'static,
) -> Subscription {
    let follow = std::rc::Rc::new(follow);
    cx.subscribe(workspace, move |_, workspace, event, cx| {
        if *event == WorkspaceEvent::ActiveSessionChanged {
            let workspace = workspace.downgrade();
            let this = cx.entity().downgrade();
            let follow = follow.clone();
            // The workspace may still be mid-update when it announces.
            cx.defer(move |cx| {
                let Some(workspace) = workspace.upgrade() else {
                    return;
                };
                let session = workspace.read(cx).active_session(cx);
                let _ = this.update(cx, |this, cx| follow(this, session, cx));
            });
        }
    })
}
