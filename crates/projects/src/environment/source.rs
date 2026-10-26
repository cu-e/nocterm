//! Exact saved target and original terminal receipt authorize environment work.
use crate::{Installed, ProjectView};
use gpui_kit::{App, Window};
use nocterm_project_model::ProjectDefinition;
use nocterm_project_providers::{EnvironmentError, EnvironmentSource};
use nocterm_project_service::ProjectBinding;
use nocterm_workspace::Workspace;
use nocterm_workspace_ports::{ExpectedSession, TerminalCapabilities, TerminalStatus};

pub(super) struct CapturedEnvironment {
    source: EnvironmentSource,
    binding: ProjectBinding,
    receipt: Option<(TerminalCapabilities, ExpectedSession)>,
}
impl CapturedEnvironment {
    pub(super) fn capture(
        definition: &ProjectDefinition,
        installed: &Installed,
        workspace: &Workspace,
        window: &Window,
        cx: &App,
    ) -> Result<Self, EnvironmentError> {
        let target = definition.target();
        let host = crate::location::host(target).ok_or(EnvironmentError::Unavailable)?;
        let binding = installed
            .model()
            .binding(definition.id(), cx)
            .map_err(|_| EnvironmentError::StaleSource)?;
        let original = crate::context::capture_host(workspace, installed, &host, window, cx).ok();
        let host_exec = original.as_ref().and_then(|source| source.executor());
        for item in workspace.items(cx) {
            let Ok(project) = item.view().downcast::<ProjectView>() else {
                continue;
            };
            let project = project.read(cx);
            if project.project() != definition.id() || !project.has_target(target) {
                continue;
            }
            let Ok(terminals) = workspace.context_terminals(project.surface_source(cx), cx) else {
                continue;
            };
            for terminal in terminals {
                let caps = terminal.capabilities;
                let Some(info) = caps.state().info(cx) else {
                    continue;
                };
                if info.status != TerminalStatus::Connected {
                    continue;
                }
                let Some(exec) = caps
                    .execution()
                    .and_then(|port| port.executor(info.expected, cx).ok())
                else {
                    continue;
                };
                return Ok(Self {
                    source: EnvironmentSource::scoped(target.clone(), exec, host_exec.clone())?,
                    binding,
                    receipt: Some((caps, info.expected)),
                });
            }
        }
        let source = original.ok_or_else(|| {
            if matches!(host, nocterm_project_model::HostLocator::Local) {
                EnvironmentError::Unavailable
            } else {
                EnvironmentError::Disconnected
            }
        })?;
        let exec = source.executor().ok_or(EnvironmentError::Unavailable)?;
        let receipt = source.expected().and_then(|expected| {
            workspace
                .context_terminals(workspace.general_surface_source(cx), cx)
                .ok()?
                .into_iter()
                .find_map(|entry| {
                    let info = entry.capabilities.state().info(cx)?;
                    (info.expected == expected).then_some((entry.capabilities, expected))
                })
        });
        Ok(Self {
            source: EnvironmentSource::host(target.clone(), exec)?,
            binding,
            receipt,
        })
    }
    pub(super) fn validate(&self, installed: &Installed, cx: &App) -> Result<(), EnvironmentError> {
        installed
            .model()
            .check_binding(self.binding, cx)
            .map_err(|_| EnvironmentError::StaleSource)?;
        if let Some((caps, expected)) = &self.receipt {
            let info = caps.state().info(cx).ok_or(EnvironmentError::StaleSource)?;
            if info.expected != *expected || info.status != TerminalStatus::Connected {
                return Err(EnvironmentError::StaleSource);
            }
            caps.execution()
                .ok_or(EnvironmentError::Unavailable)?
                .executor(*expected, cx)
                .map_err(|_| EnvironmentError::StaleSource)?;
        }
        Ok(())
    }
    pub(super) fn source(&self) -> EnvironmentSource {
        self.source.clone()
    }
}
