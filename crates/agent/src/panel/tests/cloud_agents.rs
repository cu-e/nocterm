//! The Cloud tab of the new-agent popover, driven through clicks and rendering.
use super::*;
use nocterm_cloud_agents_model::{
    AgentId, AgentLimits, AgentScope, AgentStatus, ChatHost, ChatRunner, CloudAgent, PoolOffer,
    testing::{ChatFailure, FakeCloudAgents, FakeCloudChatLauncher, FakeFailure},
};
use nocterm_hubs_model::{
    HubAddress, HubId, HubsSettings,
    testing::{FakeAccountDevices, FakeAdminKeys, FakeAgentAccess, FakeHub, FakeServer},
};

const HOME: &str = "home.test:7443";
const WORK: &str = "work.test:7443";

fn hub(address: &str) -> HubId {
    HubId::from(HubAddress::parse(address).unwrap())
}

fn agent(id: &str, name: &str, scope: AgentScope, status: AgentStatus) -> CloudAgent {
    CloudAgent {
        id: AgentId::new(id).unwrap(),
        scope,
        version: 1,
        name: name.into(),
        description: String::new(),
        icon: String::new(),
        provider: "openai_chat".into(),
        models: vec!["gpt-x".into()],
        limits: AgentLimits {
            cpu_milli: 500,
            mem_mib: 256,
            disk_mib: 512,
            max_parallel_runs: 1,
            idle_ttl_s: 600,
        },
        status,
        pools: vec![nocterm_cloud_agents_model::PoolLabel {
            id: nocterm_cloud_agents_model::PoolId::new("pool-1").unwrap(),
            name: "Shared".into(),
        }],
    }
}

/// Signed-in hubs over fakes, and the access a window of `audience` is given.
fn access(
    fake: &FakeCloudAgents,
    hubs: &[&str],
    only: Option<&str>,
    cx: &mut TestAppContext,
) -> crate::CloudAgentsAccess {
    let mut fake_hub = FakeHub::new();
    let mut records = Vec::new();
    for (seed, address) in hubs.iter().enumerate() {
        let server = FakeServer::new(address, seed as u8 + 1).with_user("ada", "secret", "lab");
        records.push(server.pinned_record(address.split('.').next().unwrap()));
        fake_hub = fake_hub.with_server(server);
    }
    for address in hubs {
        fake_hub.sign_in_as(&hub(address), "ada");
    }
    let admin = FakeAdminKeys::alongside(&fake_hub);
    let access = cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default())
            .with(HubsSettings { servers: records })
            .install(cx);
        let registry = nocterm_hubs::install(
            nocterm_hubs::Deps {
                trust: Arc::new(fake_hub.clone()),
                sign_in: Arc::new(fake_hub.clone()),
                admin_keys: Arc::new(admin),
                devices: Arc::new(FakeAccountDevices::default()),
                agent_trust: Arc::new(FakeAgentAccess::default()),
                agent_grants: Arc::new(FakeAgentAccess::default()),
                account_keys: Arc::new(nocterm_hubs_model::testing::FakeAccountKeys::alongside(
                    &fake_hub, "password",
                )),
            },
            settings,
            cx,
        );
        let service = nocterm_cloud_agents::install(
            nocterm_cloud_agents::Deps {
                catalog: Arc::new(fake.clone()),
                launcher: Arc::new(fake.clone()),
                own_runner: Arc::new(fake.clone()),
                seats: Arc::new(fake.clone()),
                usage: Arc::new(fake.clone()),
                personal: Arc::new(fake.clone()),
                pins: Arc::new(fake.clone()),
            },
            cx,
        );
        for address in hubs {
            service
                .agents()
                .update(cx, |agents, cx| agents.track(&hub(address), cx));
        }
        let access =
            crate::CloudAgentsAccess::new(service.agents().clone(), registry.hubs().clone());
        match only {
            Some(address) => access.for_hub_window(&hub(address)),
            None => access,
        }
    });
    cx.run_until_parked();
    access
}

/// A second window over the fixture's installation that offers `access`.
fn cloud_window(
    f: &Fixture,
    access: crate::CloudAgentsAccess,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<AgentPanel>) {
    let (handle, _workspace, panel, _terminal) = cx.update(|cx| {
        let installed = f.installation.installed.with_cloud_agents(Some(access));
        open_host(
            &f._directory,
            &installed,
            &f.installation.layout,
            &f.access,
            cx,
        )
    });
    cx.run_until_parked();
    (handle, panel)
}

