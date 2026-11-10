//! Choosing, creating and importing vault keys in the "Authentication" section.
use super::*;
use futures::FutureExt as _;
use nocterm_connections_model::profiles::Profile;
use nocterm_ssh_keys::{VaultSshKeys, testing::MemoryVaultSshKeys};
use std::sync::Arc;

const ENCRYPTED: &str = include_str!("../../../../ssh-keys/src/tests/fixtures/ed25519-encrypted");
const PASSPHRASE: &str = "correct horse";

fn keys_of(keys: &MemoryVaultSshKeys) -> Vec<nocterm_ssh_keys::StoredKey> {
    keys.list().now_or_never().unwrap().unwrap()
}

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

fn open_authentication(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    workspace: &gpui_kit::Entity<nocterm_workspace::Workspace>,
    installed: &crate::Installed,
    profile: Option<Profile>,
) {
    cx.simulate_window_resize(handle, gpui_kit::size(px(900.), px(900.)));
    let is_new = profile.is_none();
    cx.update_window(handle, |_, window, cx| {
        open_editor(
            installed.clone(),
            profile,
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
        if is_new {
            window.input("vault.test", cx);
        }
        window.click("editor-section-authentication", cx);
    })
    .unwrap();
    cx.run_until_parked();
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

#[gpui_kit::test]
fn a_created_key_is_chosen_and_the_connection_saves_with_it(cx: &mut TestAppContext) {
    let keys = MemoryVaultSshKeys::default();
    let (handle, workspace, _opened, installed) =
        crate::test_support::workspace_with_keys(cx, Arc::new(keys.clone()));
    open_authentication(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| window.click("auth-vault-key", cx));
    act(cx, handle, |window, cx| {
        assert!(window.try_find("vault-key-0").is_none(), "an empty vault");
        window.click("vault-key-create", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(
            window.try_find("vault-key-0").is_some(),
            "the new key is listed"
        );
        window.click("editor-save", cx);
    });
    let stored = keys_of(&keys);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].label, "vault.test");
    let profiles = saved_profiles(cx, &installed);
    assert_eq!(
        profiles[0].auth,
        Auth::VaultKey {
            credential: stored[0].credential,
            public_key: stored[0].public_key.as_str().to_owned(),
        }
    );
    assert_eq!(
        profiles[0].credential, None,
        "the key needs no password record"
    );
}

#[gpui_kit::test]
fn a_key_already_in_the_vault_is_chosen_from_the_list(cx: &mut TestAppContext) {
    let keys = MemoryVaultSshKeys::default();
    keys.generate("first".into())
        .now_or_never()
        .unwrap()
        .unwrap();
    let second = keys
        .generate("second".into())
        .now_or_never()
        .unwrap()
        .unwrap();
    let (handle, workspace, _opened, installed) =
        crate::test_support::workspace_with_keys(cx, Arc::new(keys.clone()));
    open_authentication(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| window.click("auth-vault-key", cx));
    act(cx, handle, |window, cx| window.click("vault-key-1", cx));
    act(cx, handle, |window, cx| window.click("editor-save", cx));
    let profiles = saved_profiles(cx, &installed);
    assert!(matches!(
        &profiles[0].auth,
        Auth::VaultKey { credential, .. } if *credential == second.credential
    ));
}

#[gpui_kit::test]
fn saving_without_a_chosen_key_keeps_the_editor_open(cx: &mut TestAppContext) {
    let keys = MemoryVaultSshKeys::default();
    keys.set_locked(true);
    let (handle, workspace, _opened, installed) =
        crate::test_support::workspace_with_keys(cx, Arc::new(keys.clone()));
    open_authentication(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| window.click("auth-vault-key", cx));
    act(cx, handle, |window, cx| {
        window.click("vault-key-create", cx)
    });
    act(cx, handle, |window, cx| window.click("editor-save", cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(window.try_find("vault-key-0").is_none());
    })
    .unwrap();
    assert!(saved_profiles(cx, &installed).is_empty());
}

#[gpui_kit::test]
fn an_encrypted_key_file_is_imported_after_its_passphrase_is_asked(cx: &mut TestAppContext) {
    let keys = MemoryVaultSshKeys::default()
        .with_file("/keys/id_ed25519", nocterm_session::Secret::new(ENCRYPTED));
    let (handle, workspace, _opened, installed) =
        crate::test_support::workspace_with_keys(cx, Arc::new(keys.clone()));
    open_authentication(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| {
        window.click("auth-vault-key", cx);
    });
    act(cx, handle, |window, cx| {
        window.click("editor-field-Private key", cx);
        window.input("/keys/id_ed25519", cx);
        assert!(window.try_find("editor-field-Passphrase").is_none());
        window.click("vault-key-import", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(
            window.try_find("vault-key-0").is_none(),
            "nothing stored yet"
        );
        window.click("editor-field-Passphrase", cx);
        window.input(PASSPHRASE, cx);
        window.click("vault-key-import", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(window.try_find("vault-key-0").is_some());
        window.click("editor-save", cx);
    });
    let stored = keys_of(&keys);
    assert_eq!(stored.len(), 1);
    // The vault holds the key without its passphrase.
    let text = keys.secret_of(stored[0].credential).unwrap();
    assert!(nocterm_ssh_keys::KeyMaterial::import(&text, None).is_ok());
    assert!(matches!(
        &saved_profiles(cx, &installed)[0].auth,
        Auth::VaultKey { credential, .. } if *credential == stored[0].credential
    ));
}

#[gpui_kit::test]
fn moving_a_key_file_to_the_vault_turns_the_connection_into_a_vault_key_one(
    cx: &mut TestAppContext,
) {
    let plain = include_str!("../../../../ssh-keys/src/tests/fixtures/ed25519");
    let keys =
        MemoryVaultSshKeys::default().with_file("/keys/id", nocterm_session::Secret::new(plain));
    let (handle, workspace, _opened, installed) =
        crate::test_support::workspace_with_keys(cx, Arc::new(keys.clone()));
    let mut draft = fields("moved.test");
    draft.authentication.key_path = "/keys/id".into();
    let mut profile = build(&draft, AuthKind::Key).unwrap();
    profile.name = "Moved".into();
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(profile.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
    });
    open_authentication(cx, handle, &workspace, &installed, Some(profile));
    act(cx, handle, |window, cx| window.click("vault-key-move", cx));
    act(cx, handle, |window, cx| {
        assert!(
            window.try_find("vault-key-0").is_some(),
            "the vault key view is shown"
        );
        window.click("editor-save", cx);
    });
    let stored = keys_of(&keys);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].label, "Moved");
    assert!(matches!(
        &saved_profiles(cx, &installed)[0].auth,
        Auth::VaultKey { credential, .. } if *credential == stored[0].credential
    ));
}

