//! The sessions panel and the new-session menu of a hub window.
use crate::OpenHubSessions;
use crate::plugin::SESSIONS;
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp};
use nocterm_hubs_model::{Environment, HubSessionId, HubTerminalTarget, NodeId, OrgId};
use nocterm_session::Target;
use std::time::Duration;

fn hub_window(cx: &mut TestAppContext, hub: FakeHub) -> Fixture {
    let window = fixture(
        cx,
        Environment::Hub(hub_id()),
        hub,
        vec![pinned_record()],
        Some("ada"),
    );
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
fn the_plus_button_opens_the_menu_that_starts_a_session_on_a_chosen_machine(
    cx: &mut TestAppContext,
) {
    let window = online_window(cx);
    window.click("hub-sessions-new", cx);
    assert!(cx.update(|cx| window.workspace.read(cx).new_tab_menu_is_open()));
    window.click(("new-session-node", 1usize), cx);
    assert!(
        cx.update(|cx| window.workspace.read(cx).new_tab_menu_is_open()),
        "a machine that is offline cannot be chosen"
    );
    assert!(window.terminals.tabs().is_empty());
    window.click(("new-session-node", 0usize), cx);
    assert!(!cx.update(|cx| window.workspace.read(cx).new_tab_menu_is_open()));
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
fn a_new_session_on_any_machine_leaves_the_choice_to_the_hub(cx: &mut TestAppContext) {
    let window = online_window(cx);
    window.click("hub-sessions-new", cx);
    window.click("new-session-auto", cx);
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
