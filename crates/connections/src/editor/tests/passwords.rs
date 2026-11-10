//! The password a connection signs in with, typed in the "Authentication" section and kept in
//! the window's vault.
use super::*;
use crate::{VaultPlace, test_support::ScriptedPasswords};
use nocterm_connections_model::profiles::Profile;
use nocterm_session::CredentialId;
use std::sync::Arc;

fn saved_profiles(cx: &mut TestAppContext, installed: &crate::Installed) -> Vec<Profile> {
    cx.read(|cx| {
        installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .cloned()
            .collect()
    })
}

fn act(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    steps: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        steps(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Opens the editor of `profile` (a new connection to `prod.example` without one) on its
/// "Authentication" section.
fn open(
    cx: &mut TestAppContext,
    passwords: &Arc<ScriptedPasswords>,
    profile: Option<Profile>,
) -> (gpui_kit::AnyWindowHandle, crate::Installed) {
    let (handle, workspace, _opened, installed) =
        crate::test_support::workspace_with_passwords(cx, passwords.clone());
    if let Some(profile) = &profile {
        cx.update(|cx| {
            use futures::FutureExt as _;
            installed.connections().update(cx, |connections, cx| {
                connections
                    .save_profile(profile.clone(), cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
            });
        });
    }
    cx.simulate_window_resize(handle, gpui_kit::size(px(900.), px(900.)));
    let is_new = profile.is_none();
    act(cx, handle, |window, cx| {
        open_editor(
            installed.clone(),
            profile,
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
        if is_new {
            window.input("prod.example", cx);
        }
        window.click("editor-section-authentication", cx);
    });
    (handle, installed)
}

fn type_password(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, password: &str) {
    act(cx, handle, |window, cx| {
        window.click("editor-field-Password", cx);
        window.input(password, cx);
    });
}

#[gpui_kit::test]
fn a_typed_password_goes_to_the_windows_vault_and_the_connection_names_its_record(
    cx: &mut TestAppContext,
) {
    let passwords = Arc::new(ScriptedPasswords::new(VaultPlace::Account));
    let (handle, installed) = open(cx, &passwords, None);
    act(cx, handle, |window, cx| window.click("auth-password", cx));
    type_password(cx, handle, "s3cret");
    act(cx, handle, |window, cx| window.click("editor-save", cx));

    let kept = passwords.kept.lock().unwrap().clone();
    assert_eq!(kept.len(), 1);
    let (replacing, label, target, password) = &kept[0];
    assert_eq!(*replacing, None);
    assert_eq!(label, "prod.example");
    assert_eq!(*target, Target::new("tester", "prod.example", 22));
    assert_eq!(password, "s3cret");
    let profiles = saved_profiles(cx, &installed);
    assert_eq!(profiles[0].auth, Auth::Password);
    assert_eq!(
        profiles[0].credential,
        Some(CredentialId::from_bytes([1; 16]))
    );
}

#[gpui_kit::test]
fn a_new_password_replaces_the_saved_one_and_forgetting_unlinks_it(cx: &mut TestAppContext) {
    let passwords = Arc::new(ScriptedPasswords::new(VaultPlace::Account));
    let id = CredentialId::from_bytes([9; 16]);
    let mut draft = fields("prod.example");
    draft.endpoint.user = "ops".into();
    let mut profile = build(&draft, AuthKind::Password).unwrap();
    profile.credential = Some(id);
    let (handle, installed) = open(cx, &passwords, Some(profile.clone()));
    type_password(cx, handle, "n3w");
    act(cx, handle, |window, cx| window.click("editor-save", cx));
    let kept = passwords.kept.lock().unwrap().clone();
    assert_eq!(
        kept[0].0,
        Some(id),
        "the saved record takes the new password"
    );
    assert_eq!(saved_profiles(cx, &installed)[0].credential, Some(id));

    profile = saved_profiles(cx, &installed).remove(0);
    let (handle, installed) = open(cx, &passwords, Some(profile));
    act(cx, handle, |window, cx| {
        window.click("editor-forget-password", cx)
    });
    act(cx, handle, |window, cx| {
        assert!(window.try_find("editor-forget-password").is_none());
        window.click("editor-save", cx);
    });
    assert_eq!(saved_profiles(cx, &installed)[0].credential, None);
    assert_eq!(passwords.kept.lock().unwrap().len(), 1, "nothing new kept");
}

#[gpui_kit::test]
fn a_locked_vault_keeps_the_editor_open_and_saves_nothing(cx: &mut TestAppContext) {
    let passwords = Arc::new(ScriptedPasswords::new(VaultPlace::Account));
    passwords
        .locked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (handle, installed) = open(cx, &passwords, None);
    type_password(cx, handle, "s3cret");
    act(cx, handle, |window, cx| window.click("editor-save", cx));
    act(cx, handle, |window, _| {
        assert!(
            window.try_find("editor-save").is_some(),
            "the editor stays open"
        );
    });
    assert!(saved_profiles(cx, &installed).is_empty());
}

#[gpui_kit::test]
fn a_hub_window_offers_no_key_file_and_this_computer_does(cx: &mut TestAppContext) {
    for (place, offered) in [
        (VaultPlace::Account, false),
        (VaultPlace::ThisComputer, true),
    ] {
        let passwords = Arc::new(ScriptedPasswords::new(place));
        let (handle, _) = open(cx, &passwords, None);
        act(cx, handle, |window, _| {
            assert_eq!(window.try_find("auth-key").is_some(), offered);
            assert!(window.try_find("auth-vault-key").is_some());
        });
    }
}

#[gpui_kit::test]
fn a_saved_password_is_chosen_from_the_vault_and_signs_in_to_the_new_server_too(
    cx: &mut TestAppContext,
) {
    let passwords = Arc::new(ScriptedPasswords::new(VaultPlace::Account));
    let office = CredentialId::from_bytes([5; 16]);
    passwords.saved.lock().unwrap().push(crate::SavedPassword {
        id: office,
        label: "Office".into(),
        servers: 2,
    });
    let (handle, installed) = open(cx, &passwords, None);
    type_password(cx, handle, "typed first");
    act(cx, handle, |window, cx| {
        window.click("saved-password-0", cx)
    });
    act(cx, handle, |window, cx| {
        assert!(window.try_find("editor-forget-password").is_some());
        window.click("editor-save", cx);
    });
    assert!(
        passwords.kept.lock().unwrap().is_empty(),
        "choosing drops the typed one"
    );
    assert_eq!(
        *passwords.attached.lock().unwrap(),
        vec![(office, Target::new("tester", "prod.example", 22))]
    );
    let profile = saved_profiles(cx, &installed).remove(0);
    assert_eq!(profile.credential, Some(office));

    // Saving it again without changing the password or the address attaches nothing more.
    let (handle, _) = open(cx, &passwords, Some(profile));
    act(cx, handle, |window, cx| window.click("editor-save", cx));
    assert_eq!(passwords.attached.lock().unwrap().len(), 1);
}

#[gpui_kit::test]
fn the_search_narrows_the_saved_passwords_by_name(cx: &mut TestAppContext) {
    let passwords = Arc::new(ScriptedPasswords::new(VaultPlace::Account));
    for (byte, label) in [(1u8, "Office"), (2, "homelab"), (3, "Office VPN")] {
        passwords.saved.lock().unwrap().push(crate::SavedPassword {
            id: CredentialId::from_bytes([byte; 16]),
            label: label.into(),
            servers: 1,
        });
    }
    let (handle, _) = open(cx, &passwords, None);
    let shown = |window: &mut gpui_kit::Window| {
        (0..3)
            .filter(|index| window.try_find(format!("saved-password-{index}")).is_some())
            .collect::<Vec<_>>()
    };
    act(cx, handle, |window, cx| {
        assert_eq!(shown(window), [0, 1, 2]);
        window.click("saved-passwords-search", cx);
        window.input("office", cx);
    });
    act(cx, handle, |window, cx| {
        assert_eq!(
            shown(window),
            [0, 2],
            "each keeps its place in the vault's list"
        );
        window.click("saved-password-2", cx);
    });
    act(cx, handle, |window, _| {
        assert!(window.try_find("editor-forget-password").is_some());
    });
}
