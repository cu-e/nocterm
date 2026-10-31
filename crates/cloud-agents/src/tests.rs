//! Behaviour of the Cloud agents service on fake ports.

use std::sync::Arc;

use gpui::TestAppContext;
use nocterm_cloud_agents_model::testing::{FakeCloudAgents, FakeFailure};
use nocterm_cloud_agents_model::{
    AgentId, AgentLimits, AgentScope, AgentStatus, CloudAgent, PoolId, PoolOffer,
};
use nocterm_hubs_model::{HubAddress, HubId};

use crate::{CatalogStatus, Deps, StartPhase, install};

fn hub() -> HubId {
    HubId::from(HubAddress::parse("hub.example.org").expect("address"))
}

fn agent(id: &str, name: &str) -> CloudAgent {
    CloudAgent {
        id: AgentId::new(id).expect("id"),
        scope: AgentScope::Personal,
        version: 1,
        name: name.into(),
        description: String::new(),
        icon: String::new(),
        provider: "openai_chat".into(),
        models: vec!["gpt".into()],
        limits: AgentLimits {
            cpu_milli: 500,
            mem_mib: 256,
            disk_mib: 512,
            max_parallel_runs: 1,
            idle_ttl_s: 600,
        },
        status: AgentStatus::Ready,
        pools: vec![nocterm_cloud_agents_model::PoolLabel {
            id: PoolId::new("pool-1").expect("pool"),
            name: "Shared".into(),
        }],
    }
}

fn own_runner_agent(id: &str, name: &str) -> CloudAgent {
    CloudAgent {
        pools: Vec::new(),
        ..agent(id, name)
    }
}

fn install_with(fake: &FakeCloudAgents, cx: &mut TestAppContext) -> crate::Installed {
    let fake = fake.clone();
    cx.update(|cx| {
        install(
            Deps {
                catalog: Arc::new(fake.clone()),
                launcher: Arc::new(fake.clone()),
                own_runner: Arc::new(fake.clone()),
                usage: Arc::new(fake.clone()),
                personal: Arc::new(fake.clone()),
                pins: Arc::new(fake),
            },
            cx,
        )
    })
}

#[gpui::test]
async fn tracked_hub_lists_and_follows_hints(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub(), vec![agent("a1", "Scribe")]);
    let installed = install_with(&fake, cx);
    installed
        .agents()
        .update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        assert_eq!(agents.status(&hub()), Some(CatalogStatus::Ready));
        assert_eq!(agents.agents(&hub()).len(), 1);
    });

    fake.set_agents(&hub(), vec![agent("a1", "Scribe"), agent("a2", "Coder")]);
    cx.run_until_parked();
    installed
        .agents()
        .read_with(cx, |agents, _| assert_eq!(agents.agents(&hub()).len(), 2));
}

#[gpui::test]
async fn hub_without_managed_agents_is_not_offered(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.fail(&hub(), FakeFailure::Unsupported);
    let installed = install_with(&fake, cx);
    installed
        .agents()
        .update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        assert_eq!(agents.status(&hub()), Some(CatalogStatus::Unsupported));
        assert!(!agents.is_offered(&hub()));
    });
}

#[gpui::test]
async fn failed_catalog_recovers_after_the_retry_delay(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub(), vec![agent("a1", "Scribe")]);
    fake.fail(&hub(), FakeFailure::Stale);
    let installed = install_with(&fake, cx);
    installed
        .agents()
        .update(cx, |agents, cx| agents.track(&hub(), cx));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        assert_eq!(agents.status(&hub()), Some(CatalogStatus::Failed));
    });
    fake.heal(&hub());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        assert_eq!(agents.status(&hub()), Some(CatalogStatus::Ready));
    });
}

#[gpui::test]
async fn start_reaches_ready_and_is_not_duplicated(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    let scribe = agent("a1", "Scribe");
    let installed = install_with(&fake, cx);
    let id = installed.agents().update(cx, |agents, cx| {
        let first = agents.start(&hub(), &scribe, cx);
        let again = agents.start(&hub(), &scribe, cx);
        assert_eq!(first, again);
        first
    });
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        let view = agents.start_view(id).expect("start");
        assert!(matches!(view.phase(), StartPhase::Ready(_)));
    });
    assert_eq!(fake.started().len(), 1);
}

