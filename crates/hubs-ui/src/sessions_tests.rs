//! The sessions panel: the sessions of a hub, the choice of the hub, and new sessions.
use crate::OpenHubSessions;
use crate::plugin::SESSIONS;
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp, FakeServer};
use nocterm_hubs_model::{
    Fingerprint, HubRecord, HubSessionId, HubTerminalTarget, NodeId, OrgId, TrustPolicy,
};
use nocterm_session::Target;
use std::time::Duration;

fn hub_window(cx: &mut TestAppContext, hub: FakeHub) -> Fixture {
    let window = fixture(cx, hub, vec![pinned_record()], Some("ada"));
    window.open_sessions(cx);
    window.render(cx);
    window
}

fn online_window(cx: &mut TestAppContext) -> Fixture {
    hub_window(cx, FakeHub::new().with_server(server()))
}

#[gpui_kit::test]
fn sessions_are_listed_under_the_machine_they_run_on(cx: &mut TestAppContext) {
    let window = online_window(cx);
    assert!(window.shows("hub-node-n1", cx));
    assert!(
        window.shows("hub-node-n2", cx),
        "an offline machine is listed too"
    );
    assert!(window.shows("hub-session-s1", cx));
    assert!(window.shows("hub-session-s2", cx));
}

#[gpui_kit::test]
fn opening_a_session_attaches_a_tab_to_that_session(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-session-open-s1", cx);
    let tabs = window.terminals.tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].0.as_ref(), "make");
    assert_eq!(tabs[0].1, Target::new("ada", "hub.home", 7443));
    assert!(window.hub.calls().contains(&FakeCall::Terminal(
        hub_id(),
        HubTerminalTarget::Existing(HubSessionId::new("s1"))
    )));
}

#[gpui_kit::test]
fn double_clicking_a_session_opens_it(cx: &mut TestAppContext) {
    let window = online_window(cx);
    cx.update_window(window.window, |_, handle, cx| {
        handle.double_click("hub-session-s1", cx);
    })
    .unwrap();
    window.render(cx);
    assert_eq!(window.terminals.tabs().len(), 1);
}

#[gpui_kit::test]
fn enter_opens_the_session_that_was_selected_with_a_click(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-session-s1", cx);
    assert!(window.terminals.tabs().is_empty(), "one click only selects");
    window.press("enter", cx);
    let tabs = window.terminals.tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].0.as_ref(), "make");
}

#[gpui_kit::test]
fn the_arrow_keys_move_the_selection_between_sessions(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-session-s1", cx);
    window.press("down", cx);
    window.press("enter", cx);
    let tabs = window.terminals.tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].0.as_ref(), "top", "down goes to the next session");
    window.press("up", cx);
    window.press("enter", cx);
    assert_eq!(window.terminals.tabs()[1].0.as_ref(), "make");
}

#[gpui_kit::test]
fn enter_with_nothing_selected_opens_nothing(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.press("enter", cx);
    assert!(window.terminals.tabs().is_empty());
}

#[gpui_kit::test]
fn only_sessions_of_this_account_can_be_ended(cx: &mut TestAppContext) {
    let window = online_window(cx);
    assert!(window.shows("hub-session-end-s1", cx));
    assert!(!window.shows("hub-session-end-s2", cx), "s2 belongs to bob");
}

#[gpui_kit::test]
fn ending_a_session_asks_first_and_keeping_it_ends_nothing(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-session-end-s1", cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
    assert_eq!(
        window.hub.count(FakeOp::End),
        0,
        "nothing ends before the answer"
    );
    window.click("end-session-keep", cx);
    assert!(!window.has_dialog(cx));
    assert_eq!(window.hub.count(FakeOp::End), 0);
    assert!(window.shows("hub-session-s1", cx));
}

#[gpui_kit::test]
fn confirming_ends_the_session_and_the_list_drops_it(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-session-end-s1", cx);
    window.settle_dialog(cx);
    window.click("end-session-confirm", cx);
    cx.run_until_parked();
    window.render(cx);
    assert!(window.hub.calls().iter().any(|call| matches!(
        call,
        FakeCall::End(hub, session) if *hub == hub_id() && session.as_str() == "s1"
    )));
    assert!(!window.shows("hub-session-s1", cx));
    assert!(window.shows("hub-session-s2", cx));
}

