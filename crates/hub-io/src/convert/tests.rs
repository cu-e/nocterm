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
