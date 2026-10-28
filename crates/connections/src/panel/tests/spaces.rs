use super::*;
use crate::test_support::{organization, replicate, space};
use nocterm_connections_model::{replica::ReplicaScope, test_support::MemoryReplica};

/// A panel over an account with three spaces: the user's own, "Acme" they write to and
/// "Guests" they only read. Each organization holds one connection, and `own` is personal.
fn shared(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<ConnectionsPanel>,
    [Profile; 3],
) {
    shared_in(cx, None)
}

/// The same, with `own` and `team` both filed in a folder called `folder`.
fn shared_in(
    cx: &mut TestAppContext,
    folder: Option<&str>,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<ConnectionsPanel>,
    [Profile; 3],
) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let (own, team, seen) = (gp("own", folder), gp("team", folder), gp("seen", None));
    let replica = Arc::new(MemoryReplica::new());
    replica.set_spaces(vec![
        space(ReplicaScope::Personal, "Personal", true),
        space(organization("acme"), "Acme", true),
        space(organization("guests"), "Guests", false),
    ]);
    replica.write_remote(organization("acme"), team.clone());
    replica.write_remote(organization("guests"), seen.clone());
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(own.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap()
        });
    });
    replicate(&installed, replica, cx);
    let panel = cx
        .update_window(handle, |_, window, cx| {
            let panel = cx.new(|cx| {
                ConnectionsPanel::new(installed.clone(), workspace.downgrade(), window, cx)
            });
            workspace.update(cx, |workspace, cx| {
                add_servers(workspace, panel.clone(), window, cx)
            });
            window.render_frame(cx);
            panel
        })
        .unwrap();
    (handle, panel, [own, team, seen])
}

fn has(window: &mut Window, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    window.try_find(id).is_some()
}

