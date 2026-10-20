//! Feature-owned conversion and foreground open intent; background routes skip recents.
use crate::Installed;
use gpui_kit::{App, WeakEntity, Window};
use nocterm_connections_model::profiles::{Profile, ProfileId, Recent};
use nocterm_workspace::{SessionSpec, Workspace};
pub fn spec_for_profile(profile: &Profile) -> SessionSpec {
    SessionSpec {
        profile: Some(profile.id.to_string().into()),
        options: profile.options.clone(),
        title: profile.name.clone().into(),
        target: profile.target.clone(),
        auth: profile.auth.clone(),
        credential: profile.credential,
        launch: profile.launch.clone(),
        jumps: Vec::new(),
    }
}
pub(crate) fn spec_for_recent(installed: &Installed, recent: &Recent, cx: &App) -> SessionSpec {
    match recent
        .profile
        .and_then(|id| installed.connections().read(cx).profiles(cx).get(id))
    {
        Some(profile) => spec_for_profile(profile),
        None => SessionSpec {
            profile: None,
            options: recent.options.clone(),
            title: recent.target.to_string().into(),
            target: recent.target.clone(),
            auth: recent.auth.clone(),
            credential: recent.credential,
            launch: recent.launch.clone(),
            jumps: Vec::new(),
        },
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
