//! The Servers page lists the known servers and manages them through the service.
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::{ServersPage, item, settings_page};
use crate::test_support::{Fixture, fixture_over, hub_id, pinned_record, server};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_hubs::{HubStatus, HubSummary};
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp, FakeServer};
use nocterm_hubs_model::{
    DeviceSummary, Environment, Fingerprint, HubAdminKeys as _, HubColor, HubError, HubRecord,
    HubSignIn as _, PendingAdminKey, TrustPolicy,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SnapshotWriter, StoreError};

const KEY: &str = "hub.home:7443";
const OTHER: &str = "hub.work:7443";

const OVERVIEW: usize = 0;
const DEVICES: usize = 1;
const KEYS: usize = 2;

/// Opens the tab `index` of the selected server.
fn open_tab(page: &Fixture, index: usize, cx: &mut TestAppContext) {
    page.render(cx);
    cx.update_window(page.window, |_, window, cx| {
        window.within("servers-tabs").click(index, cx);
    })
    .unwrap();
    cx.run_until_parked();
    page.render(cx);
}

/// The page in a window of its own, over a hub that `signed_in` names a user of.
fn page_over(
    cx: &mut TestAppContext,
    config: SettingsConfig,
    known: Vec<HubRecord>,
    signed_in: Option<&str>,
) -> Fixture {
    page_on(
        cx,
        config,
        FakeHub::new().with_server(server()),
        known,
        signed_in,
    )
}

fn page_on(
    cx: &mut TestAppContext,
    config: SettingsConfig,
    hub: FakeHub,
    known: Vec<HubRecord>,
    signed_in: Option<&str>,
) -> Fixture {
    let setup = fixture_over(cx, config, Environment::Local, hub, known, signed_in);
    let hubs = setup.installed.hubs().clone();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| ServersPage::new(hubs, None, cx))
        })
        .unwrap()
    });
    let page = Fixture { window, ..setup };
    page.render(cx);
    page
}

fn page(cx: &mut TestAppContext, known: Vec<HubRecord>) -> Fixture {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    page_over(cx, config, known, Some("ada"))
}

fn servers(page: &Fixture, cx: &mut TestAppContext) -> Vec<HubSummary> {
    cx.update(|cx| page.installed.hubs().read(cx).list())
}

/// How many times the settings were published.
fn revision(page: &Fixture, cx: &mut TestAppContext) -> u64 {
    cx.update(|cx| page.installed.hubs().read(cx).settings().revision(cx))
}

fn failing_config() -> SettingsConfig {
    let writer: SnapshotWriter<SettingsDocument> = Arc::new(|_| {
        Box::pin(async {
            Err(StoreError::backend(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            )))
        })
    });
    SettingsConfig::with_writer(SettingsDocument::default(), writer)
}

