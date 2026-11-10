//! What the user is told when a hub does not let them change a connection.
use nocterm_connections_model::profiles::Profile;

/// Tells the user, once for the refusal, that the hub did not let them change a connection and
/// what became of it.
pub(super) fn refusal_message(profile: &Profile, restored: bool) -> String {
    let name = &profile.name;
    if restored {
        format!(
            "The server refused the change to \u{201c}{name}\u{201d}. The synchronized version was restored."
        )
    } else {
        format!(
            "The server refused \u{201c}{name}\u{201d}. It stays on this computer and is not synchronized."
        )
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
