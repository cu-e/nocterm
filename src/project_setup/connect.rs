//! Saved connection metadata becomes an ordinary inner terminal configuration.
use gpui_kit::{App, AppContext as _, EntityId, WeakEntity, Window};
use nocterm_project_model::HostLocator;
use nocterm_workspace::{
    ProjectConnectionError, ProjectTerminalResolve, ProjectTerminalSession, SessionSpec,
};
use std::rc::Rc;

pub(super) struct TerminalResolver {
    terminal: nocterm_terminal::Installed,
    connections: Option<nocterm_connections::Installed>,
}
impl TerminalResolver {
    pub(super) fn new(
        terminal: nocterm_terminal::Installed,
        connections: Option<nocterm_connections::Installed>,
    ) -> Self {
        Self {
            terminal,
            connections,
        }
    }
}
impl ProjectTerminalResolve for TerminalResolver {
    fn resolve(
        &self,
        host: &HostLocator,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError> {
        host.validate()?;
        let spec = match host {
            HostLocator::Local => {
                return Ok(nocterm_terminal::local_project_session(&self.terminal));
            }
            HostLocator::SshDirect { target, via } => SessionSpec {
                options: Default::default(),
                title: target.to_string().into(),
                profile: None,
                target: target.clone(),
                auth: nocterm_session::Auth::Auto,
                launch: None,
                credential: None,
                jumps: via
                    .iter()
                    .cloned()
                    .map(|target| nocterm_session::ResolvedJump {
                        host: nocterm_session::JumpHost {
                            target,
                            auth: nocterm_session::Auth::Auto,
                        },
                        credential: None,
                    })
                    .collect(),
            },
            HostLocator::Ssh { .. } => {
                let connections = self
                    .connections
                    .as_ref()
                    .ok_or(ProjectConnectionError::Unavailable)?;
                nocterm_connections::resolve_parent_spec(
                    host,
                    connections.connections().read(cx).profiles(cx),
                )?
            }
        };
        Ok(nocterm_terminal::project_session(
            &self.terminal,
            spec,
            false,
        ))
    }
}

impl nocterm_workspace::ChatConnectionOpen for TerminalResolver {
    fn open(
        &self,
        profile: &str,
        source: nocterm_workspace::ProjectSurfaceReceipt,
        workspace: &WeakEntity<nocterm_workspace::Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<EntityId, nocterm_workspace::ChatConnectionOpenError> {
        use nocterm_workspace::{ChatConnectionOpenError, ContextSelectionError};
        let workspace = workspace
            .upgrade()
            .ok_or(ContextSelectionError::StaleWindow)?;
        workspace
            .read(cx)
            .validate_context_source(source, window, cx)?;
        let connections = self
            .connections
            .as_ref()
            .ok_or(ChatConnectionOpenError::Unavailable)?;
        let profiles = connections.connections().read(cx).profiles(cx);
        let profile = profiles
            .iter()
            .find(|saved| saved.id.to_string() == profile)
            .ok_or(ChatConnectionOpenError::MissingProfile)?;
        let spec = nocterm_connections::spec_for_profile(profile, profiles).map_err(|error| {
            ChatConnectionOpenError::InvalidRoute(nocterm_core::SharedCause::new(error))
        })?;
        let view = cx
            .new(|cx| nocterm_terminal::TerminalView::new(self.terminal.clone(), spec, window, cx));
        workspace
            .update(cx, |workspace, cx| {
                workspace.add_context_background_from(source, view, window, cx)
            })
            .map_err(Into::into)
    }
}
