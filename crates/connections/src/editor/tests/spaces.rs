use super::*;
use crate::test_support::{organization, replicate, space, workspace};
use gpui_kit::base::actions::Confirm;
use nocterm_connections_model::{replica::ReplicaScope, test_support::MemoryReplica};
use std::sync::Arc;

fn open_new(
    cx: &mut TestAppContext,
    spaces: Vec<nocterm_connections_model::replica::ReplicaSpace>,
) -> (
    gpui_kit::AnyWindowHandle,
    Arc<MemoryReplica>,
    crate::Installed,
) {
    let (handle, workspace, _opened, installed) = workspace(cx);
    let replica = Arc::new(MemoryReplica::new());
    replica.set_spaces(spaces);
    replicate(&installed, replica.clone(), cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (handle, replica, installed)
}

#[gpui_kit::test]
fn a_writer_of_an_organization_chooses_where_the_connection_lives(cx: &mut TestAppContext) {
    let (handle, _replica, _installed) = open_new(
        cx,
        vec![
            space(ReplicaScope::Personal, "Personal", true),
            space(organization("acme"), "Acme", true),
        ],
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("editor-space-personal").is_some());
        assert!(window.try_find("editor-space-acme").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_reader_of_organizations_is_not_offered_a_space(cx: &mut TestAppContext) {
    let (handle, _replica, _installed) = open_new(
        cx,
        vec![
            space(ReplicaScope::Personal, "Personal", true),
            space(organization("guests"), "Guests", false),
        ],
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("editor-space-personal").is_none());
        assert!(window.try_find("editor-space-guests").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_connection_is_saved_in_the_chosen_organization(cx: &mut TestAppContext) {
    let (handle, replica, installed) = open_new(
        cx,
        vec![
            space(ReplicaScope::Personal, "Personal", true),
            space(organization("acme"), "Acme", true),
        ],
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("example.test", cx);
        window.click("editor-space-acme", cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let id = cx.read(|cx| {
        installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .next()
            .unwrap()
            .id
    });
    let (scope, _, _) = replica
        .stored(id)
        .expect("the connection reaches the replica");
    assert_eq!(scope, organization("acme"));
    assert!(replica.stored_in(&ReplicaScope::Personal, id).is_none());
}

#[gpui_kit::test]
fn the_connection_stays_personal_when_no_other_space_is_chosen(cx: &mut TestAppContext) {
    let (handle, replica, installed) = open_new(
        cx,
        vec![
            space(ReplicaScope::Personal, "Personal", true),
            space(organization("acme"), "Acme", true),
        ],
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("example.test", cx);
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let id = cx.read(|cx| {
        installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .next()
            .unwrap()
            .id
    });
    let (scope, _, _) = replica
        .stored(id)
        .expect("the connection reaches the replica");
    assert_eq!(scope, ReplicaScope::Personal);
}
