use super::*;
use gpui_kit::TestAppContext;
use nocterm_session::SecretRequest;
use nocterm_vault::testing::ScriptedVaultAccess;

fn run<T>(cx: &TestAppContext, future: impl std::future::Future<Output = T>) -> T {
    cx.foreground_executor().block_test(future)
}

/// The binding a password prompt for `target` asks the vault for.
fn asked(target: &Target) -> CredentialBinding {
    CredentialBinding::for_request(&SecretRequest::Password {
        target: target.clone(),
        retry: false,
    })
    .unwrap()
}

fn unlocked(cx: &TestAppContext) -> (Arc<ScriptedVaultAccess>, Arc<dyn SavedPasswords>) {
    let vault = Arc::new(ScriptedVaultAccess::default());
    run(
        cx,
        vault.create(Secret::new("test unique master passphrase")),
    )
    .unwrap();
    let access: Arc<dyn VaultAccess> = vault.clone();
    (vault, passwords_of(&access, VaultPlace::Account))
}

#[gpui_kit::test]
fn a_typed_password_answers_its_address_and_replaces_the_saved_one_in_place(
    cx: &mut TestAppContext,
) {
    let vault = Arc::new(ScriptedVaultAccess::default());
    let access: Arc<dyn VaultAccess> = vault.clone();
    let passwords = passwords_of(&access, VaultPlace::Account);
    assert_eq!(passwords.place(), VaultPlace::Account);
    let target = Target::new("ops", "prod.example", 22);
    let locked = run(
        cx,
        passwords.keep(None, "prod".into(), target.clone(), Secret::new("first")),
    );
    assert!(matches!(locked, Err(PasswordNotKept::Locked)));

    run(
        cx,
        vault.create(Secret::new("test unique master passphrase")),
    )
    .unwrap();
    let id = run(
        cx,
        passwords.keep(None, "prod".into(), target.clone(), Secret::new("first")),
    )
    .unwrap();
    let again = run(
        cx,
        passwords.keep(
            Some(id),
            "other name".into(),
            target.clone(),
            Secret::new("second"),
        ),
    )
    .unwrap();
    assert_eq!(again, id);
    let secret = run(cx, vault.get(id, asked(&target))).unwrap();
    assert_eq!(secret.expose(), "second");
    let other = Target::new("ops", "other.example", 22);
    assert!(run(cx, vault.get(id, asked(&other))).is_err());
}

#[gpui_kit::test]
fn a_saved_password_signs_in_to_every_server_it_is_attached_to(cx: &mut TestAppContext) {
    let (vault, passwords) = unlocked(cx);
    let (a, b) = (
        Target::new("ops", "a.example", 22),
        Target::new("ops", "b.example", 22),
    );
    // Saved under a name for no server yet, as the vault page adds one.
    let id = run(
        cx,
        vault.put(
            None,
            "Office".into(),
            CredentialBinding::password([]),
            Secret::new("shared"),
        ),
    )
    .unwrap();
    run(
        cx,
        vault.put(
            None,
            "key".into(),
            CredentialBinding::PrivateKey {
                public_key: "ssh-ed25519 AAAA".into(),
            },
            Secret::new("k"),
        ),
    )
    .unwrap();
    let listed = |cx: &TestAppContext| run(cx, passwords.list()).unwrap();
    assert_eq!(
        listed(cx),
        vec![SavedPassword {
            id,
            label: "Office".into(),
            servers: 0
        }],
        "only passwords are offered"
    );
    run(cx, passwords.attach(id, a.clone())).unwrap();
    run(cx, passwords.attach(id, b.clone())).unwrap();
    run(cx, passwords.attach(id, a.clone())).unwrap();
    assert_eq!(listed(cx)[0].servers, 2);
    for target in [&a, &b] {
        assert_eq!(
            run(cx, vault.get(id, asked(target))).unwrap().expose(),
            "shared"
        );
    }
    // A new password for one of them changes it for both, under its own name.
    let c = Target::new("ops", "c.example", 22);
    run(
        cx,
        passwords.keep(Some(id), "c".into(), c.clone(), Secret::new("rotated")),
    )
    .unwrap();
    assert_eq!(listed(cx)[0].label, "Office");
    assert_eq!(listed(cx)[0].servers, 3);
    assert_eq!(
        run(cx, vault.get(id, asked(&b))).unwrap().expose(),
        "rotated"
    );
    let gone = CredentialId::from_bytes([7; 16]);
    assert!(matches!(
        run(cx, passwords.attach(gone, a)),
        Err(PasswordNotKept::Missing)
    ));
}
