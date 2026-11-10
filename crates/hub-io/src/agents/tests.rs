use nocterm_hub_agents_model::{AgentDraft, ApprovalMode, ProviderId, ServerStatus};
use nocterm_hub_proto::v2;

use super::convert;

#[test]
fn an_agent_draft_goes_to_the_hub_with_its_mode_and_servers() {
    let draft = AgentDraft {
        name: "ops".into(),
        provider: Some(ProviderId::new("p1").unwrap()),
        models: vec!["m1".into()],
        approval: ApprovalMode::Ask,
        servers: vec![nocterm_hub_agents_model::ServerId::new("s1").unwrap()],
        ..AgentDraft::default()
    }
    .validate()
    .unwrap();
    let input = convert::agent_input(draft);
    assert_eq!(input.provider_id, "p1");
    assert_eq!(input.model, "m1");
    assert_eq!(input.approval, i32::from(v2::ApprovalMode::Ask));
    assert_eq!(input.server_ids, ["s1"]);
}

#[test]
fn an_agent_of_the_hub_reads_back_with_its_mode() {
    let agent = convert::agent(v2::Agent {
        id: "a1".into(),
        name: "ops".into(),
        provider_id: "p1".into(),
        model: "m1".into(),
        models: vec!["m1".into()],
        approval: v2::ApprovalMode::Ask.into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(agent.approval, ApprovalMode::Ask);
    assert!(convert::agent(v2::Agent::default()).is_err());
}

#[test]
fn a_server_reads_back_with_its_status() {
    let server = convert::server(v2::Server {
        id: "s1".into(),
        host: "prod".into(),
        port: 22,
        user: "root".into(),
        status: v2::ServerStatus::KeyRotated.into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(server.status, ServerStatus::KeyRotated);
    assert!(
        convert::server(v2::Server {
            id: "s".into(),
            port: 70_000,
            ..Default::default()
        })
        .is_err()
    );
}
