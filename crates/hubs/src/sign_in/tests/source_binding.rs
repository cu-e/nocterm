use super::*;

#[gpui::test]
async fn an_earlier_recovery_card_cannot_acknowledge_the_replacement_accounts_key(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    let session = at_methods(cx, &fx);
    password(cx, &session, "ada", "secret");
    assert!(matches!(step(cx, &session), SignInStep::RecoveryKey { .. }));
    fx.hub.sign_in_as(&id(), "another account");
    fx.hub.leave_recovery_key_unconfirmed(&id());
    session.update(cx, |session, cx| session.acknowledge_recovery_key(cx));
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Failed(SignInError::Hub(HubError::Stale))
    ));
    assert!(fx.servers(cx).is_empty());
    assert!(fx.hub.recovery_key_unconfirmed(&id()));
}

#[gpui::test]
async fn a_failed_delayed_settings_write_never_discards_a_replacement_login(
    cx: &mut TestAppContext,
) {
    for replacements in [vec!["ada"], vec!["bob", "ada"]] {
        let (writer, release, writes) = nocterm_store::test_support::gated_writer().into_parts();
        let fx = crate::fixture::fixture_with_writer(cx, hub(), Vec::new(), writer);
        let session = at_methods(cx, &fx);
        password(cx, &session, "ada", "secret");
        cx.run_until_parked();
        assert!(matches!(step(cx, &session), SignInStep::Saving));
        assert_eq!(writes.lock().unwrap().len(), 1);
        for account in replacements {
            fx.hub.sign_in_as(&id(), account);
        }
        release
            .send(Err(nocterm_store::StoreError::backend(
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            )))
            .unwrap();
        cx.run_until_parked();
        assert!(matches!(
            step(cx, &session),
            SignInStep::Failed(SignInError::Hub(HubError::Stale))
        ));
        assert!(fx.hub.is_signed_in(&id()));
        assert!(!fx.hub.calls().contains(&FakeCall::Forget(id())));
        assert!(fx.servers(cx).is_empty());
    }
}
