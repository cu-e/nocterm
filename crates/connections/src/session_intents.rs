//! Feature-owned conversion and foreground open intent; background routes skip recents.
use crate::Installed;
use gpui_kit::{App, SharedString, WeakEntity, Window};
use nocterm_connections_model::{
    jump_route::{JumpRouteError, resolve_jump_route},
    profiles::{Profile, ProfileId, Profiles, Recent},
};
use nocterm_session::Target;
use nocterm_workspace::{SessionSpec, Workspace};

/// Resolve the original saved workspace server without opening a connection.
/// Profile grouping is insufficient: its destination and ordered route must
/// still match the saved workspace exactly before any authentication or I/O.
pub fn resolve_parent_spec(
    host: &nocterm_project_model::HostLocator,
    profiles: &Profiles,
) -> Result<SessionSpec, nocterm_workspace_ports::ProjectConnectionError> {
    use nocterm_project_model::HostLocator;
    use nocterm_workspace_ports::ProjectConnectionError;
    let HostLocator::Ssh {
        profile,
        target,
        via,
    } = host
    else {
        return Err(ProjectConnectionError::UnsupportedHost);
    };
    let saved = profiles
        .iter()
        .find(|saved| saved.id.to_string() == *profile)
        .ok_or(ProjectConnectionError::MissingProfile)?;
    let spec = spec_for_profile(saved, profiles).map_err(ProjectConnectionError::invalid_route)?;
    if spec.target != *target || spec.address().via != *via {
        return Err(ProjectConnectionError::ChangedTargetOrRoute);
    }
    Ok(spec)
}

/// The session to open for `profile`, with its jump hosts resolved against
/// `profiles`. Fails when a jump host is gone, loops back, or the route is too long.
pub fn spec_for_profile(
    profile: &Profile,
    profiles: &Profiles,
) -> Result<SessionSpec, JumpRouteError> {
    let jumps = resolve_jump_route(profiles, profile)?;
    Ok(SessionSpec {
        profile: Some(profile.id.to_string().into()),
        options: profile.options.clone(),
        title: profile.name.clone().into(),
        target: profile.target.clone(),
        auth: profile.auth.clone(),
        credential: profile.credential,
        launch: profile.launch.clone(),
        jumps,
    })
}
/// What a recent entry shows in the menu: its title and destination.
pub(crate) fn recent_label(
    installed: &Installed,
    recent: &Recent,
    cx: &App,
) -> (SharedString, Target) {
    match recent
        .profile
        .and_then(|id| installed.connections().read(cx).profiles(cx).get(id))
    {
        Some(profile) => (profile.name.clone().into(), profile.target.clone()),
        None => (recent.target.to_string().into(), recent.target.clone()),
    }
}
pub(crate) fn spec_for_recent(
    installed: &Installed,
    recent: &Recent,
    cx: &App,
) -> Result<SessionSpec, JumpRouteError> {
    let profiles = installed.connections().read(cx).profiles(cx);
    match recent.profile.and_then(|id| profiles.get(id)) {
        Some(profile) => spec_for_profile(profile, profiles),
        None => Ok(SessionSpec {
            profile: None,
            options: recent.options.clone(),
            title: recent.target.to_string().into(),
            target: recent.target.clone(),
            auth: recent.auth.clone(),
            credential: recent.credential,
            launch: recent.launch.clone(),
            jumps: Vec::new(),
        }),
    }
}
/// Tells the user why a connection could not be opened.
pub(crate) fn report_route_error(error: &JumpRouteError, window: &mut Window, cx: &mut App) {
    nocterm_ui::notice::error(
        window,
        cx,
        "connection-route",
        "Could not open connection",
        error.to_string(),
    );
}
/// Opens a saved connection in the foreground, or reports why its route cannot
/// be resolved and returns false.
pub(crate) fn open_profile(
    installed: &Installed,
    workspace: &WeakEntity<Workspace>,
    profile: &Profile,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let spec = spec_for_profile(profile, installed.connections().read(cx).profiles(cx));
    match spec {
        Ok(spec) => {
            connect(installed, workspace, spec, Some(profile.id), window, cx);
            true
        }
        Err(error) => {
            report_route_error(&error, window, cx);
            false
        }
    }
}
/// Records the foreground intent before asking the existing workspace session factory.
pub fn connect(
    installed: &Installed,
    workspace: &WeakEntity<Workspace>,
    spec: SessionSpec,
    profile: Option<ProfileId>,
    window: &mut Window,
    cx: &mut App,
) {
    installed.connections().update(cx, |connections, cx| {
        record_use(connections, &spec, profile, cx)
    });
    let _ = workspace.update(cx, |workspace, cx| workspace.open_session(spec, window, cx));
}

/// Compatibility conversion only: service receives plain intent data, never Workspace types.
pub(crate) fn record_use(
    connections: &mut crate::Connections,
    spec: &SessionSpec,
    profile: Option<ProfileId>,
    cx: &mut gpui_kit::Context<crate::Connections>,
) {
    connections.record_use(
        nocterm_connections_model::recents::RecentUse {
            options: spec.options.clone(),
            target: spec.target.clone(),
            auth: spec.auth.clone(),
            profile,
            credential: spec.credential,
            launch: spec.launch.clone(),
        },
        cx,
    );
}
#[cfg(test)]
mod tests;
