//! Resource-specific monitor identity without changing shared host resolution.
use crate::MonitorSettings;
use gpui_kit::EntityId;
use nocterm_project_model::ProjectTarget;
use nocterm_project_providers::ContainerLocation;
use nocterm_session::HostExec;
use nocterm_workspace::{Host, HostKey, MonitorSource, SessionContext};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MonitorKey {
    surface: EntityId,
    target: Option<ProjectTarget>,
    host: HostKey,
}
#[derive(Clone)]
pub(super) struct WatchHost {
    key: MonitorKey,
    host: Host,
    container: bool,
}
impl WatchHost {
    pub(super) fn key(&self) -> &MonitorKey {
        &self.key
    }
    pub(super) fn host(&self) -> &Host {
        &self.host
    }
    pub(super) fn container(&self) -> bool {
        self.container
    }
    pub(super) fn same_as(&self, other: &Self) -> bool {
        self.key == other.key && self.host.same_as(&other.host)
    }
    pub(super) fn label(&self) -> String {
        match self
            .key
            .target
            .as_ref()
            .and_then(|target| ContainerLocation::from_target(target).ok())
        {
            Some(location) => format!(
                "{} {} — {}",
                location.engine().name(),
                &location.container()[..12],
                self.host.key.label()
            ),
            None => self.host.key.label(),
        }
    }
}
pub(super) fn watched_source(
    source: &MonitorSource,
    local: Option<&Arc<dyn HostExec>>,
    settings: &MonitorSettings,
) -> Option<WatchHost> {
    if !source.available() {
        return None;
    }
    let container = match source.target() {
        Some(target @ ProjectTarget::Provider { .. }) => {
            ContainerLocation::from_target(target).ok()?;
            true
        }
        _ => false,
    };
    // A selected project with no inner session is unavailable, even on a local parent.
    if source.target().is_some() && source.session().is_none() && !source.local() {
        return None;
    }
    let host = if source.local() {
        if container || !settings.local || !settings.enabled {
            return None;
        }
        Host {
            key: HostKey::Local,
            exec: source
                .session()
                .filter(|session| session.connected)
                .and_then(|session| session.exec.clone())
                .or_else(|| local.cloned())?,
            connected: source.session().is_none_or(|session| session.connected),
            session: source.session().cloned(),
        }
    } else {
        watched_host(source.session(), None, settings)?
    };
    Some(WatchHost {
        key: MonitorKey {
            surface: source.surface(),
            target: source.target().cloned(),
            host: host.key.clone(),
        },
        host,
        container,
    })
}
pub(crate) fn watched_host(
    session: Option<&SessionContext>,
    local: Option<&Arc<dyn HostExec>>,
    settings: &MonitorSettings,
) -> Option<Host> {
    if !settings.enabled {
        return None;
    }
    if let Some(session) = session {
        return Some(Host {
            key: HostKey::Remote {
                target: session.target.clone(),
                via: session.via.clone(),
            },
            exec: session.exec.clone()?,
            connected: session.connected,
            session: Some(session.clone()),
        });
    }
    settings.local.then(|| Host::resolve(None, local)).flatten()
}
