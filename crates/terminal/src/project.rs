//! Bound workspace sessions keep ordinary authentication and reconnect.
use crate::{Installed, Terminal, TerminalView};
use gpui_kit::{App, AppContext as _, Window};
use nocterm_project_model::{ProjectTarget, RootPath};
use nocterm_project_providers::ContainerLocation;
use nocterm_workspace::{
    ContextSelectionError, ExpectedSession, ItemHandle, ProjectConnectionError,
    ProjectTerminalOpen, ProjectTerminalSession, ProjectTerminalTarget, SessionSpec,
    TerminalStatus,
};
use std::rc::Rc;

pub fn project_terminal_opener(installed: &Installed) -> Rc<dyn ProjectTerminalOpen> {
    Rc::new(ProjectTerminals {
        installed: installed.clone(),
    })
}
/// Bind a checked resource before any surface opening can be requested.
pub fn project_session(
    installed: &Installed,
    spec: SessionSpec,
    local: bool,
    location: &ProjectTarget,
) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError> {
    location.validate()?;
    let container = ContainerLocation::from_target(location).ok();
    let host = location
        .host_folder()
        .map(|(host, _)| host)
        .or_else(|| container.as_ref().map(|location| location.host()))
        .ok_or(ProjectConnectionError::ProviderUnavailable)?;
    let matches = match host {
        nocterm_project_model::HostLocator::Local => local,
        _ => !local && host.address().as_ref() == Some(&spec.address()),
    };
    if !matches {
        return Err(ProjectConnectionError::ChangedTargetOrRoute);
    }
    let (installed, local, root) = match location {
        ProjectTarget::HostFolder { root, .. } => {
            (installed.with_workspace_sessions(), local, root.clone())
        }
        ProjectTarget::Provider { .. } => {
            let location = ContainerLocation::from_target(location)
                .map_err(|_| ProjectConnectionError::ProviderUnavailable)?;
            let root = location.root().clone();
            let transport = installed
                .container_transport(local, location)
                .ok_or(ProjectConnectionError::ProviderUnavailable)?;
            (installed.with_remote(transport), false, root)
        }
    };
    Ok(Rc::new(FolderSession {
        installed,
        spec,
        local,
        root,
    }))
}
pub fn local_project_session(
    installed: &Installed,
    location: &ProjectTarget,
) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError> {
    project_session(installed, crate::terminal::local_spec(), true, location)
}
struct ProjectTerminals {
    installed: Installed,
}
struct FolderSession {
    installed: Installed,
    spec: SessionSpec,
    local: bool,
    root: RootPath,
}
impl ProjectTerminalOpen for ProjectTerminals {
    fn capture(
        &self,
        parent: Rc<dyn ItemHandle>,
        expected: ExpectedSession,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ContextSelectionError> {
        let info = parent
            .terminal_capabilities(cx)
            .and_then(|ports| ports.state().info(cx))
            .ok_or(ContextSelectionError::ParentUnavailable)?;
        if info.expected != expected || parent.item_id() != expected.item() {
            return Err(nocterm_workspace::ProjectParentError::StaleSession.into());
        }
        if info.status != TerminalStatus::Connected {
            return Err(ContextSelectionError::ParentUnavailable);
        }
        let view = parent
            .view()
            .downcast::<TerminalView>()
            .map_err(|_| ContextSelectionError::ParentUnavailable)?;
        let terminal = view.read(cx).terminal().read(cx);
        if terminal.is_command() || terminal.installed().is_container() {
            return Err(ContextSelectionError::ParentUnavailable);
        }
        project_session(
            &self.installed,
            terminal.spec().clone(),
            terminal.is_local(),
            location,
        )
        .map_err(|_| ContextSelectionError::Unavailable)
    }
}
impl ProjectTerminalSession for FolderSession {
    fn open(
        &self,
        target: ProjectTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ContextSelectionError> {
        target.validate(window, cx)?;
        let mut spec = self.spec.clone();
        let mut launch = spec.launch.clone().unwrap_or_else(|| {
            let defaults = if self.local {
                self.installed
                    .settings()
                    .setting::<nocterm_session::LocalShellSettings>(cx)
            } else {
                &self
                    .installed
                    .settings()
                    .setting::<nocterm_session::SshSettings>(cx)
                    .launch
            };
            crate::terminal::shell_launch(defaults)
        });
        launch.cwd = Some(self.root.as_str().to_owned());
        spec.launch = Some(launch);
        spec.title = "Terminal".into();
        let view = cx.new(|cx| {
            let terminal = cx.new(|cx| {
                Terminal::new_kind(self.installed.clone(), spec, self.local, None, None, cx)
            });
            TerminalView::with_terminal(terminal, window, cx)
        });
        match target.destination() {
            Some(destination) => target.surface().add_local_terminal(
                target.selection(),
                view,
                destination,
                window,
                cx,
            )?,
            None => target
                .surface()
                .add_item(target.selection(), view, window, cx)?,
        };
        Ok(())
    }
}
