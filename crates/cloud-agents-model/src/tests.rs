use nocterm_hubs_model::{HubAddress, HubColor, HubId};

use crate::*;

fn hub(host: &str) -> HubId {
    HubId::from(HubAddress::parse(host).unwrap())
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
        models: vec!["gpt-4o".into()],
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

fn org(name: &str) -> AgentScope {
    AgentScope::Org {
        id: format!("org-{name}"),
        name: name.into(),
    }
}

fn catalog(host: &str, name: &str, agents: Vec<CloudAgent>) -> HubAgents {
    HubAgents {
        hub: hub(host),
        name: name.into(),
        color: HubColor::Blue,
        agents,
    }
}

fn titles(sections: &[Section]) -> Vec<String> {
    sections.iter().map(|s| s.title.text()).collect()
}

#[test]
fn a_hub_lists_personal_then_organizations_by_name_then_the_server() {
    let hubs = [catalog(
        "work.example",
        "Work",
        vec![
            agent("1", "Srv", AgentScope::Server, AgentStatus::Ready),
            agent("2", "B", org("Zeta"), AgentStatus::Ready),
            agent("3", "Mine", AgentScope::Personal, AgentStatus::Ready),
            agent("4", "A", org("alpha"), AgentStatus::Ready),
        ],
    )];
    let sections = arrange(&hubs, &Audience::AllHubs, "");
    assert_eq!(
        titles(&sections),
        ["Personal", "alpha", "Zeta", "Server Work"]
    );
}

#[test]
fn usable_agents_come_before_blocked_ones_then_by_name() {
    let hubs = [catalog(
        "work.example",
        "Work",
        vec![
            agent("1", "Zed", AgentScope::Personal, AgentStatus::Ready),
            agent("2", "Amy", AgentScope::Personal, AgentStatus::Disabled),
            agent("3", "Bob", AgentScope::Personal, AgentStatus::Sleeping),
            agent("4", "Cat", AgentScope::Personal, AgentStatus::NeedsKey),
            agent("5", "Abe", AgentScope::Personal, AgentStatus::Ready),
        ],
    )];
    let sections = arrange(&hubs, &Audience::AllHubs, "");
    let names: Vec<_> = sections[0].agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Abe", "Zed", "Bob", "Cat", "Amy"]);
}

#[test]
fn a_hub_window_shows_only_its_own_agents_and_the_local_window_groups_by_hub() {
    let hubs = [
        catalog(
            "one.example",
            "One",
            vec![agent("1", "A", AgentScope::Server, AgentStatus::Ready)],
        ),
        catalog(
            "two.example",
            "Two",
            vec![agent("2", "B", AgentScope::Server, AgentStatus::Ready)],
        ),
    ];
    let only = arrange(&hubs, &Audience::Hub(hub("two.example")), "");
    assert_eq!(titles(&only), ["Server Two"]);
    let all = arrange(&hubs, &Audience::AllHubs, "");
    assert_eq!(titles(&all), ["Server One", "Server Two"]);
    assert_eq!(all[1].hub, hub("two.example"));
}

#[test]
fn search_matches_every_word_across_name_model_and_description_and_drops_empty_sections() {
    let mut helper = agent("1", "Code helper", AgentScope::Personal, AgentStatus::Ready);
    helper.models = vec!["claude-sonnet".into()];
    let other = agent("2", "Writer", org("Acme"), AgentStatus::Ready);
    let hubs = [catalog("w.example", "W", vec![helper, other])];
    let found = arrange(&hubs, &Audience::AllHubs, "HELP sonnet");
    assert_eq!(titles(&found), ["Personal"]);
    assert!(arrange(&hubs, &Audience::AllHubs, "nothing here").is_empty());
}

#[test]
fn only_a_missing_key_or_a_switched_off_agent_block_starting() {
    for status in [
        AgentStatus::Ready,
        AgentStatus::Sleeping,
        AgentStatus::Starting,
        AgentStatus::NoCapacity,
        AgentStatus::NeedsPoolTrust,
    ] {
        assert!(status.can_start(), "{status:?}");
    }
    assert!(!AgentStatus::Disabled.can_start());
    assert!(!AgentStatus::NeedsKey.can_start());
}

#[test]
fn organization_and_server_agents_are_managed_in_the_web_panel() {
    assert!(AgentScope::Personal.editable_here());
    assert!(!org("A").editable_here());
    assert!(!AgentScope::Server.editable_here());
    assert_eq!(web_panel_url(&hub("hub.example")), "https://hub.example/");
}

#[test]
fn a_pool_key_change_is_not_transient_but_a_full_host_is() {
    assert!(CloudAgentError::Refused(Refusal::HostCapacity).is_transient());
    assert!(
        !CloudAgentError::PoolKeyChanged {
            pinned: "a".into(),
            offered: "b".into()
        }
        .is_transient()
    );
}

#[test]
fn only_a_personal_agent_without_pools_runs_on_the_own_runner() {
    let mut mine = agent("a1", "Mine", AgentScope::Personal, AgentStatus::Ready);
    assert_eq!(mine.placement(), Placement::OwnRunner);
    mine.pools.push(PoolLabel {
        id: PoolId::new("p1").unwrap(),
        name: "Shared".into(),
    });
    assert_eq!(mine.placement(), Placement::Pool);
    let org = agent(
        "a2",
        "Org",
        AgentScope::Org {
            id: "o1".into(),
            name: "Acme".into(),
        },
        AgentStatus::Ready,
    );
    assert_eq!(org.placement(), Placement::Pool);
    let server = agent("a3", "Srv", AgentScope::Server, AgentStatus::Ready);
    assert_eq!(server.placement(), Placement::Pool);
}
