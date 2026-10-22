//! Folder terminals use the same launch, authentication and reconnect lifecycle.
use crate::{Installed, Terminal, TerminalView};
use gpui_kit::{App, AppContext as _, Window};
use nocterm_project_model::RootPath;
use nocterm_workspace::{
    ContextSelectionError, ExpectedSession, ItemHandle, ProjectTerminalOpen,
    ProjectTerminalSession, ProjectTerminalTarget, SessionSpec, TerminalStatus,
};
use std::rc::Rc;

pub fn project_terminal_opener(installed: &Installed) -> Rc<dyn ProjectTerminalOpen> {
    Rc::new(ProjectTerminals {
        installed: installed.clone(),
    })
}
/// Capture ordinary connection settings for terminals opened inside a workspace.
pub fn project_session(
    installed: &Installed,
    spec: SessionSpec,
    local: bool,
) -> Rc<dyn ProjectTerminalSession> {
    Rc::new(FolderSession {
        installed: installed.clone(),
        spec,
        local,
    })
}

pub fn local_project_session(installed: &Installed) -> Rc<dyn ProjectTerminalSession> {
    project_session(installed, crate::terminal::local_spec(), true)
}

struct ProjectTerminals {
    installed: Installed,
}
struct FolderSession {
    installed: Installed,
    spec: SessionSpec,
    local: bool,
}
impl ProjectTerminalOpen for ProjectTerminals {
    fn capture(
        &self,
        parent: Rc<dyn ItemHandle>,
        expected: ExpectedSession,
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
        if terminal.is_command() {
            return Err(ContextSelectionError::ParentUnavailable);
        }
        Ok(project_session(
            &self.installed,
            terminal.spec().clone(),
            terminal.is_local(),
        ))
    }
}
impl ProjectTerminalSession for FolderSession {
    fn open(
        &self,
        target: ProjectTerminalTarget,
        root: &RootPath,
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
        launch.cwd = Some(root.as_str().to_owned());
        spec.launch = Some(launch);
        spec.title = "Terminal".into();
        let view = cx.new(|cx| {
            let terminal = cx.new(|cx| {
                Terminal::new_kind(self.installed.clone(), spec, self.local, None, None, cx)
            });
            TerminalView::with_terminal(terminal, window, cx)
        });
        match target.destination() {
            Some(destination) => {
                target.surface().add_local_terminal(
                    target.selection(),
                    view,
                    destination,
                    window,
                    cx,
                )?;
            }
            None => {
                target
                    .surface()
                    .add_item(target.selection(), view, window, cx)?;
            }
        }
        Ok(())
    }
}