#[gpui_kit::test]
fn each_organization_has_a_section_and_a_read_only_one_says_so(cx: &mut TestAppContext) {
    let (handle, _panel, [own, team, seen]) = shared(cx);
    cx.update_window(handle, |_, window, _| {
        assert!(has(window, "connections-space-acme"));
        assert!(has(window, "connections-space-guests"));
        assert!(has(window, "connections-space-guests-readonly"));
        assert!(!has(window, "connections-space-acme-readonly"));
        for profile in [&own, &team, &seen] {
            assert!(has(window, format!("connection-{}", profile.id)));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_connection_of_a_read_only_space_has_no_edit_or_delete_button(cx: &mut TestAppContext) {
    let (handle, _panel, [own, team, seen]) = shared(cx);
    cx.update_window(handle, |_, window, _| {
        for writable in [&own, &team] {
            assert!(has(window, format!("edit-{}", writable.id)));
            assert!(has(window, format!("delete-{}", writable.id)));
        }
        assert!(!has(window, format!("edit-{}", seen.id)));
        assert!(!has(window, format!("delete-{}", seen.id)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_read_only_connection_is_neither_edited_nor_deleted_by_the_panel(cx: &mut TestAppContext) {
    let (handle, panel, [_, team, seen]) = shared(cx);
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.edit(seen.id, window, cx);
            panel.confirm_delete(seen.id, window, cx);
        });
        window.render_frame(cx);
        assert!(!has(window, "dialog"), "nothing opens for a reader");
        panel.update(cx, |panel, cx| panel.confirm_delete(team.id, window, cx));
        window.render_frame(cx);
        assert!(has(window, "dialog"), "a writer may delete");
    })
    .unwrap();
}

#[gpui_kit::test]
fn without_an_account_the_list_has_no_sections(cx: &mut TestAppContext) {
    let a = gp("a", None);
    let (handle, _panel, _opened, _installed) = setup(cx, std::slice::from_ref(&a));
    cx.update_window(handle, |_, window, _| {
        assert!(has(window, format!("edit-{}", a.id)));
        assert!(!has(window, "connections-space-acme"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn deleting_a_folder_leaves_the_connections_of_an_organization_with_the_same_name(
    cx: &mut TestAppContext,
) {
    let (handle, panel, [own, team, _]) = shared_in(cx, Some("Work"));
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| window.click("group-delete-all", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let connections = panel.read(cx).installed.connections().clone();
        let profiles = connections.read(cx).profiles(cx);
        assert!(
            profiles.get(own.id).is_none(),
            "the personal folder's connection goes"
        );
        assert!(profiles.get(team.id).is_some(), "the organization's stays");
    })
    .unwrap();
}

#[gpui_kit::test]
fn renaming_a_folder_leaves_the_connections_of_an_organization_with_the_same_name(
    cx: &mut TestAppContext,
) {
    let (handle, panel, [own, team, _]) = shared_in(cx, Some("Work"));
    start_rename_via_double_click(cx, handle, "Work");
    set_rename_value(cx, handle, &panel, "Team");
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let connections = panel.read(cx).installed.connections().clone();
        let profiles = connections.read(cx).profiles(cx);
        assert_eq!(profiles.get(own.id).unwrap().group.as_deref(), Some("Team"));
        assert_eq!(
            profiles.get(team.id).unwrap().group.as_deref(),
            Some("Work")
        );
    })
    .unwrap();
}

/// A panel after the account lost sight of "Ghost": `lost` is known to live there, `own` is
/// personal, and both are filed in `folder`.
fn orphaned(
    cx: &mut TestAppContext,
    folder: Option<&str>,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<ConnectionsPanel>,
    [Profile; 2],
) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let (own, lost) = (gp("own", folder), gp("lost", folder));
    let replica = Arc::new(MemoryReplica::new());
    let writable = |scope, name| space(scope, name, true);
    replica.set_spaces(vec![
        writable(ReplicaScope::Personal, "Personal"),
        writable(organization("ghost"), "Ghost"),
    ]);
    replica.write_remote(organization("ghost"), lost.clone());
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(own.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap()
        });
    });
    replicate(&installed, replica.clone(), cx);
    replica.set_spaces(vec![writable(ReplicaScope::Personal, "Personal")]);
    let panel = cx
        .update_window(handle, |_, window, cx| {
            let panel = cx.new(|cx| {
                ConnectionsPanel::new(installed.clone(), workspace.downgrade(), window, cx)
            });
            workspace.update(cx, |workspace, cx| {
                add_servers(workspace, panel.clone(), window, cx)
            });
            window.render_frame(cx);
            panel
        })
        .unwrap();
    (handle, panel, [own, lost])
}

#[gpui_kit::test]
fn a_connection_of_an_unreachable_organization_is_listed_read_only(cx: &mut TestAppContext) {
    let (handle, _panel, [own, lost]) = orphaned(cx, None);
    cx.update_window(handle, |_, window, _| {
        assert!(has(window, "connections-space-unavailable"));
        assert!(has(window, "connections-space-unavailable-readonly"));
        assert!(has(window, format!("connection-{}", lost.id)));
        assert!(!has(window, format!("edit-{}", lost.id)));
        assert!(!has(window, format!("delete-{}", lost.id)));
        assert!(
            has(window, format!("edit-{}", own.id)),
            "the user's own stays editable"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_unreachable_connection_is_not_filed_in_a_personal_folder(cx: &mut TestAppContext) {
    let (handle, _panel, [_, lost]) = orphaned(cx, Some("Work"));
    cx.update_window(handle, |_, window, _| {
        assert!(has(window, "group-members-Work"));
        assert!(has(window, "connections-space-unavailable"));
        assert!(has(window, format!("connection-{}", lost.id)));
        assert!(!has(window, format!("edit-{}", lost.id)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn deleting_a_folder_leaves_the_connections_of_an_unreachable_organization(
    cx: &mut TestAppContext,
) {
    let (handle, panel, [own, lost]) = orphaned(cx, Some("Work"));
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| window.click("group-delete-all", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let connections = panel.read(cx).installed.connections().clone();
        let profiles = connections.read(cx).profiles(cx);
        assert!(profiles.get(own.id).is_none());
        assert!(profiles.get(lost.id).is_some(), "the organization's stays");
    })
    .unwrap();
}
