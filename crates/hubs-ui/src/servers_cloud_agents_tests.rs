//! The Cloud agents tab lists the agents of the account and manages the personal ones.
use std::sync::Arc;

use super::{ServersPage, item};
use crate::test_support::{Fixture, fixture_over, hub_id, pinned_record, server};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_cloud_agents_model::{
    AgentId, AgentLimits, AgentScope, AgentStatus, CloudAgent, PoolChoice, PoolId,
    testing::{FakeCloudAgents, FakeFailure},
};
use nocterm_hubs_model::Environment;
use nocterm_hubs_model::testing::FakeHub;
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;

const KEY: &str = "hub.home:7443";
const CLOUD_AGENTS: usize = 4;

fn agent(id: &str, name: &str, scope: AgentScope) -> CloudAgent {
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
        status: AgentStatus::Ready,
        pools: vec![nocterm_cloud_agents_model::PoolLabel {
            id: nocterm_cloud_agents_model::PoolId::new("pool-1").unwrap(),
            name: "Shared".into(),
        }],
    }
}

fn page(cx: &mut TestAppContext, fake: &FakeCloudAgents) -> Fixture {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let setup = fixture_over(
        cx,
        config,
        Environment::Local,
        FakeHub::new().with_server(server()),
        vec![pinned_record()],
        Some("ada"),
    );
    let hubs = setup.installed.hubs().clone();
    let fake = fake.clone();
    let (window, _) = cx.update(|cx| {
        let service = nocterm_cloud_agents::install(
            nocterm_cloud_agents::Deps {
                catalog: Arc::new(fake.clone()),
                launcher: Arc::new(fake.clone()),
                own_runner: Arc::new(fake.clone()),
                seats: Arc::new(fake.clone()),
                usage: Arc::new(fake.clone()),
                personal: Arc::new(fake.clone()),
                pins: Arc::new(fake),
            },
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| ServersPage::new(hubs, Some(service.agents().clone()), cx))
        })
        .unwrap()
    });
    let page = Fixture { window, ..setup };
    page.render(cx);
    cx.update_window(page.window, |_, window, cx| {
        window.within("servers-tabs").click(CLOUD_AGENTS, cx);
    })
    .unwrap();
    cx.run_until_parked();
    page.render(cx);
    page
}

fn type_into(page: &Fixture, field: &str, text: &str, cx: &mut TestAppContext) {
    page.click(item(field, KEY), cx);
    page.type_text(text, cx);
}

#[gpui_kit::test]
fn only_personal_agents_can_be_edited_or_deleted_here(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(
        &hub_id(),
        vec![
            agent("mine", "Scribe", AgentScope::Personal),
            agent("theirs", "Auditor", AgentScope::Server),
        ],
    );
    let page = page(cx, &fake);
    assert!(page.shows(item("cloud-agent-mine", KEY), cx));
    assert!(page.shows(item("cloud-agent-edit-mine", KEY), cx));
    assert!(page.shows(item("cloud-agent-theirs", KEY), cx));
    assert!(!page.shows(item("cloud-agent-edit-theirs", KEY), cx));
    assert!(!page.shows(item("cloud-agent-delete-theirs", KEY), cx));
    assert!(page.shows(item("cloud-agents-web", KEY), cx));
}

#[gpui_kit::test]
fn a_server_without_managed_agents_says_so(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.fail(&hub_id(), FakeFailure::Unsupported);
    let page = page(cx, &fake);
    assert!(page.shows(item("cloud-agents-unsupported", KEY), cx));
}

