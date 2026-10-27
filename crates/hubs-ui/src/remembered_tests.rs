//! What a hub window keeps for the next start, and how it comes back.
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, restored_fixture, server};
use gpui_kit::TestAppContext;
use nocterm_hubs_model::testing::{FakeCall, FakeHub};
use nocterm_hubs_model::{
    Environment, HubSessionId, HubSessionState, HubTerminalTarget, HubWindowMemory, OrgId,
    RememberedSession,
};

fn memory(organization: Option<&str>, sessions: &[&str]) -> HubWindowMemory {
    HubWindowMemory::new(
        organization.map(OrgId::new),
        sessions
            .iter()
            .map(|entry| {
                let (org, id) = entry.split_once('/').unwrap_or(("lab", entry));
                RememberedSession::new(OrgId::new(org), HubSessionId::new(id))
            })
            .collect(),
    )
}

fn lab() -> FakeHub {
    FakeHub::new().with_server(server())
}

fn window(cx: &mut TestAppContext) -> Fixture {
    let window = fixture(
        cx,
        Environment::Hub(hub_id()),
        lab(),
        vec![pinned_record()],
        Some("ada"),
    );
    window.render(cx);
    window
}

fn titles(window: &Fixture) -> Vec<String> {
    window
        .terminals
        .tabs()
        .iter()
        .map(|(title, _)| title.to_string())
        .collect()
}

fn attached(window: &Fixture, session: &str) -> bool {
    window.hub.calls().contains(&FakeCall::Terminal(
        hub_id(),
        HubTerminalTarget::Existing(HubSessionId::new(session)),
    ))
}

#[gpui_kit::test]
fn a_window_remembers_its_organization_and_the_sessions_of_its_tabs_in_order(
    cx: &mut TestAppContext,
) {
    let window = window(cx);
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &[])));

    window.click("hub-session-open-s2", cx);
    window.click("hub-session-open-s1", cx);

    assert_eq!(
        window.memory.latest(),
        Some(memory(Some("lab"), &["s2", "s1"]))
    );
}

#[gpui_kit::test]
fn closing_a_tab_forgets_its_session_and_keeps_the_others(cx: &mut TestAppContext) {
    let window = window(cx);
    window.click("hub-session-open-s1", cx);
    window.click("hub-session-open-s2", cx);

    window.terminals.close_tab(0);
    cx.run_until_parked();

    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &["s2"])));
}

#[gpui_kit::test]
fn a_session_a_tab_started_is_remembered_once_the_hub_made_it(cx: &mut TestAppContext) {
    let window = window(cx);
    window.hub.create_sessions_as(["made-1"]);

    window.click("hub-sessions-new", cx);
    window.click("new-session-auto", cx);
    cx.run_until_parked();

    assert_eq!(
        window.memory.latest(),
        Some(memory(Some("lab"), &["made-1"]))
    );
    window.terminals.close_tab(0);
    cx.run_until_parked();
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &[])));
}

#[gpui_kit::test]
fn a_session_the_hub_never_made_is_not_remembered(cx: &mut TestAppContext) {
    let window = window(cx);

    window.click("hub-sessions-new", cx);
    window.click("new-session-auto", cx);
    cx.run_until_parked();

    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &[])));
}

#[gpui_kit::test]
fn another_organization_is_remembered_when_it_is_chosen(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server().with_membership("ada", "zoo"));
    let window = fixture(
        cx,
        Environment::Hub(hub_id()),
        hub,
        vec![pinned_record()],
        Some("ada"),
    );
    let state = window
        .installed
        .hubs()
        .update(cx, |hubs, cx| hubs.window(&hub_id(), cx))
        .unwrap();

    state.update(cx, |state, cx| state.select_org(OrgId::new("zoo"), cx));
    cx.run_until_parked();

    assert_eq!(window.memory.latest(), Some(memory(Some("zoo"), &[])));
}

