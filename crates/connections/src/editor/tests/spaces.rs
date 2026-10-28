use super::*;
use crate::test_support::{organization, replicate, space, workspace};
use gpui_kit::base::actions::Confirm;
use nocterm_connections_model::{
    profiles::Profile, replica::ReplicaScope, test_support::MemoryReplica,
};
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

#[gpui_kit::test]
fn a_failed_save_leaves_no_pending_move(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = workspace(cx);
    let replica = Arc::new(MemoryReplica::new());
    replica.set_spaces(vec![
        space(ReplicaScope::Personal, "Personal", true),
        space(organization("acme"), "Acme", true),
    ]);
    let original = Profile {
        id: nocterm_connections_model::test_support::profile_id_for("kept"),
        name: "kept".into(),
        group: None,
        target: nocterm_session::Target::new("tester", "kept.test", 22),
        description: String::new(),
        options: Default::default(),
        auth: Default::default(),
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        jump_hosts: Vec::new(),
    };
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            futures::FutureExt::now_or_never(connections.save_profile(original.clone(), cx))
                .unwrap()
                .unwrap()
        });
    });
    let replication = replicate(&installed, replica.clone(), cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(
            installed.clone(),
            Some(original.clone()),
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Someone else changes the connection while the form is open, so the save is refused.
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            let changed = Profile {
                description: "changed elsewhere".into(),
                ..original.clone()
            };
            futures::FutureExt::now_or_never(connections.save_profile(changed, cx))
                .unwrap()
                .unwrap()
        });
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-space-acme", cx);
        window.render_frame(cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some(), "the form stays open");
    })
    .unwrap();
    let space = replication.read_with(cx, |replication, cx| replication.space_of(original.id, cx));
    assert_eq!(space, ReplicaScope::Personal);
    assert!(
        replica
            .stored_in(&organization("acme"), original.id)
            .is_none()
    );
}

fn existing(auth: nocterm_session::Auth, credential: bool) -> Profile {
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for("secret"),
        name: "secret".into(),
        group: None,
        target: nocterm_session::Target::new("tester", "secret.test", 22),
        description: String::new(),
        options: Default::default(),
        auth,
        credential: credential.then(|| nocterm_session::CredentialId::from_bytes([7; 16])),
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        jump_hosts: Vec::new(),
    }
}

/// Opens `profile` in an editor over an account that writes to "Acme", then chooses `space`
/// and says whether the note about the shared secret is shown.
fn note_after_choosing(cx: &mut TestAppContext, profile: Profile, space: &'static str) -> bool {
    let (handle, workspace, _opened, installed) = workspace(cx);
    let replica = Arc::new(MemoryReplica::new());
    replica.set_spaces(vec![
        space_of(ReplicaScope::Personal, "Personal"),
        space_of(organization("acme"), "Acme"),
    ]);
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            futures::FutureExt::now_or_never(connections.save_profile(profile.clone(), cx))
                .unwrap()
                .unwrap()
        });
    });
    replicate(&installed, replica, cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(
            installed.clone(),
            Some(profile),
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
        window.click(space, cx);
        window.render_frame(cx);
        window.try_find("editor-space-secret-note").is_some()
    })
    .unwrap()
}

fn space_of(scope: ReplicaScope, name: &str) -> nocterm_connections_model::replica::ReplicaSpace {
    space(scope, name, true)
}

#[gpui_kit::test]
fn a_saved_password_is_said_to_be_shared_when_an_organization_is_chosen(cx: &mut TestAppContext) {
    let password = existing(nocterm_session::Auth::Password, true);
    assert!(note_after_choosing(cx, password, "editor-space-acme"));
}

#[gpui_kit::test]
fn a_vault_key_is_said_to_be_shared_when_an_organization_is_chosen(cx: &mut TestAppContext) {
    let auth = nocterm_session::Auth::VaultKey {
        credential: nocterm_session::CredentialId::from_bytes([9; 16]),
        public_key: "ssh-ed25519 AAAA".to_owned(),
    };
    assert!(note_after_choosing(
        cx,
        existing(auth, false),
        "editor-space-acme"
    ));
}

#[gpui_kit::test]
fn nothing_is_said_while_the_connection_stays_personal(cx: &mut TestAppContext) {
    let password = existing(nocterm_session::Auth::Password, true);
    assert!(!note_after_choosing(cx, password, "editor-space-personal"));
}

#[gpui_kit::test]
fn nothing_is_said_about_a_connection_without_a_saved_secret(cx: &mut TestAppContext) {
    let plain = existing(nocterm_session::Auth::Auto, false);
    assert!(!note_after_choosing(cx, plain, "editor-space-acme"));
}
