use super::*;
use crate::test_support::{replicate, unattached};
use nocterm_connections_model::test_support::MemoryReplica;
use nocterm_connections_store::{ReplicationFailure, ReplicationStatus};
use nocterm_workspace::UnlockVault;
use std::{cell::Cell, rc::Rc};

gpui_kit::actions!(hubs, [SignInToServer]);

/// How the account stands in a panel over one personal connection.
enum Account {
    /// Attached to a replica that answers.
    Synced,
    /// Attached to a replica that cannot be reached.
    Offline,
    /// Attached to a replica whose keys are closed.
    Locked,
    /// Bound to an account that is signed out.
    SignedOut,
}

fn account(
    cx: &mut TestAppContext,
    state: Account,
) -> (gpui_kit::AnyWindowHandle, Entity<ConnectionsPanel>, Profile) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let own = gp("own", None);
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(own.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap()
        });
    });
    match state {
        Account::SignedOut => {
            unattached(&installed, cx);
        }
        state => {
            let replica = Arc::new(MemoryReplica::new());
            match state {
                Account::Offline => replica.set_offline(true),
                Account::Locked => replica.set_locked(true),
                _ => {}
            }
            replicate(&installed, replica, cx);
        }
    }
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
    (handle, panel, own)
}

fn has(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, id: &str) -> bool {
    cx.update_window(handle, |_, window, _| {
        window.try_find(id.to_owned()).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_list_in_step_with_the_account_has_no_strip_and_can_be_changed(cx: &mut TestAppContext) {
    let (handle, _panel, own) = account(cx, Account::Synced);
    assert!(!has(cx, handle, "connections-account-notice"));
    assert!(has(cx, handle, &format!("edit-{}", own.id)));
    assert!(has(cx, handle, &format!("delete-{}", own.id)));
}

#[gpui_kit::test]
fn a_locked_account_shows_an_unlock_strip_that_asks_for_the_vault(cx: &mut TestAppContext) {
    let (handle, _panel, _own) = account(cx, Account::Locked);
    assert!(has(cx, handle, "connections-account-notice"));
    let asked = Rc::new(Cell::new(0));
    let counter = asked.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &UnlockVault, _| counter.set(counter.get() + 1));
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("connections-account-action", cx)
    })
    .unwrap();
    assert_eq!(asked.get(), 1);
}

#[gpui_kit::test]
fn a_signed_out_account_shows_a_sign_in_strip_that_asks_to_sign_in(cx: &mut TestAppContext) {
    let (handle, _panel, _own) = account(cx, Account::SignedOut);
    assert!(has(cx, handle, "connections-account-notice"));
    let asked = Rc::new(Cell::new(0));
    let counter = asked.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &SignInToServer, _| counter.set(counter.get() + 1));
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("connections-account-action", cx)
    })
    .unwrap();
    assert_eq!(asked.get(), 1);
}

#[gpui_kit::test]
fn an_unreachable_server_is_told_without_a_remedy(cx: &mut TestAppContext) {
    let (handle, _panel, _own) = account(cx, Account::Offline);
    assert!(has(cx, handle, "connections-account-notice"));
    assert!(!has(cx, handle, "connections-account-action"));
}

#[gpui_kit::test]
fn outside_sync_nothing_is_added_edited_deleted_or_moved(cx: &mut TestAppContext) {
    for state in [Account::Offline, Account::Locked, Account::SignedOut] {
        let (handle, panel, own) = account(cx, state);
        assert!(!has(cx, handle, &format!("edit-{}", own.id)));
        assert!(!has(cx, handle, &format!("delete-{}", own.id)));
        cx.update_window(handle, |_, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.edit(own.id, window, cx);
                panel.confirm_delete(own.id, window, cx);
                panel.start_rename("Work", window, cx);
                panel.confirm_delete_group("Work".into(), window, cx);
            });
            crate::open_editor(
                panel.read(cx).installed.clone(),
                None,
                panel.read(cx).workspace.clone(),
                window,
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("dialog".to_owned()).is_none());
        })
        .unwrap();
    }
}

#[test]
fn only_a_list_in_step_with_the_account_is_editable() {
    use crate::panel::account_state::{editable, notice};
    let failed = ReplicationStatus::Failed(ReplicationFailure::Conflicts);
    for status in [ReplicationStatus::Synced, ReplicationStatus::Syncing] {
        assert!(editable(&status));
        assert!(notice(&status).is_none());
    }
    for status in [
        ReplicationStatus::Detached,
        ReplicationStatus::Offline,
        ReplicationStatus::Locked,
        ReplicationStatus::NeedsNewerClient,
        failed,
    ] {
        assert!(!editable(&status), "{status:?}");
        assert!(notice(&status).is_some(), "{status:?}");
    }
}
