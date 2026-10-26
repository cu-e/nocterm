//! Selected resource observation for monitoring; never an implicit host fallback.
use crate::SessionContext;
use gpui::EntityId;
use nocterm_project_model::{HostLocator, ProjectTarget};

/// Immutable selected surface facts. Executors remain those of the live inner session.
#[derive(Clone)]
pub struct MonitorSource {
    surface: EntityId,
    target: Option<ProjectTarget>,
    session: Option<SessionContext>,
    local: bool,
    available: bool,
}
impl MonitorSource {
    pub fn unavailable(surface: EntityId) -> Self {
        Self {
            surface,
            target: None,
            session: None,
            local: false,
            available: false,
        }
    }
    pub fn general(surface: EntityId, session: Option<SessionContext>, local: bool) -> Self {
        Self {
            surface,
            target: None,
            session: if local { None } else { session },
            local,
            available: true,
        }
    }
    pub fn project(
        surface: EntityId,
        target: Option<ProjectTarget>,
        session: Option<SessionContext>,
        local: bool,
    ) -> Self {
        let local = local
            && matches!(
                target,
                Some(ProjectTarget::HostFolder {
                    host: HostLocator::Local,
                    ..
                })
            );
        let available = target.is_some();
        Self {
            surface,
            target,
            session: if local { None } else { session },
            local,
            available,
        }
    }
    pub fn surface(&self) -> EntityId {
        self.surface
    }
    pub fn target(&self) -> Option<&ProjectTarget> {
        self.target.as_ref()
    }
    pub fn session(&self) -> Option<&SessionContext> {
        self.session.as_ref()
    }
    pub fn local(&self) -> bool {
        self.local
    }
    pub fn available(&self) -> bool {
        self.available
    }
}
