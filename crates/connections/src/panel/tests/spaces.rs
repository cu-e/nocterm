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
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let (own, team, seen) = (gp("own", None), gp("team", None), gp("seen", None));
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