fn open_cloud_tab(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-new-thread", cx);
        window.render_frame(cx);
        window.click("new-agent-tab-cloud", cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn shown(
    handle: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> bool {
    let id = id.into();
    cx.update_window(handle, |_, window, _| window.try_find(id).is_some())
        .unwrap()
}

#[gpui_kit::test]
fn the_cloud_tab_is_hidden_while_no_hub_offers_agents(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    fake.fail(&hub(HOME), FakeFailure::Unsupported);
    let (handle, _) = cloud_window(&f, access(&fake, &[HOME], None, cx), cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-new-thread", cx);
        window.render_frame(cx);
        assert!(window.try_find("new-agent-tab-cloud").is_none());
        assert!(window.try_find("new-agent-codex").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_cloud_agent_starts_through_its_steps_and_reports_ready(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    fake.set_agents(
        &hub(HOME),
        vec![agent(
            "a1",
            "Scribe",
            AgentScope::Personal,
            AgentStatus::Ready,
        )],
    );
    let (handle, _) = cloud_window(&f, access(&fake, &[HOME], None, cx), cx);
    open_cloud_tab(handle, cx);
    assert!(shown(handle, format!("cloud-agent-{HOME}-a1"), cx));
    assert!(shown(handle, "cloud-manage-agents", cx));
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("cloud-agent-{HOME}-a1"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(fake.started().len(), 1);
    assert!(shown(handle, "cloud-dismiss-StartId(1)", cx));
}

#[gpui_kit::test]
fn an_unpinned_pool_asks_for_trust_before_the_start_continues(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    let id = AgentId::new("a1").unwrap();
    fake.set_agents(
        &hub(HOME),
        vec![agent(
            "a1",
            "Scribe",
            AgentScope::Personal,
            AgentStatus::NeedsPoolTrust,
        )],
    );
    fake.require_trust(
        &id,
        PoolOffer {
            pool: nocterm_cloud_agents_model::PoolId::new("p1").unwrap(),
            name: "Shared".into(),
            fingerprint: "AB12 CD34".into(),
        },
    );
    let (handle, _) = cloud_window(&f, access(&fake, &[HOME], None, cx), cx);
    open_cloud_tab(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("cloud-agent-{HOME}-a1"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!fake.is_pinned(
        &hub(HOME),
        &nocterm_cloud_agents_model::PoolId::new("p1").unwrap()
    ));
    assert!(shown(handle, "cloud-trust-StartId(1)", cx));
    cx.update_window(handle, |_, window, cx| {
        window.click("cloud-trust-StartId(1)", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(fake.is_pinned(
        &hub(HOME),
        &nocterm_cloud_agents_model::PoolId::new("p1").unwrap()
    ));
}

#[gpui_kit::test]
fn a_hub_window_lists_only_its_own_agents_and_the_local_window_every_hub(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    fake.set_agents(
        &hub(HOME),
        vec![agent(
            "a1",
            "Scribe",
            AgentScope::Personal,
            AgentStatus::Ready,
        )],
    );
    fake.set_agents(
        &hub(WORK),
        vec![agent(
            "b1",
            "Auditor",
            AgentScope::Server,
            AgentStatus::Sleeping,
        )],
    );
    let (all, _) = cloud_window(&f, access(&fake, &[HOME, WORK], None, cx), cx);
    open_cloud_tab(all, cx);
    assert!(shown(all, format!("cloud-agent-{HOME}-a1"), cx));
    assert!(shown(all, format!("cloud-agent-{WORK}-b1"), cx));

    let (only, _) = cloud_window(&f, access(&fake, &[HOME, WORK], Some(WORK), cx), cx);
    open_cloud_tab(only, cx);
    assert!(!shown(only, format!("cloud-agent-{HOME}-a1"), cx));
    assert!(shown(only, format!("cloud-agent-{WORK}-b1"), cx));
}

#[gpui_kit::test]
fn a_key_that_is_missing_blocks_the_row(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    fake.set_agents(
        &hub(HOME),
        vec![agent(
            "a1",
            "Scribe",
            AgentScope::Personal,
            AgentStatus::NeedsKey,
        )],
    );
    let (handle, _) = cloud_window(&f, access(&fake, &[HOME], None, cx), cx);
    open_cloud_tab(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("cloud-agent-{HOME}-a1"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(fake.started().is_empty());
}

fn chat_access(
    fake: &FakeCloudAgents,
    launcher: &FakeCloudChatLauncher,
    cx: &mut TestAppContext,
) -> crate::CloudAgentsAccess {
    access(fake, &[HOME], None, cx).with_chats(Arc::new(launcher.clone()))
}

fn ready_scribe() -> FakeCloudAgents {
    let fake = FakeCloudAgents::default();
    fake.set_agents(
        &hub(HOME),
        vec![agent(
            "a1",
            "Scribe",
            AgentScope::Personal,
            AgentStatus::Ready,
        )],
    );
    fake
}

fn click(handle: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn a_ready_start_opens_the_chat_of_its_agent_in_the_window_that_pressed_start(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let launcher = FakeCloudChatLauncher::default();
    let (handle, _) = cloud_window(&f, chat_access(&ready_scribe(), &launcher, cx), cx);
    open_cloud_tab(handle, cx);
    click(handle, format!("cloud-agent-{HOME}-a1"), cx);
    assert_eq!(
        launcher.created(),
        vec![ChatHost::Agent {
            agent: AgentId::new("a1").unwrap(),
            model: "gpt-x".into(),
        }]
    );
    assert_eq!(launcher.opened().len(), 1);
    assert!(!shown(handle, "cloud-dismiss-StartId(1)", cx));
    assert!(!shown(handle, "cloud-open-chat-StartId(1)", cx));
}

#[gpui_kit::test]
fn a_ready_online_only_agent_opens_a_chat_that_leases_its_key_to_the_own_runner(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    let mut mine = agent("a1", "Scribe", AgentScope::Personal, AgentStatus::Ready);
    mine.pools.clear();
    fake.set_agents(&hub(HOME), vec![mine]);
    let launcher = FakeCloudChatLauncher::default();
    launcher.set_runners(vec![ChatRunner {
        id: [9; 16],
        label: "laptop".into(),
    }]);
    let (handle, _) = cloud_window(&f, chat_access(&fake, &launcher, cx), cx);
    open_cloud_tab(handle, cx);
    cx.run_until_parked();
    click(handle, format!("cloud-agent-{HOME}-a1"), cx);
    assert_eq!(
        launcher.created(),
        vec![ChatHost::OwnAgent {
            runner: ChatRunner {
                id: [9; 16],
                label: "laptop".into(),
            },
            agent: AgentId::new("a1").unwrap(),
            model: "gpt-x".into(),
        }]
    );
    assert_eq!(launcher.opened().len(), 1);
}

#[gpui_kit::test]
fn another_window_offers_open_chat_and_a_slow_runner_asks_to_try_again(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let launcher = FakeCloudChatLauncher::default();
    launcher.fail_creating(ChatFailure::RunnerNotReady);
    let access = chat_access(&ready_scribe(), &launcher, cx);
    let (pressed, _) = cloud_window(&f, access.clone(), cx);
    let (other, _) = cloud_window(&f, access, cx);
    open_cloud_tab(pressed, cx);
    open_cloud_tab(other, cx);
    click(pressed, format!("cloud-agent-{HOME}-a1"), cx);
    assert_eq!(
        launcher.created().len(),
        1,
        "only the pressing window opens"
    );
    assert!(shown(pressed, "cloud-chat-notice", cx));
    assert!(shown(other, "cloud-open-chat-StartId(1)", cx));

    launcher.heal();
    click(other, "cloud-open-chat-StartId(1)", cx);
    assert_eq!(launcher.created().len(), 2);
    assert_eq!(launcher.opened().len(), 1);
    assert!(!shown(other, "cloud-open-chat-StartId(1)", cx));
}

#[gpui_kit::test]
fn a_new_cloud_chat_opens_on_the_chosen_runner(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let launcher = FakeCloudChatLauncher::default();
    let runner = ChatRunner {
        id: [7; 16],
        label: "laptop".into(),
    };
    launcher.set_runners(vec![runner.clone()]);
    let (handle, _) = cloud_window(&f, chat_access(&ready_scribe(), &launcher, cx), cx);
    open_cloud_tab(handle, cx);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    click(handle, format!("cloud-chat-new-{HOME}-laptop"), cx);
    assert_eq!(launcher.created(), vec![ChatHost::Runner(runner)]);
    assert_eq!(launcher.opened().len(), 1);
}

#[gpui_kit::test]
fn an_earlier_cloud_chat_in_the_history_opens_from_its_row(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let launcher = FakeCloudChatLauncher::default();
    launcher.add_history("Release notes");
    let (handle, _) = cloud_window(&f, chat_access(&ready_scribe(), &launcher, cx), cx);
    open_cloud_tab(handle, cx);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(shown(handle, format!("cloud-chat-history-{HOME}"), cx));
    click(handle, format!("cloud-chat-entry-{HOME}-0"), cx);
    assert!(launcher.created().is_empty());
    assert_eq!(launcher.opened().len(), 1);
}

#[gpui_kit::test]
fn a_failed_start_shows_one_entry_under_its_agent_and_can_be_retried(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    let scribe = agent("a1", "Scribe", AgentScope::Personal, AgentStatus::Ready);
    for _ in 0..2 {
        fake.script_start(
            &scribe.id,
            nocterm_cloud_agents_model::testing::StartScript::Fail(FakeFailure::ProvisioningFailed),
        );
    }
    fake.set_agents(&hub(HOME), vec![scribe]);
    let (handle, _) = cloud_window(&f, access(&fake, &[HOME], None, cx), cx);
    open_cloud_tab(handle, cx);
    click(handle, format!("cloud-agent-{HOME}-a1"), cx);
    click(handle, format!("cloud-agent-{HOME}-a1"), cx);

    assert_eq!(fake.started().len(), 2);
    assert!(!shown(handle, "cloud-start-StartId(1)", cx));
    assert!(shown(handle, "cloud-start-StartId(2)", cx));
    assert!(shown(handle, "cloud-retry-StartId(2)", cx));
    assert!(shown(handle, "cloud-dismiss-StartId(2)", cx));
    assert!(shown(handle, format!("cloud-agent-{HOME}-a1"), cx));

    click(handle, "cloud-retry-StartId(2)", cx);
    assert_eq!(fake.started().len(), 3);
    assert!(!shown(handle, "cloud-retry-StartId(3)", cx));
    assert!(shown(handle, "cloud-open-chat-StartId(3)", cx));
}

#[gpui_kit::test]
fn a_start_whose_agent_row_is_gone_stays_listed_on_top(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    let scribe = agent("a1", "Scribe", AgentScope::Personal, AgentStatus::Ready);
    fake.script_start(
        &scribe.id,
        nocterm_cloud_agents_model::testing::StartScript::Fail(FakeFailure::ProvisioningFailed),
    );
    fake.set_agents(&hub(HOME), vec![scribe]);
    let (handle, _) = cloud_window(&f, access(&fake, &[HOME], None, cx), cx);
    open_cloud_tab(handle, cx);
    click(handle, format!("cloud-agent-{HOME}-a1"), cx);
    fake.set_agents(&hub(HOME), Vec::new());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!shown(handle, format!("cloud-agent-{HOME}-a1"), cx));
    assert!(shown(handle, "cloud-start-StartId(1)", cx));
}

#[gpui_kit::test]
fn a_seat_is_stopped_or_removed_from_its_row_after_the_chat_opened(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fake = ready_scribe();
    let launcher = FakeCloudChatLauncher::default();
    let (handle, _) = cloud_window(&f, chat_access(&fake, &launcher, cx), cx);
    open_cloud_tab(handle, cx);
    assert!(
        !shown(handle, format!("cloud-stop-{HOME}-a1"), cx),
        "no seat yet"
    );
    click(handle, format!("cloud-agent-{HOME}-a1"), cx);
    assert_eq!(launcher.opened().len(), 1);
    assert!(
        !shown(handle, "cloud-dismiss-StartId(1)", cx),
        "the opened chat dismissed the start"
    );
    open_cloud_tab(handle, cx);

    click(handle, format!("cloud-remove-{HOME}-a1"), cx);
    assert!(fake.removed().is_empty(), "asks before removing");
    assert!(shown(handle, format!("cloud-remove-confirm-{HOME}-a1"), cx));
    click(handle, format!("cloud-remove-keep-{HOME}-a1"), cx);
    assert!(fake.removed().is_empty());

    click(handle, format!("cloud-stop-{HOME}-a1"), cx);
    assert_eq!(fake.stopped().len(), 1);

    click(handle, format!("cloud-remove-{HOME}-a1"), cx);
    click(handle, format!("cloud-remove-confirm-{HOME}-a1"), cx);
    assert_eq!(fake.removed().len(), 1);
    assert!(
        !shown(handle, format!("cloud-stop-{HOME}-a1"), cx),
        "a removed seat has no actions"
    );
}

#[gpui_kit::test]
fn a_seat_of_an_earlier_session_has_its_actions_and_an_own_runner_agent_has_none(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let fake = FakeCloudAgents::default();
    let mut mine = agent("a2", "Offline", AgentScope::Personal, AgentStatus::Ready);
    mine.pools.clear();
    fake.set_agents(
        &hub(HOME),
        vec![
            agent("a1", "Scribe", AgentScope::Personal, AgentStatus::Ready),
            mine,
        ],
    );
    fake.set_seated(&AgentId::new("a1").unwrap());
    fake.set_seated(&AgentId::new("a2").unwrap());
    let launcher = FakeCloudChatLauncher::default();
    let (handle, _) = cloud_window(&f, chat_access(&fake, &launcher, cx), cx);
    open_cloud_tab(handle, cx);
    assert!(shown(handle, format!("cloud-stop-{HOME}-a1"), cx));
    assert!(!shown(handle, format!("cloud-stop-{HOME}-a2"), cx));
}
