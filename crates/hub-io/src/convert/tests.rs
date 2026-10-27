use super::*;

#[test]
fn hub_info_keeps_what_a_sign_in_screen_needs() {
    let wire = v1::HubInfo {
        name: "Team hub".into(),
        bootstrapped: true,
        password_login: true,
        oidc_providers: vec![v1::OidcProvider {
            id: "corp".into(),
            display_name: "Corporate".into(),
        }],
        ..Default::default()
    };
    let info = hub_info(&wire, true);
    assert_eq!(info.name, "Team hub");
    assert!(info.password_login && info.bootstrapped && info.signed_in);
    assert_eq!(info.oidc_providers[0].id, "corp");
    assert!(!hub_info(&wire, false).signed_in);
}

#[test]
fn an_account_lists_the_organizations_it_belongs_to() {
    let me = v1::Me {
        account_id: "a1".into(),
        username: "ada".into(),
        display_name: "Ada".into(),
        memberships: vec![
            v1::Membership {
                org: Some(v1::Org {
                    org_id: "o1".into(),
                    name: "Lab".into(),
                    created_at_ms: 0,
                }),
                ..Default::default()
            },
            v1::Membership::default(),
        ],
        ..Default::default()
    };
    let account = account(me);
    assert_eq!(account.id.as_str(), "a1");
    assert_eq!(account.orgs.len(), 1);
    assert_eq!(account.orgs[0].name, "Lab");
}

#[test]
fn removed_nodes_are_not_listed() {
    let list = v1::NodeList {
        nodes: vec![
            v1::NodeInfo {
                node_id: "n1".into(),
                name: "build".into(),
                online: true,
                sessions: 2,
                ..Default::default()
            },
            v1::NodeInfo {
                node_id: "n2".into(),
                removed: true,
                ..Default::default()
            },
        ],
    };
    let nodes = nodes(list);
    assert_eq!(nodes.len(), 1);
    assert_eq!((nodes[0].id.as_str(), nodes[0].sessions), ("n1", 2));
    assert!(nodes[0].online);
}

#[test]
fn sessions_carry_their_state_owner_and_node() {
    let wire = |state: v1::SessionState| v1::SessionInfo {
        session_id: "s1".into(),
        org_id: "o1".into(),
        node_id: "n1".into(),
        node_name: "build".into(),
        owner_account_id: "a1".into(),
        title: "make".into(),
        state: state.into(),
        created_at_ms: 42,
        node_online: true,
        ..Default::default()
    };
    let list = v1::SessionList {
        sessions: vec![
            wire(v1::SessionState::Starting),
            wire(v1::SessionState::Running),
            wire(v1::SessionState::Exited),
            wire(v1::SessionState::Lost),
            wire(v1::SessionState::Unspecified),
        ],
    };
    let states: Vec<_> = sessions(list.clone()).iter().map(|s| s.state).collect();
    assert_eq!(
        states,
        [
            HubSessionState::Starting,
            HubSessionState::Running,
            HubSessionState::Exited,
            HubSessionState::Lost,
            HubSessionState::Lost,
        ]
    );
    let first = &sessions(list)[0];
    assert_eq!(first.id.as_str(), "s1");
    assert_eq!(first.owner.as_str(), "a1");
    assert_eq!(first.node_name, "build");
    assert_eq!(first.created_at_ms, 42);
    assert!(first.node_online);
}

#[test]
fn a_state_from_a_newer_hub_is_shown_as_lost() {
    let wire = v1::SessionInfo {
        state: 99,
        ..Default::default()
    };
    let session = &sessions(v1::SessionList {
        sessions: vec![wire],
    })[0];
    assert_eq!(session.state, HubSessionState::Lost);
}
