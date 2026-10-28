use std::time::Duration;

use gpui::TestAppContext;
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp};
use nocterm_hubs_model::{
    AdminKeyError, Fingerprint, HubColor, HubError, HubRecord, PendingAdminKey, TrustPolicy,
};

use crate::fixture::{
    address, fixture, fixture_with_known_failing_settings, id, pinned_record, server,
};
use crate::{HubStatus, HubsError, Load};

fn signed_in_hub() -> FakeHub {
    let hub = FakeHub::new().with_server(server());
    hub.sign_in_as(&id(), "ada");
    hub
}

#[gpui::test]
async fn a_known_hub_connects_and_goes_online(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    assert_eq!(fx.status(cx), Some(HubStatus::Connecting));
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
    assert_eq!(fx.hub.count(FakeOp::Connect), 1);
}

#[gpui::test]
async fn a_hub_without_credentials_is_signed_out(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server());
    let fx = fixture(cx, hub, vec![pinned_record()]);
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::SignedOut));
}

#[gpui::test]
async fn an_unreachable_hub_is_retried_with_growing_delays_until_it_answers(
    cx: &mut TestAppContext,
) {
    let hub = signed_in_hub();
    hub.edit_server(&address(), |server| server.online = false);
    let fx = fixture(cx, hub, vec![pinned_record()]);
    cx.run_until_parked();
    assert!(matches!(fx.status(cx), Some(HubStatus::Offline { .. })));
    assert_eq!(fx.hub.count(FakeOp::Connect), 1);

    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 2);

    // The second wait is five seconds, so two more do not reach it.
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 2);

    fx.hub
        .edit_server(&address(), |server| server.online = true);
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
}

#[gpui::test]
async fn a_changed_certificate_is_a_trust_failure_that_is_never_retried(cx: &mut TestAppContext) {
    let hub = signed_in_hub();
    hub.edit_server(&address(), |server| {
        server.certificate = Fingerprint::from_bytes([9; 32]);
    });
    let fx = fixture(cx, hub, vec![pinned_record()]);
    cx.run_until_parked();
    assert_eq!(
        fx.status(cx),
        Some(HubStatus::TrustFailed {
            pinned: Some(Fingerprint::from_bytes([7; 32])),
            presented: Fingerprint::from_bytes([9; 32]),
        })
    );
    cx.executor().advance_clock(Duration::from_secs(300));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 1);
}

#[gpui::test]
async fn a_revoked_device_loses_its_credentials_and_is_not_retried(cx: &mut TestAppContext) {
    let hub = signed_in_hub();
    hub.revoke_device(&id());
    let fx = fixture(cx, hub, vec![pinned_record()]);
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Revoked));
    assert!(!fx.hub.is_signed_in(&id()));
    cx.executor().advance_clock(Duration::from_secs(300));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 1);
}

#[gpui::test]
async fn a_hub_with_another_protocol_asks_for_an_upgrade(cx: &mut TestAppContext) {
    let hub = signed_in_hub();
    hub.edit_server(&address(), |server| server.protocol = 2);
    let fx = fixture(cx, hub, vec![pinned_record()]);
    cx.run_until_parked();
    assert_eq!(
        fx.status(cx),
        Some(HubStatus::UpgradeRequired { hub: 2, client: 1 })
    );
}

#[gpui::test]
async fn windows_of_one_hub_share_one_connection_and_one_state(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let first = fx
        .hubs
        .update(cx, |hubs, cx| hubs.session_lists(&id(), cx))
        .expect("a known hub");
    let second = fx
        .hubs
        .update(cx, |hubs, cx| hubs.session_lists(&id(), cx))
        .expect("a known hub");
    assert_eq!(first, second);
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 1);
    first.read_with(cx, |window, _| {
        assert!(matches!(window.load(), Load::Loaded));
    });
}

