use super::*;
use crate::test_support::{organization, replicate, workspace};
use gpui_kit::TestAppContext;
use nocterm_connections_model::{
    profiles::{Profile, ProfileId},
    test_support::MemoryReplica,
};
use nocterm_session::{Auth, CredentialId, Target};
use std::sync::Arc;

fn profile(name: &str) -> Profile {
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for(name),
        name: name.into(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("root", "db1.example", 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    }
}

fn credential_of(
    installed: &crate::Installed,
    id: ProfileId,
    cx: &TestAppContext,
) -> Option<CredentialId> {
    installed.connections().read_with(cx, |connections, cx| {
        connections.profiles(cx).get(id).and_then(|p| p.credential)
    })
}

#[gpui_kit::test]
async fn a_password_saved_for_a_personal_connection_skips_the_shared_one_with_the_same_address(
    cx: &mut TestAppContext,
) {
    let (_handle, _workspace, _opened, installed) = workspace(cx);
    let (mine, team) = (profile("mine"), profile("team"));
    let replica = Arc::new(MemoryReplica::new());
    replica.write_remote(organization("acme"), team.clone());
    let saving = installed.connections().update(cx, |connections, cx| {
        connections.save_profile(mine.clone(), cx)
    });
    saving.await.unwrap();
    replicate(&installed, replica, cx);

    let secret = CredentialId::from_bytes([3; 16]);
    let scope = CredentialScope::direct(mine.target.clone(), mine.auth.clone());
    let scope = cx.update(|cx| {
        installed
            .credential_spaces()
            .within_space_of(scope, Some(mine.id), cx)
    });
    installed
        .connections()
        .update(cx, |connections, cx| {
            connections.associate_credential(scope, secret, cx)
        })
        .await
        .unwrap();

    assert_eq!(credential_of(&installed, mine.id, cx), Some(secret));
    assert_eq!(credential_of(&installed, team.id, cx), None);
}

#[gpui_kit::test]
async fn without_an_account_every_connection_is_personal_and_nothing_is_limited(
    cx: &mut TestAppContext,
) {
    let (_handle, _workspace, _opened, installed) = workspace(cx);
    let (one, two) = (profile("one"), profile("two"));
    for item in [&one, &two] {
        installed
            .connections()
            .update(cx, |connections, cx| {
                connections.save_profile(item.clone(), cx)
            })
            .await
            .unwrap();
    }
    let secret = CredentialId::from_bytes([4; 16]);
    let scope = CredentialScope::direct(one.target.clone(), one.auth.clone());
    let scope = cx.update(|cx| {
        installed
            .credential_spaces()
            .within_space_of(scope, Some(one.id), cx)
    });
    installed
        .connections()
        .update(cx, |connections, cx| {
            connections.associate_credential(scope, secret, cx)
        })
        .await
        .unwrap();
    assert_eq!(credential_of(&installed, one.id, cx), Some(secret));
    assert_eq!(credential_of(&installed, two.id, cx), Some(secret));
}
