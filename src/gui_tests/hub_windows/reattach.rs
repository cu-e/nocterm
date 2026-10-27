//! A restored hub window brings back its organization and the sessions of its tabs.
use super::restore::restore_records;
use super::*;
use nocterm_hubs_model::{
    HubSessionId, HubTerminalTarget, HubWindowMemory, OrgId, testing::FakeCall,
};

fn memory(organization: Option<&str>, sessions: &[&str]) -> HubWindowMemory {
    HubWindowMemory::new(
        organization.map(OrgId::new),
        sessions.iter().map(|id| HubSessionId::new(*id)).collect(),
    )
}

fn record(memory: HubWindowMemory) -> WindowRecord {
    WindowRecord::new(Environment::Hub(hub_id())).with_hub(memory)
}

fn kept(booted: &Booted, cx: &mut TestAppContext) -> Vec<WindowRecord> {
    cx.update(|cx| booted.windowing().records(cx))
}

fn attached(hub: &FakeHub, session: &str) -> bool {
    hub.calls().contains(&FakeCall::Terminal(
        hub_id(),
        HubTerminalTarget::Existing(HubSessionId::new(session)),
    ))
}

fn hub_window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let all = cx.update(|cx| cx.windows());
    assert_eq!(all.len(), 1);
    all[0]
}

#[gpui_kit::test]
fn a_restored_hub_window_reattaches_its_remembered_session(cx: &mut TestAppContext) {
    let hub = hub();
    let booted = boot_over(Some(&hub), &Arc::new(MockTransport::default()), cx);
    let _screen = hub.terminals().queue(None, None);

    restore_records(&booted, &[record(memory(Some("lab"), &["s1"]))], cx);

    assert!(attached(&hub, "s1"));
    assert_eq!(tab_count(hub_window(cx), cx), 1);
    assert_eq!(kept(&booted, cx), [record(memory(Some("lab"), &["s1"]))]);
}

#[gpui_kit::test]
fn a_session_that_ended_meanwhile_is_dropped_quietly(cx: &mut TestAppContext) {
    let hub = hub();
    let booted = boot_over(Some(&hub), &Arc::new(MockTransport::default()), cx);
    let _screen = hub.terminals().queue(None, None);

    restore_records(&booted, &[record(memory(Some("lab"), &["gone", "s1"]))], cx);

    assert!(attached(&hub, "s1"));
    assert!(!attached(&hub, "gone"));
    assert_eq!(tab_count(hub_window(cx), cx), 1);
    assert_eq!(kept(&booted, cx), [record(memory(Some("lab"), &["s1"]))]);
}

#[gpui_kit::test]
fn a_restored_hub_window_shows_the_remembered_organization(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(
        FakeServer::new(ADDRESS, 7)
            .named("Home hub")
            .with_user("ada", "secret", "lab")
            .with_membership("ada", "zoo")
            .with_node("n1", "build box", true)
            .with_session("s1", "lab", "n1", "ada", "make")
            .with_session("z1", "zoo", "n1", "ada", "feed"),
    );
    hub.sign_in_as(&hub_id(), "ada");
    let booted = boot_over(Some(&hub), &Arc::new(MockTransport::default()), cx);
    let _screen = hub.terminals().queue(None, None);

    restore_records(&booted, &[record(memory(Some("zoo"), &["z1"]))], cx);

    assert!(attached(&hub, "z1"));
    assert_eq!(kept(&booted, cx), [record(memory(Some("zoo"), &["z1"]))]);
}

#[gpui_kit::test]
fn a_window_of_this_computer_keeps_nothing_of_a_hub(cx: &mut TestAppContext) {
    let hub = hub();
    let booted = boot_over(Some(&hub), &Arc::new(MockTransport::default()), cx);

    restore_records(
        &booted,
        &[WindowRecord::new(Environment::Local).with_hub(memory(Some("lab"), &["s1"]))],
        cx,
    );

    assert!(!attached(&hub, "s1"));
    assert_eq!(kept(&booted, cx), [WindowRecord::new(Environment::Local)]);
}
