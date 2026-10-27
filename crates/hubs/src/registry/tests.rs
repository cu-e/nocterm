use std::time::Duration;

use gpui::TestAppContext;
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp};
use nocterm_hubs_model::{Fingerprint, HubError, HubRecord, TrustPolicy};

use crate::fixture::{address, fixture, id, pinned_record, server};
use crate::{HubStatus, Load};

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
        .update(cx, |hubs, cx| hubs.window(&id(), cx))
        .expect("a known hub");
    let second = fx
        .hubs
        .update(cx, |hubs, cx| hubs.window(&id(), cx))
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
            .update(cx, |hubs, cx| hubs.window(&id(), cx))
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
        .update(cx, |hubs, cx| hubs.window(&id(), cx))
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
        .update(cx, |hubs, cx| hubs.window(&id(), cx))
        .unwrap();
    cx.run_until_parked();
    fx.hub
        .edit_server(&address(), |server| server.online = false);
    window.update(cx, |window, cx| window.refresh(cx));
    cx.run_until_parked();
    assert!(matches!(fx.status(cx), Some(HubStatus::Offline { .. })));
}