#[gpui_kit::test]
fn the_sessions_that_still_run_are_attached_again_in_order(cx: &mut TestAppContext) {
    let window = restored_fixture(cx, lab(), memory(Some("lab"), &["s2", "s1"]));

    assert_eq!(titles(&window), ["top", "make"]);
    assert!(attached(&window, "s1") && attached(&window, "s2"));
    assert_eq!(
        window.memory.latest(),
        Some(memory(Some("lab"), &["s2", "s1"]))
    );
}

#[gpui_kit::test]
fn a_session_that_ended_meanwhile_is_dropped_without_a_tab(cx: &mut TestAppContext) {
    let hub = lab();
    hub.edit_server(&pinned_record().address, |server| {
        server.sessions[1].state = HubSessionState::Exited;
    });

    let window = restored_fixture(cx, hub, memory(Some("lab"), &["s2", "gone", "s1"]));

    assert_eq!(titles(&window), ["make"]);
    assert!(!attached(&window, "s2") && !attached(&window, "gone"));
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &["s1"])));
}

#[gpui_kit::test]
fn the_remembered_organization_is_shown_when_the_account_still_belongs_to_it(
    cx: &mut TestAppContext,
) {
    let hub = FakeHub::new().with_server(server().with_membership("ada", "zoo").with_session(
        "z1",
        "zoo",
        "n1",
        "ada",
        "zoo shell",
    ));

    let window = restored_fixture(cx, hub, memory(Some("zoo"), &["zoo/z1"]));

    assert_eq!(titles(&window), ["zoo shell"]);
    assert_eq!(
        window.memory.latest(),
        Some(memory(Some("zoo"), &["zoo/z1"]))
    );
}

#[gpui_kit::test]
fn a_tab_of_another_organization_than_the_one_shown_comes_back_too(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server().with_membership("ada", "zoo").with_session(
        "z1",
        "zoo",
        "n1",
        "ada",
        "zoo shell",
    ));

    let window = restored_fixture(cx, hub, memory(Some("zoo"), &["lab/s1", "zoo/z1"]));

    assert_eq!(titles(&window), ["make", "zoo shell"]);
    assert_eq!(
        window.memory.latest(),
        Some(memory(Some("zoo"), &["lab/s1", "zoo/z1"]))
    );
}

#[gpui_kit::test]
fn a_session_of_an_organization_the_account_left_is_dropped(cx: &mut TestAppContext) {
    let window = restored_fixture(cx, lab(), memory(Some("lab"), &["gone/g1", "s1"]));

    assert_eq!(titles(&window), ["make"]);
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &["s1"])));
}

#[gpui_kit::test]
fn an_organization_the_account_left_gives_way_to_the_default(cx: &mut TestAppContext) {
    let window = restored_fixture(cx, lab(), memory(Some("gone"), &["s1"]));

    assert_eq!(titles(&window), ["make"]);
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &["s1"])));
}

#[gpui_kit::test]
fn a_hub_that_is_not_reachable_keeps_the_sessions_until_it_is(cx: &mut TestAppContext) {
    let hub = lab();
    let address = pinned_record().address;
    hub.edit_server(&address, |server| server.online = false);

    let window = restored_fixture(cx, hub, memory(Some("lab"), &["s1", "gone"]));
    window.render(cx);

    assert!(window.terminals.tabs().is_empty());
    assert_eq!(
        window.memory.count(),
        0,
        "nothing changed, so the saved memory stands"
    );

    window
        .hub
        .edit_server(&address, |server| server.online = true);
    window.click("hub-sessions-retry", cx);

    assert_eq!(titles(&window), ["make"]);
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &["s1"])));
}

#[gpui_kit::test]
fn a_window_that_restores_nothing_keeps_nothing_new(cx: &mut TestAppContext) {
    let window = restored_fixture(cx, lab(), HubWindowMemory::default());

    assert!(window.terminals.tabs().is_empty());
    assert_eq!(window.memory.latest(), Some(memory(Some("lab"), &[])));
}
