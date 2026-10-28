//! The Servers page lists the known servers and manages them through the service.
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::{ServersPage, row::item, settings_page};
use crate::test_support::{Fixture, fixture_over, hub_id, pinned_record, server};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use nocterm_hubs::{HubStatus, HubSummary};
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp};
use nocterm_hubs_model::{Fingerprint, HubColor, HubRecord, TrustPolicy};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SnapshotWriter, StoreError};

const KEY: &str = "hub.home:7443";

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
    let setup = fixture_over(cx, config, hub, known, signed_in);
    let hubs = setup.installed.hubs().clone();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| ServersPage::new(hubs, cx))
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
fn a_signed_in_server_can_become_the_account_server_and_stop_being_it(cx: &mut TestAppContext) {
    let page = page(cx, vec![pinned_record()]);
    assert!(page.shows(item("server-account-on", KEY), cx));
    assert!(!page.shows(item("server-account-off", KEY), cx));

    page.click(item("server-account-on", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(page.shows(item("server-account-off", KEY), cx));
    let chosen = cx.update(|cx| page.installed.hubs().read(cx).account_hub(cx));
    assert_eq!(chosen, Some(hub_id()));

    page.click(item("server-account-off", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(page.shows(item("server-account-on", KEY), cx));
}

#[gpui_kit::test]
fn a_server_that_is_not_signed_in_cannot_become_the_account_server(cx: &mut TestAppContext) {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_over(cx, config, vec![pinned_record()], None);
    assert!(!page.shows(item("server-account-on", KEY), cx));
}

#[gpui_kit::test]
fn a_server_whose_recovery_key_was_never_confirmed_offers_a_new_one(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "secret", "recovery-ada"));
    hub.leave_recovery_key_unconfirmed(&hub_id());
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_on(cx, config, hub, vec![pinned_record()], Some("ada"));
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
    assert!(!page.shows(item("server-recovery-issue", KEY), cx));
}

/// Counts how often the page asks to be drawn again.
fn count_redraws(page: &Fixture, cx: &mut TestAppContext) -> Rc<Cell<usize>> {
    let redraws = Rc::new(Cell::new(0));
    let counted = Rc::clone(&redraws);
    cx.update_window(page.window, |root, _, cx| {
        let servers = root.downcast::<ServersPage>().unwrap();
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