#[gpui::test]
async fn an_unknown_hub_has_no_window_status_or_removal(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), Vec::new());
    assert!(
        fx.hubs
            .update(cx, |hubs, cx| hubs.session_lists(&id(), cx))
            .is_none()
    );
    assert_eq!(fx.status(cx), None);
    let removed = fx.hubs.update(cx, |hubs, cx| hubs.remove(&id(), cx));
    assert!(removed.await.is_err());
}

#[gpui::test]
async fn a_server_added_to_the_settings_starts_connecting(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), Vec::new());
    cx.update(|cx| {
        fx.settings
            .update::<nocterm_hubs_model::HubsSettings>(cx, |settings| {
                settings.upsert(pinned_record());
            })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
}

#[gpui::test]
async fn removing_a_server_forgets_its_credentials_and_drops_it_from_the_list(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    fx.hubs
        .update(cx, |hubs, cx| hubs.remove(&id(), cx))
        .await
        .unwrap();
    cx.run_until_parked();
    assert!(fx.servers(cx).is_empty());
    assert_eq!(fx.status(cx), None);
    assert!(fx.hub.calls().contains(&FakeCall::Forget(id())));
    assert!(!fx.hub.is_signed_in(&id()));
}

#[gpui::test]
async fn signing_out_keeps_the_server_listed_but_signed_out(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    fx.hubs
        .update(cx, |hubs, cx| hubs.sign_out(&id(), cx))
        .await
        .unwrap();
    assert_eq!(fx.status(cx), Some(HubStatus::SignedOut));
    assert_eq!(fx.servers(cx).len(), 1);
    assert!(!fx.hub.is_signed_in(&id()));
}

#[gpui::test]
async fn a_changed_trust_in_the_settings_connects_again(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let changed = HubRecord::new(address(), TrustPolicy::PublicCa, "Home");
    cx.update(|cx| {
        fx.settings
            .update::<nocterm_hubs_model::HubsSettings>(cx, move |s| s.upsert(changed))
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 2);
    // The fake server is not publicly trusted, so the new trust fails and stays failed.
    assert!(matches!(
        fx.status(cx),
        Some(HubStatus::TrustFailed { pinned: None, .. })
    ));
}

#[gpui::test]
async fn a_hub_that_loses_the_session_while_online_is_connected_again(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let window = fx
        .hubs
        .update(cx, |hubs, cx| hubs.session_lists(&id(), cx))
        .unwrap();
    cx.run_until_parked();
    fx.hub.fail_next(FakeOp::Nodes, HubError::NotSignedIn);
    window.update(cx, |window, cx| window.refresh(cx));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Connect), 2);
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
}

#[gpui::test]
async fn a_hub_that_stops_answering_while_online_goes_offline(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let window = fx
        .hubs
        .update(cx, |hubs, cx| hubs.session_lists(&id(), cx))
        .unwrap();
    cx.run_until_parked();
    fx.hub
        .edit_server(&address(), |server| server.online = false);
    window.update(cx, |window, cx| window.refresh(cx));
    cx.run_until_parked();
    assert!(matches!(fx.status(cx), Some(HubStatus::Offline { .. })));
}

fn revision(fx: &crate::fixture::Fixture, cx: &mut TestAppContext) -> u64 {
    cx.read(|cx| fx.settings.revision(cx))
}

#[gpui::test]
async fn renaming_a_server_is_one_settings_edit_and_shows_in_the_list(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let before = revision(&fx, cx);

    fx.hubs
        .update(cx, |hubs, cx| hubs.set_label(&id(), "  Office\n", cx))
        .await
        .unwrap();
    cx.run_until_parked();

    assert_eq!(revision(&fx, cx), before + 1);
    assert_eq!(fx.servers(cx)[0].label, "Office");
    let label = fx
        .hubs
        .read_with(cx, |hubs, _| hubs.summary(&id()).unwrap().label);
    assert_eq!(label, "Office");
    assert_eq!(
        fx.hub.count(FakeOp::Connect),
        1,
        "no new connection for a name"
    );
}