#[gpui_kit::test]
fn the_plus_beside_a_machine_starts_a_session_on_it(cx: &mut TestAppContext) {
    let window = online_window(cx);
    assert!(
        !window.shows("hub-node-new-n2", cx),
        "a machine that is offline cannot be chosen"
    );
    window.click("hub-node-new-n1", cx);
    assert_eq!(window.terminals.tabs()[0].0.as_ref(), "Shell on build box");
    assert!(window.hub.calls().contains(&FakeCall::Terminal(
        hub_id(),
        HubTerminalTarget::New {
            org: OrgId::new("lab"),
            node: Some(NodeId::new("n1")),
            title: "Shell on build box".to_owned(),
        }
    )));
}

#[gpui_kit::test]
fn the_plus_in_the_header_leaves_the_choice_of_the_machine_to_the_hub(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-sessions-new", cx);
    assert_eq!(window.terminals.tabs()[0].0.as_ref(), "Shell");
    assert!(window.hub.calls().contains(&FakeCall::Terminal(
        hub_id(),
        HubTerminalTarget::New {
            org: OrgId::new("lab"),
            node: None,
            title: "Shell".to_owned(),
        }
    )));
}

#[gpui_kit::test]
fn without_a_known_server_the_panel_offers_to_sign_in(cx: &mut TestAppContext) {
    let window = fixture(cx, FakeHub::new().with_server(server()), Vec::new(), None);
    window.open_sessions(cx);
    assert!(window.shows("hub-sessions-add-server", cx));
    window.click("hub-sessions-add-server", cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
}

#[gpui_kit::test]
fn a_server_that_is_signed_out_offers_to_sign_in_again(cx: &mut TestAppContext) {
    let window = fixture(
        cx,
        FakeHub::new().with_server(server()),
        vec![pinned_record()],
        None,
    );
    window.open_sessions(cx);
    window.click("hub-sessions-sign-in", cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
}

#[gpui_kit::test]
fn a_changed_certificate_is_explained_and_its_sessions_are_never_listed(cx: &mut TestAppContext) {
    let changed = HubRecord::new(
        pinned_record().address,
        TrustPolicy::Pinned(Fingerprint::from_bytes([9; 32])),
        "Home",
    );
    let window = fixture(
        cx,
        FakeHub::new().with_server(server()),
        vec![changed],
        Some("ada"),
    );
    window.open_sessions(cx);
    assert!(!window.shows("hub-session-s1", cx));
    window.click("hub-sessions-explain", cx);
    window.settle_dialog(cx);
    assert!(window.shows("trust-failed-remove", cx));
}

#[gpui_kit::test]
fn with_several_servers_the_panel_lists_the_one_chosen(cx: &mut TestAppContext) {
    let other = FakeServer::new("other.home:7443", 3)
        .named("Other hub")
        .with_user("ada", "secret", "lab")
        .with_node("m1", "other box", true);
    let hub = FakeHub::new()
        .with_server(server())
        .with_server(other.clone());
    hub.sign_in_as(&other_id(), "ada");
    let window = fixture(
        cx,
        hub,
        vec![pinned_record(), other.pinned_record("Other")],
        Some("ada"),
    );
    window.open_sessions(cx);
    assert!(
        window.shows("hub-node-n1", cx),
        "the first server shows at first"
    );
    assert!(!window.shows("hub-node-m1", cx));
    window.click(format!("hub-choice-{}", other_id()), cx);
    cx.run_until_parked();
    window.render(cx);
    assert!(window.shows("hub-node-m1", cx));
    assert!(!window.shows("hub-node-n1", cx));
}

fn other_id() -> nocterm_hubs_model::HubId {
    nocterm_hubs_model::HubId::from(
        nocterm_hubs_model::HubAddress::parse("other.home:7443").unwrap(),
    )
}

#[gpui_kit::test]
fn an_unreachable_hub_shows_a_retry_that_connects_again(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server());
    hub.edit_server(&pinned_record().address, |server| server.online = false);
    let window = hub_window(cx, hub);
    assert!(window.shows("hub-sessions-retry", cx));
    let before = window.hub.count(FakeOp::Connect);
    window.click("hub-sessions-retry", cx);
    assert!(window.hub.count(FakeOp::Connect) > before);
}

#[gpui_kit::test]
fn the_lists_refresh_while_the_panel_shows_and_stop_when_it_is_hidden(cx: &mut TestAppContext) {
    let window = online_window(cx);
    let read = |window: &Fixture| window.hub.count(FakeOp::Sessions);
    let shown = read(&window);
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    assert!(read(&window) > shown, "a shown panel polls");

    window.dispatch(OpenHubSessions, cx);
    assert!(!cx.update(|cx| window.workspace.read(cx).panel_is_open(&SESSIONS.id)));
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    let hidden = read(&window);
    cx.executor().advance_clock(Duration::from_secs(12));
    cx.run_until_parked();
    assert_eq!(read(&window), hidden, "a hidden panel does not poll");
}
