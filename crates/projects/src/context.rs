//! Descriptive grouping and original human capture are deliberately separate.
use gpui_kit::{App, Window};
use nocterm_project_model::HostLocator;
use nocterm_workspace::Workspace;
use nocterm_workspace_ports::{
    ProjectCreationSource, ProjectParentDescriptor, ProjectParentError, TerminalCapabilities,
};

pub(crate) fn current(
    workspace: &Workspace,
    installed: &crate::Installed,
    window: &Window,
    cx: &App,
) -> Option<ProjectParentDescriptor> {
    if let Some(project) = workspace
        .active_item(cx)
        .and_then(|item| item.view().downcast::<crate::ProjectView>().ok())
    {
        let parent = project.read(cx).parent()?;
        return server(workspace, parent.host(), Some(parent), cx).ok();
    }
    let Some(item) = workspace.command_item(window, cx) else {
        return installed
            .local_source()
            .ok()
            .map(|source| source.parent().clone());
    };
    let Some(capabilities) = item.terminal_capabilities(cx) else {
        return installed
            .local_source()
            .ok()
            .map(|source| source.parent().clone());
    };
    let parent = capture(&capabilities, cx)
        .map(|source| source.parent().clone())
        .ok()
        .or_else(|| describe(&capabilities, cx))?;
    server(workspace, parent.host(), Some(&parent), cx).ok()
}

pub(crate) fn summary(
    workspace: &Workspace,
    host: &HostLocator,
    cx: &App,
) -> Option<nocterm_workspace::ConnectionSummary> {
    let HostLocator::Ssh {
        profile, target, ..
    } = host
    else {
        return None;
    };
    workspace
        .connection_directory()?
        .connections(cx)
        .into_iter()
        .find(|connection| connection.id.as_ref() == profile && &connection.target == target)
}

pub(crate) fn server(
    workspace: &Workspace,
    host: &HostLocator,
    fallback: Option<&ProjectParentDescriptor>,
    cx: &App,
) -> Result<ProjectParentDescriptor, ProjectParentError> {
    let title = summary(workspace, host, cx)
        .map(|connection| connection.name)
        .or_else(|| {
            fallback
                .filter(|parent| parent.host() == host)
                .map(|parent| parent.title().clone())
        })
        .unwrap_or_else(|| match host {
            HostLocator::Local => "This computer".into(),
            HostLocator::Ssh { target, .. } | HostLocator::SshDirect { target, .. } => {
                target.to_string().into()
            }
        });
    ProjectParentDescriptor::new(host.clone(), title)
}

fn describe(capabilities: &TerminalCapabilities, cx: &App) -> Option<ProjectParentDescriptor> {
    let info = capabilities.state().info(cx)?;
    let host = if info.local {
        HostLocator::Local
    } else {
        let facts = capabilities.fact_session()?.current(cx)?;
        match facts.profile {
            Some(profile) => HostLocator::Ssh {
                profile: profile.to_string(),
                target: facts.session.target,
                via: facts.session.via,
            },
            None => HostLocator::SshDirect {
                target: facts.session.target,
                via: facts.session.via,
            },
        }
    };
    ProjectParentDescriptor::new(host, info.title).ok()
}

pub(crate) fn capture_current(
    workspace: &Workspace,
    installed: &crate::Installed,
    window: &Window,
    cx: &App,
) -> Result<ProjectCreationSource, ProjectParentError> {
    let project_parent = match workspace
        .active_item(cx)
        .and_then(|item| item.view().downcast::<crate::ProjectView>().ok())
    {
        Some(project) => Some(
            project
                .read(cx)
                .parent()
                .cloned()
                .ok_or(ProjectParentError::UnavailableParent)?,
        ),
        None => None,
    };
    if let Some(item) = workspace.command_item(window, cx)
        && let Some(capabilities) = item.terminal_capabilities(cx)
        && capabilities.project_parent().is_some()
    {
        let source = capture(&capabilities, cx)?;
        if let Some(parent) = &project_parent {
            if source.parent().host() != parent.host() {
                return capture_host(workspace, installed, parent.host(), window, cx);
            }
            return Ok(ProjectCreationSource::new(
                server(workspace, parent.host(), Some(parent), cx)?,
                source.expected().ok_or(ProjectParentError::StaleSession)?,
                source.cwd().map(Into::into),
            )?
            .with_resources(source.executor(), source.filesystem()));
        }
        return Ok(source);
    }
    if let Some(parent) = project_parent {
        return capture_host(workspace, installed, parent.host(), window, cx);
    }
    if let Some(capabilities) = workspace
        .command_item(window, cx)
        .and_then(|item| item.terminal_capabilities(cx))
        && !capabilities.state().info(cx).is_some_and(|info| info.local)
    {
        return Err(ProjectParentError::Disconnected);
    }
    installed.local_source()
}

pub(crate) fn capture_host(
    workspace: &Workspace,
    installed: &crate::Installed,
    host: &HostLocator,
    window: &Window,
    cx: &App,
) -> Result<ProjectCreationSource, ProjectParentError> {
    let candidates = workspace
        .context_terminals(workspace.general_surface_source(cx), cx)
        .map_err(|_| ProjectParentError::Disconnected)?;
    let preferred = workspace
        .command_item(window, cx)
        .map(|item| item.item_id());
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| Some(candidate.item) == preferred)
        && let Ok(source) = capture(&candidate.capabilities, cx)
        && source.parent().host() == host
    {
        return Ok(source);
    }
    let mut refusal = None;
    for candidate in candidates {
        match capture(&candidate.capabilities, cx) {
            Ok(source) if source.parent().host() == host => return Ok(source),
            Err(error)
                if describe(&candidate.capabilities, cx)
                    .is_some_and(|parent| parent.host() == host)
                    && (refusal.is_none()
                        || !matches!(error, ProjectParentError::Disconnected)) =>
            {
                refusal = Some(error);
            }
            _ => {}
        }
    }
    if matches!(host, HostLocator::Local) {
        return installed.local_source();
    }
    Err(refusal.unwrap_or(ProjectParentError::Disconnected))
}

fn capture(
    caps: &TerminalCapabilities,
    cx: &App,
) -> Result<ProjectCreationSource, ProjectParentError> {
    let info = caps
        .state()
        .info(cx)
        .ok_or(ProjectParentError::Disconnected)?;
    if info.status != nocterm_workspace_ports::TerminalStatus::Connected {
        return Err(ProjectParentError::Disconnected);
    }
    let source = caps
        .project_parent()
        .ok_or(ProjectParentError::UnavailableParent)?
        .capture(info.expected, cx)?;
    let exec = caps
        .execution()
        .and_then(|port| port.executor(info.expected, cx).ok());
    let fs = caps
        .fact_session()
        .and_then(|port| port.current(cx))
        .filter(|snapshot| snapshot.expected == info.expected)
        .and_then(|snapshot| snapshot.session.fs);
    Ok(source.with_resources(exec, fs))
}