#[gpui::test]
async fn an_empty_name_shows_the_address_again(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    fx.hubs
        .update(cx, |hubs, cx| hubs.set_label(&id(), "   ", cx))
        .await
        .unwrap();
    cx.run_until_parked();
    let label = fx
        .hubs
        .read_with(cx, |hubs, _| hubs.summary(&id()).unwrap().label);
    assert_eq!(label, address().to_string());
}

#[gpui::test]
async fn colouring_a_server_is_one_settings_edit_and_can_be_undone(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let before = revision(&fx, cx);

    fx.hubs
        .update(cx, |hubs, cx| {
            hubs.set_color(&id(), Some(HubColor::Green), cx)
        })
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(revision(&fx, cx), before + 1);
    let color = fx
        .hubs
        .read_with(cx, |hubs, _| hubs.summary(&id()).unwrap().color);
    assert_eq!(color, Some(HubColor::Green));
    assert_eq!(fx.status(cx), Some(HubStatus::Online), "still connected");

    fx.hubs
        .update(cx, |hubs, cx| hubs.set_color(&id(), None, cx))
        .await
        .unwrap();
    assert_eq!(fx.servers(cx)[0].color, None);
}

#[gpui::test]
async fn editing_an_unknown_server_fails_and_changes_nothing(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), Vec::new());
    let before = revision(&fx, cx);
    let renamed = fx
        .hubs
        .update(cx, |hubs, cx| hubs.set_label(&id(), "x", cx));
    let colored = fx.hubs.update(cx, |hubs, cx| {
        hubs.set_color(&id(), Some(HubColor::Red), cx)
    });
    assert!(matches!(renamed.await, Err(HubsError::Unknown(_))));
    assert!(matches!(colored.await, Err(HubsError::Unknown(_))));
    assert_eq!(revision(&fx, cx), before);
}

#[gpui::test]
async fn a_failed_save_leaves_the_name_and_colour_as_they_were(cx: &mut TestAppContext) {
    let fx = fixture_with_known_failing_settings(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let renamed = fx
        .hubs
        .update(cx, |hubs, cx| hubs.set_label(&id(), "Office", cx));
    assert!(matches!(renamed.await, Err(HubsError::Save(_))));
    let colored = fx.hubs.update(cx, |hubs, cx| {
        hubs.set_color(&id(), Some(HubColor::Cyan), cx)
    });
    assert!(matches!(colored.await, Err(HubsError::Save(_))));
    cx.run_until_parked();
    assert_eq!(fx.servers(cx), vec![pinned_record()]);
}

#[gpui::test]
async fn signing_out_erases_the_credentials_and_keeps_name_and_colour(cx: &mut TestAppContext) {
    let marked = pinned_record().with_color(Some(HubColor::Magenta));
    let fx = fixture(cx, signed_in_hub(), vec![marked.clone()]);
    cx.run_until_parked();
    fx.hubs
        .update(cx, |hubs, cx| hubs.sign_out(&id(), cx))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::SignedOut));
    assert_eq!(fx.servers(cx), vec![marked]);
    assert!(fx.hub.calls().contains(&FakeCall::SignOut(id())));
    assert!(!fx.hub.is_signed_in(&id()));
}

#[gpui::test]
async fn a_server_that_cannot_be_removed_from_the_settings_shows_as_signed_out(
    cx: &mut TestAppContext,
) {
    let fx = fixture_with_known_failing_settings(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Online));

    let removed = fx.hubs.update(cx, |hubs, cx| hubs.remove(&id(), cx));
    assert!(matches!(removed.await, Err(HubsError::Save(_))));
    cx.run_until_parked();

    assert_eq!(fx.servers(cx), vec![pinned_record()], "still listed");
    assert!(!fx.hub.is_signed_in(&id()), "the credentials are gone");
    assert_eq!(
        fx.status(cx),
        Some(HubStatus::SignedOut),
        "the list does not claim a connection that cannot work"
    );
}

