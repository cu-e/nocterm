//! Group confirmation dispatch captures the original connection owner.
use super::*;
#[derive(Clone, Copy)]
pub(super) enum GroupAction {
    Ungroup,
    Delete,
}

pub(super) fn run_group_action(
    connections: &Entity<Connections>,
    action: GroupAction,
    group: String,
    expected: Vec<ProfileId>,
    panel: WeakEntity<ConnectionsPanel>,
    window: &mut Window,
    cx: &mut App,
) {
    let (done, key, title) = connections.update(cx, |connections, cx| match action {
        GroupAction::Ungroup => (
            connections.ungroup(group.clone(), cx),
            "connections-ungroup",
            "Could not ungroup connections",
        ),
        GroupAction::Delete => (
            connections.delete_group(group.clone(), expected, cx),
            "connections-delete-group",
            "Could not delete group",
        ),
    });
    window
        .spawn(cx, async move |cx| match done.await {
            Ok(()) => {
                let _ = panel.update(cx, |panel, cx| {
                    panel.collapsed.remove(&group);
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = cx.update(|window, cx| {
                    nocterm_ui::notice::error(window, cx, key, title, error.to_string())
                });
            }
        })
        .detach();
}