fn events_of(
    cx: &mut TestAppContext,
    keys: &MemoryVaultSshKeys,
    run: impl FnOnce(
        &mut crate::editor::vault_key::VaultKeyField,
        &mut gpui_kit::Window,
        &mut gpui_kit::Context<crate::editor::vault_key::VaultKeyField>,
    ),
) -> Vec<crate::editor::vault_key::VaultKeyEvent> {
    use crate::editor::vault_key::VaultKeyField;
    let (handle, _workspace, _opened, _installed) =
        crate::test_support::workspace_with_keys(cx, Arc::new(keys.clone()));
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = events.clone();
    let field = cx
        .update_window(handle, |_, window, cx| {
            let field = cx.new(|cx| VaultKeyField::new(Arc::new(keys.clone()), None, window, cx));
            cx.subscribe(&field, move |_, event, _| {
                seen.borrow_mut().push(event.clone())
            })
            .detach();
            field.update(cx, |field, cx| run(field, window, cx));
            field
        })
        .unwrap();
    cx.run_until_parked();
    drop(field);
    events.borrow().clone()
}

#[gpui_kit::test]
fn the_key_field_reports_a_created_key_as_chosen(cx: &mut TestAppContext) {
    let keys = MemoryVaultSshKeys::default();
    let events = events_of(cx, &keys, |field, window, cx| {
        field.create("made", window, cx)
    });
    assert_eq!(events, [crate::editor::vault_key::VaultKeyEvent::Chosen]);
}

#[gpui_kit::test]
fn the_key_field_reports_a_refusal_and_a_protected_file_as_events(cx: &mut TestAppContext) {
    use crate::editor::vault_key::VaultKeyEvent;
    let locked = MemoryVaultSshKeys::default();
    locked.set_locked(true);
    let events = events_of(cx, &locked, |field, window, cx| {
        field.create("made", window, cx)
    });
    assert!(matches!(events.as_slice(), [VaultKeyEvent::Failed(_)]));

    let protected = MemoryVaultSshKeys::default()
        .with_file("/keys/id", nocterm_session::Secret::new(ENCRYPTED));
    let events = events_of(cx, &protected, |field, window, cx| {
        field.import("made", "/keys/id".into(), window, cx)
    });
    assert!(matches!(
        events.as_slice(),
        [VaultKeyEvent::PassphraseAsked, VaultKeyEvent::Failed(_)]
    ));
}
