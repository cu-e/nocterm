//! Capture only an original unscoped host; inner container executors are never fallback.
use crate::Installed;
use gpui_kit::{App, Window};
use nocterm_project_providers::{ContainerActionError, ContainerActionSource, ContainerLocation};
use nocterm_workspace::Workspace;
use nocterm_workspace_ports::{ExpectedSession, TerminalCapabilities, TerminalStatus};

pub(super) struct CapturedRemoval {
    source: ContainerActionSource,
    receipt: Option<(TerminalCapabilities, ExpectedSession)>,
}
impl CapturedRemoval {
    pub(super) fn capture(
        location: ContainerLocation,
        installed: &Installed,
        workspace: &Workspace,
        window: &Window,
        cx: &App,
    ) -> Result<Self, ContainerActionError> {
        let original =
            crate::context::capture_host(workspace, installed, location.host(), window, cx)
                .map_err(|_| ContainerActionError::Unavailable)?;
        let receipt = if let Some(expected) = original.expected() {
            let caps = workspace
                .context_terminals(workspace.general_surface_source(cx), cx)
                .map_err(|_| ContainerActionError::Unavailable)?
                .into_iter()
                .find_map(|entry| {
                    let info = entry.capabilities.state().info(cx)?;
                    (info.expected == expected).then_some(entry.capabilities)
                })
                .ok_or(ContainerActionError::StaleSource)?;
            Some((caps, expected))
        } else {
            None
        };
        let source = ContainerActionSource::new(
            location,
            original.parent().host(),
            original
                .executor()
                .ok_or(ContainerActionError::Unavailable)?,
        )?;
        Ok(Self { source, receipt })
    }
    pub(super) fn validate(&self, cx: &App) -> Result<(), ContainerActionError> {
        if let Some((caps, expected)) = &self.receipt {
            let info = caps
                .state()
                .info(cx)
                .ok_or(ContainerActionError::StaleSource)?;
            if info.expected != *expected || info.status != TerminalStatus::Connected {
                return Err(ContainerActionError::StaleSource);
            }
            caps.execution()
                .ok_or(ContainerActionError::Unavailable)?
                .executor(*expected, cx)
                .map_err(|_| ContainerActionError::StaleSource)?;
            let current = caps
                .project_parent()
                .ok_or(ContainerActionError::Unavailable)?
                .capture(*expected, cx)
                .map_err(|_| ContainerActionError::StaleSource)?;
            if current.parent().host() != self.source.location().host() {
                return Err(ContainerActionError::StaleSource);
            }
        }
        Ok(())
    }
    pub(super) fn source(&self) -> ContainerActionSource {
        self.source.clone()
    }
}
