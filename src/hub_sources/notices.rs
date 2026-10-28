//! What the user is told when a hub does not let them change a connection.
use nocterm_connections_model::{
    profiles::Profile,
    replica::{ReplicaScope, ReplicaSpace},
};

/// What the user calls the space a refusal came from.
pub(super) fn space_name(spaces: &[ReplicaSpace], scope: &ReplicaScope) -> String {
    spaces
        .iter()
        .find(|space| &space.scope == scope)
        .map_or_else(|| "the organization".to_owned(), |space| space.name.clone())
}

/// Tells the user, once for the refusal, that the hub did not let them change a connection and
/// where it is now.
pub(super) fn refusal_message(
    profile: &Profile,
    space: &str,
    scope: &ReplicaScope,
    restored: bool,
) -> String {
    let name = &profile.name;
    match (restored, scope) {
        (true, _) => format!(
            "You may not change \u{201c}{name}\u{201d} in {space}. The shared version was restored."
        ),
        (false, ReplicaScope::Organization(_)) => format!(
            "You may not add \u{201c}{name}\u{201d} to {space}. It stays in your personal space."
        ),
        (false, ReplicaScope::Personal) => format!(
            "The server refused \u{201c}{name}\u{201d} in your personal space. \
             It stays on this computer and is not synchronized."
        ),
    }
}

/// Shows `message` in the window the user is working in; with no window there is no one to tell.
pub(super) fn notify(message: String, cx: &mut gpui_kit::App) {
    tracing::warn!(%message, "the hub refused a change to a connection");
    let Some(handle) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
    else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        nocterm_ui::notice::warning(
            window,
            cx,
            "connections-refused",
            "Connection not changed",
            message,
        );
    });
}
