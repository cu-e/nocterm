use super::*;

#[test]
fn hub_info_keeps_what_a_sign_in_screen_needs() {
    let wire = v2::HubInfo {
        name: "Team hub".into(),
        needs_setup: false,
        password_login: true,
        oidc_providers: vec![v2::OidcProvider {
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
fn an_account_says_whether_it_administers_the_hub() {
    let me = v2::Me {
        account_id: "a1".into(),
        username: "ada".into(),
        display_name: "Ada".into(),
        admin: true,
        ..Default::default()
    };
    let account = account(me);
    assert_eq!(account.id.as_str(), "a1");
    assert!(account.admin);
}