#[gpui::test]
async fn credentials_that_cannot_be_erased_keep_the_server_listed(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    fx.hub.fail_next(FakeOp::Forget, HubError::NotSignedIn);
    let removed = fx.hubs.update(cx, |hubs, cx| hubs.remove(&id(), cx));
    assert!(matches!(removed.await, Err(HubsError::Hub(_))));
    cx.run_until_parked();
    assert_eq!(fx.servers(cx), vec![pinned_record()]);
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
}

#[gpui::test]
async fn the_account_server_is_chosen_in_one_edit_and_cleared_when_it_is_removed(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    assert_eq!(fx.hubs.read_with(cx, |hubs, cx| hubs.account_hub(cx)), None);
    let before = revision(&fx, cx);

    fx.hubs
        .update(cx, |hubs, cx| hubs.set_account(Some(&id()), cx))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(revision(&fx, cx), before + 1);
    assert_eq!(
        fx.hubs.read_with(cx, |hubs, cx| hubs.account_hub(cx)),
        Some(id())
    );

    fx.hubs
        .update(cx, |hubs, cx| hubs.remove(&id(), cx))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fx.hubs.read_with(cx, |hubs, cx| hubs.account_hub(cx)), None);
}

#[gpui::test]
async fn an_unknown_server_cannot_become_the_account_server(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), Vec::new());
    let before = revision(&fx, cx);
    let result = fx
        .hubs
        .update(cx, |hubs, cx| hubs.set_account(Some(&id()), cx))
        .await;
    assert!(matches!(result, Err(HubsError::Unknown(_))));
    assert_eq!(revision(&fx, cx), before);
}

fn grace_of_acme(fingerprint: &str) -> PendingAdminKey {
    PendingAdminKey {
        organization: "Acme".to_owned(),
        admin: "Grace".to_owned(),
        account_id: "grace".to_owned(),
        fingerprint: fingerprint.to_owned(),
        replaces: None,
    }
}

#[gpui::test]
async fn a_pending_admin_key_is_listed_for_a_known_hub_and_confirming_it_redraws_observers(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    cx.run_until_parked();
    let redraws = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = redraws.clone();
    let _observed =
        cx.update(|cx| cx.observe(&fx.hubs, move |_, _| counted.set(counted.get() + 1)));

    // A synchronization found an administrator nobody confirmed: the list follows at once.
    fx.admin_keys.offer(&id(), grace_of_acme("aaaa bbbb"));
    cx.run_until_parked();
    assert!(redraws.get() > 0);
    let listed = fx
        .hubs
        .read_with(cx, |hubs, _| hubs.pending_admin_keys(&id()));
    assert_eq!(listed, [grace_of_acme("aaaa bbbb")]);

    let before = redraws.get();
    let confirmed = fx.hubs.update(cx, |hubs, cx| {
        hubs.confirm_admin_key(&id(), "grace", "AAAA-BBBB", cx)
    });
    cx.run_until_parked();
    assert_eq!(confirmed, Ok(()));
    assert!(redraws.get() > before);
    let listed = fx
        .hubs
        .read_with(cx, |hubs, _| hubs.pending_admin_keys(&id()));
    assert!(listed.is_empty());
}

#[gpui::test]
async fn a_key_that_is_not_the_one_shown_is_not_confirmed(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), vec![pinned_record()]);
    fx.admin_keys.offer(&id(), grace_of_acme("aaaa"));

    let refused = fx.hubs.update(cx, |hubs, cx| {
        hubs.confirm_admin_key(&id(), "grace", "ffff", cx)
    });

    assert_eq!(refused, Err(AdminKeyError::Mismatch));
    assert!(fx.admin_keys.confirmed().is_empty());
}

#[gpui::test]
async fn a_hub_that_is_not_listed_has_no_pending_admin_keys(cx: &mut TestAppContext) {
    let fx = fixture(cx, signed_in_hub(), Vec::new());
    fx.admin_keys.offer(&id(), grace_of_acme("aaaa"));

    let listed = fx
        .hubs
        .read_with(cx, |hubs, _| hubs.pending_admin_keys(&id()));

    assert!(listed.is_empty());
}
