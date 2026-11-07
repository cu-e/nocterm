use std::sync::Arc;

use gpui::TestAppContext;
use nocterm_hub_agents_model::testing::FakeHubAgents;
use nocterm_hubs_model::{HubAddress, HubId};

use crate::{AgentAccessError, AgentTarget, Deps, HubAgentsEvent, install};
use nocterm_ssh_keys::testing::{FakeAgentHosts, FakeAuthorizedKeys};

fn hub() -> HubId {
    HubId::from(HubAddress::parse("hub.example:7443").unwrap())
}

fn deps(fake: &FakeHubAgents) -> Deps {
    deps_over(fake, &FakeAgentHosts::default())
}

fn deps_over(fake: &FakeHubAgents, hosts: &FakeAgentHosts) -> Deps {
    Deps {
        directory: Arc::new(fake.clone()),
        servers: Arc::new(fake.clone()),
        chats: Arc::new(fake.clone()),
        hosts: Arc::new(hosts.clone()),
    }
}

fn web() -> AgentTarget {
    AgentTarget {
        connection: "web".into(),
        name: "Web".into(),
        host: "web.example".into(),
        port: 22,
        user: "deploy".into(),
    }
}

#[gpui::test]
fn a_tracked_hub_offers_its_agents_as_agents_a_chat_can_start(cx: &mut TestAppContext) {
    let fake = FakeHubAgents::new()
        .with_provider(&hub(), "Models", &["m1"])
        .with_agent(&hub(), "Ops", "m1");
    let installed = cx.update(|cx| install(deps(&fake), cx));
    let agents = installed.agents().clone();
    agents.update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();
    agents.read_with(cx, |agents, _| {
        assert_eq!(agents.agents(&hub()).len(), 1);
        let launches = agents.launches();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].name, "Ops");
        assert_eq!(launches[0].hub.as_ref().unwrap().hub, hub());
    });
    agents.update(cx, |agents, cx| agents.untrack(&hub(), cx));
    agents.read_with(cx, |agents, _| assert!(agents.launches().is_empty()));
}

#[gpui::test]
fn a_change_of_the_hubs_chats_is_told_to_the_views(cx: &mut TestAppContext) {
    let fake = FakeHubAgents::new();
    let installed = cx.update(|cx| install(deps(&fake), cx));
    let agents = installed.agents().clone();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let _subscription = cx.update({
        let seen = seen.clone();
        |cx| {
            cx.subscribe(&agents, move |_, event: &HubAgentsEvent, _| {
                seen.lock().unwrap().push(event.clone());
            })
        }
    });
    agents.update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();
    fake.chats_changed();
    cx.run_until_parked();
    assert!(
        seen.lock()
            .unwrap()
            .contains(&HubAgentsEvent::ChatsChanged(hub()))
    );
}

#[gpui::test]
fn a_failed_listing_keeps_the_last_lists_and_says_why(cx: &mut TestAppContext) {
    let fake = FakeHubAgents::new()
        .with_provider(&hub(), "Models", &["m1"])
        .with_agent(&hub(), "Ops", "m1");
    let installed = cx.update(|cx| install(deps(&fake), cx));
    let agents = installed.agents().clone();
    agents.update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();
    fake.set_offline(true);
    agents.update(cx, |agents, cx| agents.refresh(&hub(), cx));
    cx.run_until_parked();
    agents.read_with(cx, |agents, _| {
        assert_eq!(agents.agents(&hub()).len(), 1);
        assert!(agents.problem(&hub()).is_some());
    });
}

#[gpui::test]
fn granting_access_writes_the_agent_key_line_and_records_the_server(cx: &mut TestAppContext) {
    let (fake, hosts) = (FakeHubAgents::new(), FakeAgentHosts::default());
    let file = FakeAuthorizedKeys::with_text("ssh-ed25519 AAAAown me\n");
    hosts.connect("web", "deploy", "web.example", file.clone());
    let agents = cx
        .update(|cx| install(deps_over(&fake, &hosts), cx))
        .agents()
        .clone();
    agents.update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();

    let granted = agents.update(cx, |agents, cx| agents.grant_access(&hub(), web(), cx));
    cx.run_until_parked();

    assert!(cx.foreground_executor().block_test(granted).is_ok());
    let text = file.text();
    assert!(text.starts_with("ssh-ed25519 AAAAown me\n"), "{text}");
    assert!(text.contains("AAAAkey0 nocterm-agents"), "{text}");
    agents.read_with(cx, |agents, _| {
        let server = agents
            .server_of(&hub(), &web())
            .expect("the server is recorded");
        assert_eq!(server.host_key_fingerprint, "ssh-ed25519 AAAAfakehostkey");
    });
}

#[gpui::test]
fn access_needs_an_open_session_and_records_nothing_without_one(cx: &mut TestAppContext) {
    let fake = FakeHubAgents::new();
    let agents = cx.update(|cx| install(deps(&fake), cx)).agents().clone();
    agents.update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();

    let granted = agents.update(cx, |agents, cx| agents.grant_access(&hub(), web(), cx));
    cx.run_until_parked();

    assert!(matches!(
        cx.foreground_executor().block_test(granted),
        Err(AgentAccessError::NotConnected)
    ));
    agents.read_with(cx, |agents, _| {
        assert!(agents.server_of(&hub(), &web()).is_none())
    });
}

#[gpui::test]
fn withdrawing_access_removes_only_the_agent_key_line_and_the_record(cx: &mut TestAppContext) {
    let (fake, hosts) = (FakeHubAgents::new(), FakeAgentHosts::default());
    let file = FakeAuthorizedKeys::with_text("ssh-ed25519 AAAAown me\n");
    hosts.connect("web", "deploy", "web.example", file.clone());
    let agents = cx
        .update(|cx| install(deps_over(&fake, &hosts), cx))
        .agents()
        .clone();
    agents.update(cx, |agents, cx| agents.track(&hub(), cx));
    let granted = agents.update(cx, |agents, cx| agents.grant_access(&hub(), web(), cx));
    cx.run_until_parked();
    assert!(cx.foreground_executor().block_test(granted).is_ok());

    let withdrawn = agents.update(cx, |agents, cx| agents.withdraw_access(&hub(), &web(), cx));
    cx.run_until_parked();

    assert!(matches!(
        cx.foreground_executor().block_test(withdrawn),
        Ok(Some(_))
    ));
    assert_eq!(file.text(), "ssh-ed25519 AAAAown me\n");
    agents.read_with(cx, |agents, _| {
        assert!(agents.server_of(&hub(), &web()).is_none())
    });
}