#[gpui_kit::test]
fn a_personal_agent_is_saved_with_its_key_working_offline_by_default(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    fake.set_pools(
        &hub_id(),
        vec![PoolChoice {
            pool: PoolId::new("p1").unwrap(),
            name: "Shared".into(),
            accepts_personal: true,
        }],
    );
    let page = page(cx, &fake);
    page.click(item("cloud-agents-new", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    type_into(&page, "cloud-agent-name", "Scribe", cx);
    type_into(&page, "cloud-agent-model", "gpt-x", cx);
    type_into(&page, "cloud-agent-key", "sk-secret", cx);
    page.click(item("cloud-agent-save", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    let saved = fake.saved();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Scribe");
    assert!(saved[0].work_offline);
    assert!(!page.shows(item("cloud-agent-form", KEY), cx));
}

#[gpui_kit::test]
fn the_instructions_of_an_agent_working_offline_are_saved_with_it(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    fake.set_pools(
        &hub_id(),
        vec![PoolChoice {
            pool: PoolId::new("p1").unwrap(),
            name: "Shared".into(),
            accepts_personal: true,
        }],
    );
    let page = page(cx, &fake);
    page.click(item("cloud-agents-new", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    type_into(&page, "cloud-agent-name", "Scribe", cx);
    type_into(&page, "cloud-agent-model", "gpt-x", cx);
    type_into(&page, "cloud-agent-key", "sk-secret", cx);
    type_into(&page, "cloud-agent-instructions", "Answer briefly.", cx);
    page.click(item("cloud-agent-save", KEY), cx);
    cx.run_until_parked();
    assert_eq!(fake.saved()[0].instructions, "Answer briefly.");
}

#[gpui_kit::test]
fn a_form_missing_its_key_is_refused_before_anything_is_sent(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    let page = page(cx, &fake);
    page.click(item("cloud-agents-new", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    type_into(&page, "cloud-agent-name", "Scribe", cx);
    type_into(&page, "cloud-agent-model", "gpt-x", cx);
    page.click(item("cloud-agent-save", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(fake.saved().is_empty());
    assert!(page.shows(item("cloud-agents-problem", KEY), cx));
}

#[gpui_kit::test]
fn a_forgotten_pool_key_leaves_the_list_and_is_no_longer_trusted(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    let (kept, dropped) = (PoolId::new("p1").unwrap(), PoolId::new("p2").unwrap());
    fake.seed_pin(&hub_id(), &kept, "AA:AA");
    fake.seed_pin(&hub_id(), &dropped, "BB:BB");
    let page = page(cx, &fake);
    assert!(page.shows(item("cloud-agents-pinned-p1", KEY), cx));
    assert!(page.shows(item("cloud-agents-pinned-p2", KEY), cx));
    page.click(item("cloud-agents-forget-p2", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(!page.shows(item("cloud-agents-pinned-p2", KEY), cx));
    assert!(page.shows(item("cloud-agents-pinned-p1", KEY), cx));
    assert!(!fake.is_pinned(&hub_id(), &dropped));
    assert!(fake.is_pinned(&hub_id(), &kept));
}

#[gpui_kit::test]
fn usage_of_own_agents_is_listed_per_model(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    let sample = |model: &str, spend_micros| nocterm_cloud_agents_model::UsageSample {
        model: model.into(),
        runs: 1,
        input_tokens: 10,
        output_tokens: 2,
        spend_micros,
    };
    fake.set_usage(&hub_id(), vec![sample("gpt-x", 500), sample("", 5)]);
    let page = page(cx, &fake);
    assert!(page.shows(item("cloud-agents-usage", KEY), cx));
    assert!(page.shows(item("cloud-agents-usage-gpt-x", KEY), cx));
    assert!(page.shows(item("cloud-agents-usage-not attributed", KEY), cx));
}

#[gpui_kit::test]
fn a_hub_without_usage_shows_no_table(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    let page = page(cx, &fake);
    assert!(!page.shows(item("cloud-agents-usage", KEY), cx));
}

#[gpui_kit::test]
fn instructions_typed_before_switching_to_online_only_are_not_sent(cx: &mut TestAppContext) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(&hub_id(), Vec::new());
    let page = page(cx, &fake);
    page.click(item("cloud-agents-new", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    type_into(&page, "cloud-agent-name", "Scribe", cx);
    type_into(&page, "cloud-agent-model", "gpt-x", cx);
    type_into(&page, "cloud-agent-key", "sk-secret", cx);
    type_into(&page, "cloud-agent-instructions", "Answer briefly.", cx);
    page.click(item("cloud-agent-offline", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(!page.shows(item("cloud-agent-instructions", KEY), cx));
    page.click(item("cloud-agent-save", KEY), cx);
    cx.run_until_parked();
    let saved = fake.saved();
    assert_eq!(saved.len(), 1);
    assert!(!saved[0].work_offline);
    assert_eq!(saved[0].instructions, "");
}

#[gpui_kit::test]
fn editing_an_agent_starts_with_empty_instructions_that_are_saved_as_typed(
    cx: &mut TestAppContext,
) {
    let fake = FakeCloudAgents::default();
    fake.set_agents(
        &hub_id(),
        vec![agent("mine", "Scribe", AgentScope::Personal)],
    );
    fake.set_pools(
        &hub_id(),
        vec![PoolChoice {
            pool: PoolId::new("p1").unwrap(),
            name: "Shared".into(),
            accepts_personal: true,
        }],
    );
    let page = page(cx, &fake);
    page.click(item("cloud-agent-edit-mine", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    type_into(&page, "cloud-agent-key", "sk-secret", cx);
    page.click(item("cloud-agent-save", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    assert_eq!(fake.saved()[0].instructions, "", "empty removes them");

    page.click(item("cloud-agent-edit-mine", KEY), cx);
    cx.run_until_parked();
    page.render(cx);
    type_into(&page, "cloud-agent-key", "sk-secret", cx);
    type_into(&page, "cloud-agent-instructions", "Answer briefly.", cx);
    page.click(item("cloud-agent-save", KEY), cx);
    cx.run_until_parked();
    assert_eq!(fake.saved()[0].instructions, "Answer briefly.");
}