#[gpui::test]
async fn unknown_pool_waits_for_trust_then_continues(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    let scribe = agent("a1", "Scribe");
    let pool = PoolId::new("pool-1").expect("pool");
    fake.require_trust(
        &scribe.id,
        PoolOffer {
            pool: pool.clone(),
            name: "Shared".into(),
            fingerprint: "AB CD".into(),
        },
    );
    let installed = install_with(&fake, cx);
    let id = installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &scribe, cx));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        let view = agents.start_view(id).expect("start");
        assert!(matches!(view.phase(), StartPhase::AwaitingTrust(_)));
    });
    assert!(!fake.is_pinned(&hub(), &pool));

    installed
        .agents()
        .update(cx, |agents, cx| agents.trust_pool(id, cx));
    cx.run_until_parked();
    assert!(fake.is_pinned(&hub(), &pool));
    installed.agents().read_with(cx, |agents, _| {
        let view = agents.start_view(id).expect("start");
        assert!(matches!(view.phase(), StartPhase::Ready(_)));
    });
}

#[gpui::test]
async fn refusing_the_pool_ends_the_start_without_pinning(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    let scribe = agent("a1", "Scribe");
    let pool = PoolId::new("pool-1").expect("pool");
    fake.require_trust(
        &scribe.id,
        PoolOffer {
            pool: pool.clone(),
            name: "Shared".into(),
            fingerprint: "AB CD".into(),
        },
    );
    let installed = install_with(&fake, cx);
    let id = installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &scribe, cx));
    cx.run_until_parked();
    installed
        .agents()
        .update(cx, |agents, cx| agents.refuse_pool(id, cx));
    installed.agents().read_with(cx, |agents, _| {
        let view = agents.start_view(id).expect("start");
        assert!(matches!(view.phase(), StartPhase::Failed(_)));
    });
    assert!(!fake.is_pinned(&hub(), &pool));
}

#[gpui::test]
async fn changed_pool_key_fails_the_start(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    let scribe = agent("a1", "Scribe");
    fake.script_start(
        &scribe.id,
        nocterm_cloud_agents_model::testing::StartScript::Fail(FakeFailure::PoolKeyChanged),
    );
    let installed = install_with(&fake, cx);
    let id = installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &scribe, cx));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        let view = agents.start_view(id).expect("start");
        assert!(matches!(view.phase(), StartPhase::Failed(_)));
    });
}

#[gpui::test]
async fn a_personal_agent_without_a_pool_starts_on_the_own_runner(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    let scribe = own_runner_agent("a1", "Scribe");
    let installed = install_with(&fake, cx);
    let id = installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &scribe, cx));
    cx.run_until_parked();
    installed.agents().read_with(cx, |agents, _| {
        let view = agents.start_view(id).expect("start");
        assert!(matches!(view.phase(), StartPhase::Ready(_)));
    });
    assert_eq!(fake.own_runner_started(), vec![scribe.id.clone()]);
    assert!(fake.started().is_empty(), "no pool seat is reserved");
}

#[gpui::test]
async fn an_agent_on_a_pool_never_touches_the_own_runner(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    let shared = agent("a1", "Scribe");
    let installed = install_with(&fake, cx);
    installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &shared, cx));
    cx.run_until_parked();
    assert_eq!(fake.started().len(), 1);
    assert!(fake.own_runner_started().is_empty());
}

#[gpui::test]
async fn the_lease_is_renewed_in_the_background_until_the_start_is_dismissed(
    cx: &mut TestAppContext,
) {
    let fake = FakeCloudAgents::default();
    let scribe = own_runner_agent("a1", "Scribe");
    let installed = install_with(&fake, cx);
    let id = installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &scribe, cx));
    cx.run_until_parked();
    assert!(fake.renewed().is_empty());

    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.run_until_parked();
    assert_eq!(fake.renewed().len(), 1);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(fake.renewed().len(), 2);

    installed
        .agents()
        .update(cx, |agents, cx| agents.dismiss(id, cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(fake.renewed().len(), 2, "a dismissed start stops renewing");
}

#[gpui::test]
async fn a_failed_renewal_ends_the_renewing(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.fail_renewals_after(1);
    let scribe = own_runner_agent("a1", "Scribe");
    let installed = install_with(&fake, cx);
    installed
        .agents()
        .update(cx, |agents, cx| agents.start(&hub(), &scribe, cx));
    cx.run_until_parked();
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(31));
        cx.run_until_parked();
    }
    assert_eq!(fake.renewed().len(), 1);
}