fn notices(page: &Fixture, cx: &mut TestAppContext) -> usize {
    cx.update_window(page.window, |_, window, cx| {
        nocterm_ui::notice::count(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn without_servers_the_page_invites_signing_in(cx: &mut TestAppContext) {
    let page = page(cx, Vec::new());
    assert!(page.shows("servers-empty", cx));
    assert!(page.shows("servers-add", cx));
}

#[gpui_kit::test]
fn the_page_is_the_servers_page_of_the_settings(cx: &mut TestAppContext) {
    let page = page(cx, Vec::new());
    let spec = settings_page(&page.installed);
    assert_eq!(spec.id, "servers");
    assert_eq!(spec.title.as_ref(), "Servers");
}

#[gpui_kit::test]
fn a_pinned_server_shows_its_fingerprint(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    assert!(page.shows(item("server", KEY), cx));
    assert!(page.shows(item("server-pinned", KEY), cx));
    assert!(!page.shows(item("server-public-ca", KEY), cx));
    assert!(!page.shows(item("server-presented", KEY), cx));
}

#[gpui_kit::test]
fn a_server_trusted_through_a_public_authority_says_nothing_is_pinned(cx: &mut TestAppContext) {
    let record = HubRecord::new(pinned_record().address, TrustPolicy::PublicCa, "Home");
    let page = page(cx, vec![record]);
    assert!(page.shows(item("server-public-ca", KEY), cx));
    assert!(!page.shows(item("server-pinned", KEY), cx));
}

#[gpui_kit::test]
fn a_changed_certificate_is_shown_and_removing_is_the_only_way_out(cx: &mut TestAppContext) {
    let changed = HubRecord::new(
        pinned_record().address,
        TrustPolicy::Pinned(Fingerprint::from_bytes([9; 32])),
        "Home",
    );
    let page = page(cx, vec![changed.clone()]);
    assert!(matches!(
        servers(&page, cx)[0].status,
        HubStatus::TrustFailed { .. }
    ));
    assert!(page.shows(item("server-pinned", KEY), cx));
    assert!(page.shows(item("server-presented", KEY), cx));
    assert!(page.shows(item("server-remove", KEY), cx));
    assert!(!page.shows(item("server-sign-out", KEY), cx));
    assert!(!page.shows("sign-in-trust", cx), "nothing to accept");
    assert_eq!(
        cx.update(|cx| page.installed.hubs().read(cx).record(&hub_id()).cloned()),
        Some(changed),
        "the record is as it was"
    );
}

#[gpui_kit::test]
fn a_name_typed_in_the_row_is_saved_with_enter(cx: &mut TestAppContext) {
    let page = page(cx, vec![server().pinned_record("")]);
    page.click(item("server-name", KEY), cx);
    page.type_text("Office", cx);
    page.press("enter", cx);
    cx.run_until_parked();
    let label = cx.update(|cx| page.installed.label(&hub_id(), cx));
    assert_eq!(label.as_deref(), Some("Office"));
}

#[gpui_kit::test]
fn a_name_that_was_not_edited_never_overwrites_a_newer_one(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    page.render(cx);
    // The name changes elsewhere, for instance in another window.
    page.installed
        .hubs()
        .update(cx, |hubs, cx| hubs.set_label(&hub_id(), "Elsewhere", cx))
        .detach();
    cx.run_until_parked();

    page.click(item("server-name", KEY), cx);
    page.press("tab", cx);
    cx.run_until_parked();

    let label = cx.update(|cx| page.installed.label(&hub_id(), cx));
    assert_eq!(label.as_deref(), Some("Elsewhere"));
}

#[gpui_kit::test]
fn a_name_is_saved_once_when_enter_is_followed_by_leaving_the_field(cx: &mut TestAppContext) {
    let page = page(cx, vec![server().pinned_record("")]);
    page.click(item("server-name", KEY), cx);
    page.type_text("Office", cx);
    let before = revision(&page, cx);
    page.press("enter", cx);
    page.press("tab", cx);
    cx.run_until_parked();

    assert_eq!(revision(&page, cx), before + 1);
}

#[gpui_kit::test]
fn a_cleared_name_shows_the_address_again(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    page.click(item("server-name", KEY), cx);
    page.press("ctrl-a", cx);
    page.press("backspace", cx);
    page.press("enter", cx);
    cx.run_until_parked();
    let label = cx.update(|cx| page.installed.label(&hub_id(), cx));
    assert_eq!(label.as_deref(), Some(KEY));
}

#[gpui_kit::test]
fn choosing_a_colour_marks_the_server_and_none_clears_the_mark(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    page.click(item("server-color-blue", KEY), cx);
    cx.run_until_parked();
    assert_eq!(servers(&page, cx)[0].color, Some(HubColor::Blue));
    page.click(item("server-color-none", KEY), cx);
    cx.run_until_parked();
    assert_eq!(servers(&page, cx)[0].color, None);
}

#[gpui_kit::test]
fn signing_out_keeps_the_server_listed_and_offers_no_second_sign_out(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    assert!(page.shows(item("server-sign-out", KEY), cx));
    page.click(item("server-sign-out", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    let listed = servers(&page, cx);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, HubStatus::SignedOut);
    assert!(!page.hub.is_signed_in(&hub_id()));
    assert!(!page.shows(item("server-sign-out", KEY), cx));
    assert!(page.shows(item("server-remove", KEY), cx));
}

#[gpui_kit::test]
fn a_server_that_is_not_signed_in_has_no_sign_out(cx: &mut TestAppContext) {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_over(cx, config, vec![pinned_record()], None);
    assert!(!page.shows(item("server-sign-out", KEY), cx));
}

#[gpui_kit::test]
fn keeping_a_server_at_the_confirmation_changes_nothing(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    page.click(item("server-remove", KEY), cx);
    page.settle_dialog(cx);
    assert!(page.has_dialog(cx), "removal asks first");
    assert!(servers(&page, cx).len() == 1);
    page.click("remove-server-keep", cx);
    cx.run_until_parked();
    assert_eq!(servers(&page, cx).len(), 1);
    assert!(page.hub.is_signed_in(&hub_id()));
}

#[gpui_kit::test]
fn confirming_removes_the_server_and_erases_its_credentials(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    page.click(item("server-remove", KEY), cx);
    page.settle_dialog(cx);
    page.click("remove-server-confirm", cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(servers(&page, cx).is_empty());
    assert!(!page.hub.is_signed_in(&hub_id()));
    assert!(page.hub.calls().contains(&FakeCall::Forget(hub_id())));
    assert!(!page.shows(item("server", KEY), cx));
    assert!(page.shows("servers-empty", cx));
}

#[gpui_kit::test]
fn a_change_that_cannot_be_saved_is_reported(cx: &mut TestAppContext) {
    let page = page_over(cx, failing_config(), vec![pinned_record()], Some("ada"));
    page.click(item("server-color-red", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert_eq!(notices(&page, cx), 1);
    assert_eq!(servers(&page, cx)[0].color, None, "the colour was not kept");
}

#[gpui_kit::test]
fn a_server_whose_recovery_key_was_never_confirmed_offers_a_new_one(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "secret", "recovery-ada"));
    hub.leave_recovery_key_unconfirmed(&hub_id());
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_on(cx, config, hub, vec![pinned_record()], Some("ada"));
    open_tab(&page, KEYS, cx);
    assert!(page.shows(item("server-recovery-issue", KEY), cx));

    page.click(item("server-recovery-issue", KEY), cx);
    page.settle_dialog(cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(page.shows("sign-in-key-saved", cx));
    assert_eq!(page.hub.count(FakeOp::ReissueRecoveryKey), 1);

    page.click("sign-in-key-saved", cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(
        !page.shows(item("server-recovery-issue", KEY), cx),
        "a saved key is not offered again"
    );
}

#[gpui_kit::test]
fn a_server_with_a_confirmed_recovery_key_offers_no_new_one(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    open_tab(&page, KEYS, cx);
    assert!(!page.shows(item("server-recovery-issue", KEY), cx));
}

/// Counts how often the page asks to be drawn again.
fn count_redraws(page: &Fixture, cx: &mut TestAppContext) -> Rc<Cell<usize>> {
    let redraws = Rc::new(Cell::new(0));
    let counted = Rc::clone(&redraws);
    cx.update_window(page.window, |root, _, cx| {
        let servers = root
            .downcast::<gpui_kit::base::Root>()
            .unwrap()
            .read(cx)
            .view()
            .clone()
            .downcast::<ServersPage>()
            .unwrap();
        cx.observe(&servers, move |_, _| counted.set(counted.get() + 1))
            .detach();
    })
    .unwrap();
    redraws
}

#[gpui_kit::test]
fn the_offer_of_a_new_recovery_key_appears_as_soon_as_the_vault_is_unlocked(
    cx: &mut TestAppContext,
) {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "secret", "recovery-ada"));
    hub.leave_recovery_key_unconfirmed(&hub_id());
    hub.lock_keys(&hub_id());
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_on(cx, config, hub.clone(), vec![pinned_record()], Some("ada"));
    open_tab(&page, KEYS, cx);
    assert!(
        !page.shows(item("server-recovery-issue", KEY), cx),
        "a locked vault offers nothing"
    );
    let redraws = count_redraws(&page, cx);

    hub.unlock_keys(&hub_id());
    cx.run_until_parked();

    assert!(redraws.get() > 0, "the page is redrawn by the unlock alone");
    page.render(cx);
    assert!(page.shows(item("server-recovery-issue", KEY), cx));
}

fn grace(fingerprint: &str, replaces: Option<&str>) -> PendingAdminKey {
    PendingAdminKey {
        organization: "Acme".to_owned(),
        admin: "Grace Hopper".to_owned(),
        account_id: "grace".to_owned(),
        fingerprint: fingerprint.to_owned(),
        replaces: replaces.map(str::to_owned),
    }
}

const GRACE_KEY: &str = "AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-0000-1111-2222-3333";

/// The page over a signed-in server, with Grace's key waiting for confirmation.
fn page_with_pending_key(cx: &mut TestAppContext, replaces: Option<&str>) -> Fixture {
    let page = page(cx, vec![pinned_record()]);
    open_tab(&page, KEYS, cx);
    page.admin_keys.offer(&hub_id(), grace(GRACE_KEY, replaces));
    cx.run_until_parked();
    page.render(cx);
    page
}

#[gpui_kit::test]
fn a_server_with_no_pending_administrator_key_shows_no_confirmation(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    open_tab(&page, KEYS, cx);
    assert!(!page.shows(item("server-admin-keys", KEY), cx));
    assert!(page.shows(item("server-keys-settled", KEY), cx));
}

#[gpui_kit::test]
fn a_pending_administrator_key_appears_with_its_fingerprint_as_soon_as_it_is_found(
    cx: &mut TestAppContext,
) {
    let page = page(cx, vec![pinned_record()]);
    open_tab(&page, KEYS, cx);
    assert!(!page.shows(item("server-admin-keys", KEY), cx));

    page.admin_keys.offer(&hub_id(), grace(GRACE_KEY, None));
    cx.run_until_parked();
    page.render(cx);

    assert!(page.shows(item("server-admin-key-0", KEY), cx));
    assert!(page.shows(item("server-admin-confirm-0", KEY), cx));
    assert!(!page.shows(item("server-admin-changed-0", KEY), cx));
}

#[test]
fn the_fingerprint_is_grouped_for_reading() {
    assert_eq!(
        super::tabs::admin_keys::grouped(GRACE_KEY),
        "AAAA BBBB CCCC DDDD EEEE FFFF 0000 1111\n2222 3333"
    );
    assert_eq!(super::tabs::admin_keys::grouped("short"), "short");
}

#[gpui_kit::test]
fn confirming_the_shown_key_removes_it_from_the_list_and_tells_the_account(
    cx: &mut TestAppContext,
) {
    let page = page_with_pending_key(cx, None);
    let seen = page.hub.keys_revision();

    page.click(item("server-admin-confirm-0", KEY), cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(!page.shows(item("server-admin-keys", KEY), cx));
    let confirmed = page.admin_keys.confirmed();
    assert_eq!(confirmed.len(), 1);
    assert_eq!(confirmed[0].1.account_id, "grace");
    assert_eq!(confirmed[0].1.fingerprint, GRACE_KEY);
    assert_ne!(
        page.hub.keys_revision(),
        seen,
        "the replica is told to read again"
    );
}

#[gpui_kit::test]
fn a_changed_administrator_key_is_shown_with_a_warning_and_both_fingerprints(
    cx: &mut TestAppContext,
) {
    let old = "9999-8888-7777-6666-5555-4444-3333-2222-1111-0000";
    let page = page_with_pending_key(cx, Some(old));

    assert!(page.shows(item("server-admin-changed-0", KEY), cx));
    assert!(page.shows(item("server-admin-before-0", KEY), cx));
    assert!(page.shows(item("server-admin-fingerprint-0", KEY), cx));
    assert!(page.shows(item("server-admin-confirm-0", KEY), cx));

    page.click(item("server-admin-confirm-0", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(!page.shows(item("server-admin-keys", KEY), cx));
}

#[gpui_kit::test]
fn a_key_the_account_no_longer_offers_is_reported_and_stays_unconfirmed(cx: &mut TestAppContext) {
    let page = page_with_pending_key(cx, None);
    let shown = page.admin_keys.pending_admin_keys(&hub_id())[0].clone();
    // The key on offer changed after the person read the fingerprint of the old one.
    page.admin_keys.offer(&hub_id(), grace("FFFF-FFFF", None));
    let before = notices(&page, cx);
    let hubs = page.installed.hubs().clone();

    cx.update_window(page.window, |_, window, cx| {
        crate::manage::confirm_admin_key(&hubs, &shown, GRACE_KEY, window, cx);
    })
    .unwrap();

    assert_eq!(notices(&page, cx), before + 1);
    assert!(page.admin_keys.confirmed().is_empty());
}

#[gpui_kit::test]
fn closed_account_keys_hide_the_pending_administrators(cx: &mut TestAppContext) {
    let page = page_with_pending_key(cx, None);
    assert!(page.shows(item("server-admin-keys", KEY), cx));

    page.hub.lock_keys(&hub_id());
    cx.run_until_parked();
    page.render(cx);

    assert!(!page.shows(item("server-admin-keys", KEY), cx));
}

fn two_servers() -> (FakeHub, Vec<HubRecord>) {
    let other = FakeServer::new(OTHER, 7)
        .named("Work hub")
        .with_user("ada", "secret", "lab");
    let records = vec![pinned_record(), other.pinned_record("Work")];
    (
        FakeHub::new().with_server(server()).with_server(other),
        records,
    )
}

fn device(id: &str, name: &str, seen: i64, current: bool) -> DeviceSummary {
    DeviceSummary {
        id: id.into(),
        name: name.into(),
        platform: "desktop".into(),
        created_at_ms: 1_000,
        last_seen_at_ms: seen,
        is_current: current,
    }
}

/// The page over a signed-in server whose account has a laptop (this device) and a phone.
fn page_with_devices(cx: &mut TestAppContext) -> Fixture {
    let page = page(cx, vec![pinned_record()]);
    page.devices.set(
        &hub_id(),
        vec![
            device("phone", "Pixel", 1_700_000_000_000, false),
            device("laptop", "Laptop", 1_700_000_500_000, true),
        ],
    );
    page
}

#[gpui_kit::test]
fn the_first_server_is_selected_and_choosing_another_shows_its_overview(cx: &mut TestAppContext) {
    let (hub, records) = two_servers();
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_on(cx, config, hub, records, Some("ada"));
    assert!(page.shows(item("server-select", KEY), cx));
    assert!(page.shows(item("server-select", OTHER), cx));
    assert!(page.shows(item("server-overview", KEY), cx));
    assert!(!page.shows(item("server-overview", OTHER), cx));

    page.click(item("server-select", OTHER), cx);

    assert!(page.shows(item("server-overview", OTHER), cx));
    assert!(!page.shows(item("server-overview", KEY), cx));
}

#[gpui_kit::test]
fn the_tabs_switch_between_the_overview_the_devices_and_the_keys(cx: &mut TestAppContext) {
    let page = page_with_devices(cx);
    assert!(page.shows(item("server-overview", KEY), cx));

    open_tab(&page, DEVICES, cx);
    assert!(page.shows("devices-tab", cx));
    assert!(!page.shows(item("server-overview", KEY), cx));

    open_tab(&page, KEYS, cx);
    assert!(page.shows(item("server-trust-tab", KEY), cx));
    assert!(!page.shows("devices-tab", cx));

    open_tab(&page, OVERVIEW, cx);
    assert!(page.shows(item("server-overview", KEY), cx));
}

#[gpui_kit::test]
fn choosing_another_server_starts_at_its_overview(cx: &mut TestAppContext) {
    let (hub, records) = two_servers();
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_on(cx, config, hub, records, Some("ada"));
    open_tab(&page, KEYS, cx);

    page.click(item("server-select", OTHER), cx);

    assert!(page.shows(item("server-overview", OTHER), cx));
}

#[gpui_kit::test]
fn the_devices_tab_lists_the_account_devices_and_marks_this_one(cx: &mut TestAppContext) {
    let page = page_with_devices(cx);
    open_tab(&page, DEVICES, cx);

    assert!(page.shows(item("device", "phone"), cx));
    assert!(page.shows(item("device", "laptop"), cx));
    assert!(page.shows(item("device-current", "laptop"), cx));
    assert!(!page.shows(item("device-current", "phone"), cx));
    assert!(page.shows(item("device-seen", "phone"), cx));
}

#[gpui_kit::test]
fn the_devices_tab_says_so_while_the_list_is_read_and_when_there_is_none(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    let release = page.devices.hold_next_list();
    open_tab(&page, DEVICES, cx);
    assert!(page.shows("devices-loading", cx));

    drop(release);
    cx.run_until_parked();
    page.render(cx);
    assert!(!page.shows("devices-loading", cx));
    assert!(page.shows("devices-empty", cx));
}

#[gpui_kit::test]
fn a_device_list_that_cannot_be_read_is_reported_and_read_again_on_request(
    cx: &mut TestAppContext,
) {
    let page = page_with_devices(cx);
    page.devices.fail_with(HubError::PermissionDenied);
    open_tab(&page, DEVICES, cx);
    assert!(page.shows("devices-error", cx));
    assert!(!page.shows(item("device", "phone"), cx));

    page.devices.recover();
    page.click("devices-retry", cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(!page.shows("devices-error", cx));
    assert!(page.shows(item("device", "phone"), cx));
}

#[gpui_kit::test]
fn revoking_a_device_asks_first_and_keeping_it_changes_nothing(cx: &mut TestAppContext) {
    let page = page_with_devices(cx);
    open_tab(&page, DEVICES, cx);

    page.click(item("device-revoke", "phone"), cx);
    page.settle_dialog(cx);
    assert!(page.has_dialog(cx), "revoking asks first");
    assert!(page.devices.revoked().is_empty());
    page.click("revoke-device-keep", cx);
    cx.run_until_parked();

    assert!(page.devices.revoked().is_empty());
    assert!(page.shows(item("device", "phone"), cx));
}

#[gpui_kit::test]
fn confirming_revokes_the_device_and_the_list_is_read_again(cx: &mut TestAppContext) {
    let page = page_with_devices(cx);
    open_tab(&page, DEVICES, cx);

    page.click(item("device-revoke", "phone"), cx);
    page.settle_dialog(cx);
    page.click("revoke-device-confirm", cx);
    cx.run_until_parked();
    page.render(cx);

    assert_eq!(page.devices.revoked(), vec![(hub_id(), "phone".to_owned())]);
    assert!(!page.shows(item("device", "phone"), cx));
    assert!(page.shows(item("device", "laptop"), cx));
}

#[gpui_kit::test]
fn revoking_this_device_signs_out_of_the_server(cx: &mut TestAppContext) {
    let page = page_with_devices(cx);
    open_tab(&page, DEVICES, cx);

    page.click(item("device-revoke", "laptop"), cx);
    page.settle_dialog(cx);
    page.click("revoke-device-confirm", cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(
        page.devices.revoked().is_empty(),
        "the sign-out path ends the session"
    );
    assert_eq!(page.hub.count(FakeOp::SignOut), 1);
    assert_eq!(servers(&page, cx)[0].status, HubStatus::SignedOut);
    assert!(page.shows(item("server-tab-signed-out", KEY), cx));
}

#[gpui_kit::test]
fn a_server_that_is_not_signed_in_offers_signing_in_instead_of_devices_and_keys(
    cx: &mut TestAppContext,
) {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_over(cx, config, vec![pinned_record()], None);

    open_tab(&page, DEVICES, cx);
    assert!(page.shows(item("server-tab-signed-out", KEY), cx));
    assert!(page.shows(item("server-tab-sign-in", KEY), cx));
    assert!(!page.shows("devices-tab", cx));

    open_tab(&page, KEYS, cx);
    assert!(page.shows(item("server-tab-signed-out", KEY), cx));
    assert!(!page.shows(item("server-trust-tab", KEY), cx));

    page.click(item("server-tab-sign-in", KEY), cx);
    page.settle_dialog(cx);
    assert!(page.has_dialog(cx), "the sign-in dialog opens");
}

#[path = "servers_agent_access_tests.rs"]
mod agent_access;
