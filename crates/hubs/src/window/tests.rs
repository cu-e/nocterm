use std::time::Duration;

use gpui::{Entity, TestAppContext};
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp};
use nocterm_hubs_model::{HubSessionId, HubTerminalTarget, NodeId, OrgId};

use crate::fixture::{Fixture, address, fixture, id, pinned_record, server};
use crate::{HubWindow, Load};

fn open(cx: &mut TestAppContext) -> (Fixture, Entity<HubWindow>) {
    let hub = FakeHub::new().with_server(server());
    hub.sign_in_as(&id(), "ada");
    let fx = fixture(cx, hub, vec![pinned_record()]);
    cx.run_until_parked();
    let window = fx
        .hubs
        .update(cx, |hubs, cx| hubs.window(&id(), cx))
        .expect("a known hub");
    cx.run_until_parked();
    (fx, window)
}

#[gpui::test]
async fn the_window_reads_the_account_its_organization_nodes_and_sessions(cx: &mut TestAppContext) {
    let (_fx, window) = open(cx);
    window.read_with(cx, |window, _| {
        assert!(matches!(window.load(), Load::Loaded));
        assert_eq!(window.account().unwrap().username, "ada");
        assert_eq!(window.org(), Some(&OrgId::new("lab")));
        assert_eq!(window.nodes().len(), 1);
        let titles: Vec<_> = window.sessions().iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["make", "top"]);
        let mine: Vec<_> = window
            .sessions()
            .iter()
            .filter(|s| window.is_mine(s))
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].title, "make");
    });
}

#[gpui::test]
async fn asking_again_while_a_read_runs_repeats_it_once_instead_of_piling_up(
    cx: &mut TestAppContext,
) {
    let (fx, window) = open(cx);
    let before = fx.hub.count(FakeOp::Sessions);
    let release = fx.hub.hold(FakeOp::Sessions);
    window.update(cx, |window, cx| window.refresh(cx));
    cx.run_until_parked();
    window.update(cx, |window, cx| window.refresh(cx));
    window.update(cx, |window, cx| window.refresh(cx));
    release.release();
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Sessions), before + 2);
}

#[gpui::test]
async fn an_organization_chosen_while_a_read_runs_is_not_replaced_by_that_read(
    cx: &mut TestAppContext,
) {
    let (fx, window) = open(cx);
    fx.hub.edit_server(&address(), |server| {
        *server = server
            .clone()
            .with_session("z1", "zoo", "n1", "ada", "zoo shell");
    });
    let release = fx.hub.hold(FakeOp::Sessions);
    window.update(cx, |window, cx| window.refresh(cx));
    cx.run_until_parked();

    window.update(cx, |window, cx| window.select_org(OrgId::new("zoo"), cx));
    release.release();
    cx.run_until_parked();

    window.read_with(cx, |window, _| {
        assert_eq!(window.org(), Some(&OrgId::new("zoo")));
        let titles: Vec<_> = window.sessions().iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["zoo shell"]);
    });
}

#[gpui::test]
async fn ending_a_session_removes_it_from_the_list(cx: &mut TestAppContext) {
    let (fx, window) = open(cx);
    window
        .update(cx, |window, cx| window.end(HubSessionId::new("s1"), cx))
        .await
        .unwrap();
    cx.run_until_parked();
    assert!(
        fx.hub
            .calls()
            .contains(&FakeCall::End(id(), HubSessionId::new("s1")))
    );
    window.read_with(cx, |window, _| {
        let titles: Vec<_> = window.sessions().iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["top"]);
    });
}

#[gpui::test]
async fn the_lists_refresh_every_few_seconds_only_while_somebody_watches(cx: &mut TestAppContext) {
    let (fx, window) = open(cx);
    let watch = window.update(cx, |window, cx| window.watch(cx));
    let start = fx.hub.count(FakeOp::Sessions);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Sessions), start + 1);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Sessions), start + 2);

    drop(watch);
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(fx.hub.count(FakeOp::Sessions), start + 2);
}

#[gpui::test]
async fn a_new_session_target_names_the_shown_organization_and_a_terminal_records_it(
    cx: &mut TestAppContext,
) {
    let (fx, window) = open(cx);
    let target = window.read_with(cx, |window, _| {
        window.new_session(Some(NodeId::new("n1")), "build")
    });
    let target = target.expect("the organization is known");
    assert_eq!(
        target,
        HubTerminalTarget::New {
            org: OrgId::new("lab"),
            node: Some(NodeId::new("n1")),
            title: "build".into(),
        }
    );
    let _transport = window.read_with(cx, |window, _| window.terminal(target.clone()));
    assert!(fx.hub.calls().contains(&FakeCall::Terminal(id(), target)));
}

fn two_organizations(fx: &Fixture) {
    fx.hub.edit_server(&address(), |server| {
        *server = server.clone().with_membership("ada", "zoo").with_session(
            "z1",
            "zoo",
            "n1",
            "ada",
            "zoo shell",
        );
    });
}

#[gpui::test]
async fn another_organization_is_loading_until_its_lists_arrive(cx: &mut TestAppContext) {
    let (fx, window) = open(cx);
    two_organizations(&fx);
    let release = fx.hub.hold(FakeOp::Sessions);

    window.update(cx, |window, cx| window.select_org(OrgId::new("zoo"), cx));
    cx.run_until_parked();
    window.read_with(cx, |window, _| {
        assert!(matches!(window.load(), Load::Loading));
        assert!(window.sessions().is_empty());
    });

    release.release();
    cx.run_until_parked();
    window.read_with(cx, |window, _| {
        assert!(matches!(window.load(), Load::Loaded));
        assert_eq!(window.sessions().len(), 1);
    });
}
