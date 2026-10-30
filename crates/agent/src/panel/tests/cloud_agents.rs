//! The Cloud tab of the new-agent popover, driven through clicks and rendering.
use super::*;
use nocterm_cloud_agents_model::{
    AgentId, AgentLimits, AgentScope, AgentStatus, CloudAgent, PoolOffer,
    testing::{FakeCloudAgents, FakeFailure},
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
        pools: Vec::new(),
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
            },
            settings,
            cx,
        );
        let service = nocterm_cloud_agents::install(
            nocterm_cloud_agents::Deps {
                catalog: Arc::new(fake.clone()),
                launcher: Arc::new(fake.clone()),
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
