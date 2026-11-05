//! The Agent access (SSH) tab shows the agent CA, the servers that trust it and the chat
//! accesses, and changes them through the service.
use super::*;
use nocterm_hubs_model::{
    AgentAccessError, AgentAccessGrant, AgentCa, HostChoice, ServerTrust, TrustedServer,
};

const AGENTS: usize = 3;

fn trusted(connection: &str, state: ServerTrust) -> TrustedServer {
    TrustedServer {
        connection: connection.to_owned(),
        user: "root".into(),
        host: format!("{connection}.example"),
        state,
        needs_ca_update: false,
        added_at_ms: 1_700_000_000_000,
        added_by_device: "Laptop".into(),
        last_verified_at_ms: None,
    }
}

fn grant(id: &str) -> AgentAccessGrant {
    AgentAccessGrant {
        grant_id: id.to_owned(),
        agent_id: "agent-1".into(),
        chat_id: "ab12".into(),
        targets: vec!["web".into()],
        issued_at_ms: 1_700_000_000_000,
        expires_at_ms: 1_700_086_400_000,
    }
}

fn page_with_access(cx: &mut TestAppContext) -> Fixture {
    let page = page(cx, vec![pinned_record()]);
    page.agent_access.set_ca(AgentCa {
        fingerprint: "SHA256:abc".into(),
        created_at_ms: 1_700_000_000_000,
    });
    page.agent_access.set_servers(vec![
        trusted("web", ServerTrust::Trusted),
        trusted("db", ServerTrust::Trusted),
    ]);
    page.agent_access.set_grants(vec![grant("g1")]);
    page
}

#[gpui_kit::test]
fn the_agent_access_tab_lists_the_ca_the_servers_and_the_accesses(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    assert!(!page.shows("agent-access-tab", cx));
    open_tab(&page, AGENTS, cx);

    assert!(page.shows("agent-access-tab", cx));
    assert!(page.shows("agent-access-ca", cx));
    assert!(page.shows(item("agent-server", "web"), cx));
    assert!(page.shows(item("agent-server", "db"), cx));
    assert!(page.shows(item("agent-grant", "g1"), cx));

    open_tab(&page, OVERVIEW, cx);
    assert!(!page.shows("agent-access-tab", cx));
}

#[gpui_kit::test]
fn the_agent_access_tab_asks_to_sign_in_first(cx: &mut TestAppContext) {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let page = page_over(cx, config, vec![pinned_record()], None);
    open_tab(&page, AGENTS, cx);

    assert!(page.shows(item("server-tab-signed-out", KEY), cx));
    assert!(!page.shows("agent-access-tab", cx));
}

#[gpui_kit::test]
fn denying_an_unreachable_server_leaves_its_removal_pending(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    page.agent_access.set_unreachable("web", true);
    open_tab(&page, AGENTS, cx);

    page.click(item("agent-deny", "web"), cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(page.shows(item("agent-server", "web"), cx), "still listed");
    assert!(page.shows(item("agent-server-pending", "web"), cx));
    assert!(page.shows("agent-access-notice", cx));
}

#[gpui_kit::test]
fn denying_a_reachable_server_removes_it_from_the_list(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    open_tab(&page, AGENTS, cx);

    page.click(item("agent-deny", "web"), cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(!page.shows(item("agent-server", "web"), cx));
    assert!(page.shows(item("agent-server", "db"), cx));
}

#[gpui_kit::test]
fn the_selected_servers_are_denied_together(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    open_tab(&page, AGENTS, cx);

    page.click(item("agent-select", "web"), cx);
    page.click(item("agent-select", "db"), cx);
    page.click("agent-deny-selected", cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(!page.shows(item("agent-server", "web"), cx));
    assert!(!page.shows(item("agent-server", "db"), cx));
    assert!(page.shows("agent-servers-empty", cx));
}

#[gpui_kit::test]
fn a_server_whose_line_vanished_is_shown_as_drifted(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    page.agent_access
        .set_servers(vec![trusted("web", ServerTrust::Drifted)]);
    open_tab(&page, AGENTS, cx);

    assert!(page.shows(item("agent-server-drifted", "web"), cx));
    assert!(page.shows(item("agent-deny", "web"), cx));
}

#[gpui_kit::test]
fn allowing_servers_adds_the_ticked_connected_ones(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    page.agent_access.set_connected_hosts(vec![
        HostChoice {
            connection: "web".into(),
            user: "root".into(),
            host: "web.example".into(),
        },
        HostChoice {
            connection: "cache".into(),
            user: "root".into(),
            host: "cache.example".into(),
        },
    ]);
    open_tab(&page, AGENTS, cx);

    page.click("agent-allow", cx);
    assert!(page.shows("agent-picker", cx));
    assert!(
        !page.shows(item("agent-pick", "web"), cx),
        "a trusting server is not offered"
    );
    page.click(item("agent-pick", "cache"), cx);
    page.click("agent-allow-confirm", cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(page.shows(item("agent-server", "cache"), cx));
    assert!(!page.shows("agent-picker", cx));
}

#[gpui_kit::test]
fn revoking_an_access_ends_it_and_the_list_is_read_again(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    open_tab(&page, AGENTS, cx);

    page.click(item("agent-grant-revoke", "g1"), cx);
    cx.run_until_parked();
    page.render(cx);

    assert_eq!(page.agent_access.revoked(), vec!["g1".to_owned()]);
    assert!(!page.shows(item("agent-grant", "g1"), cx));
}

#[gpui_kit::test]
fn reissuing_the_ca_asks_first(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    open_tab(&page, AGENTS, cx);

    page.click("agent-reissue", cx);
    page.settle_dialog(cx);
    assert!(page.has_dialog(cx));
    page.click("agent-reissue-keep", cx);
    cx.run_until_parked();
    assert_eq!(page.agent_access.reissued(), 0);

    page.click("agent-reissue", cx);
    page.settle_dialog(cx);
    page.click("agent-reissue-confirm", cx);
    cx.run_until_parked();
    page.render(cx);
    assert_eq!(page.agent_access.reissued(), 1);
}

#[gpui_kit::test]
fn a_servers_own_ca_line_is_copied_to_the_clipboard(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    open_tab(&page, AGENTS, cx);

    page.click(item("agent-copy-line", "db"), cx);
    cx.run_until_parked();

    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("cert-authority SHA256:abc db"));
}

#[gpui_kit::test]
fn a_server_with_the_shared_principal_needs_an_update_until_it_connects(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    let mut old = trusted("web", ServerTrust::Trusted);
    old.needs_ca_update = true;
    page.agent_access.set_servers(vec![old]);
    open_tab(&page, AGENTS, cx);
    assert!(page.shows(item("agent-server-update", "web"), cx));
    assert!(!page.shows(item("agent-server-trusted", "web"), cx));

    let hubs = page.installed.hubs().clone();
    cx.update(|cx| hubs.update(cx, |hubs, cx| hubs.reconcile_agent_host("web", cx)));
    cx.run_until_parked();
    open_tab(&page, OVERVIEW, cx);
    open_tab(&page, AGENTS, cx);
    assert!(page.shows(item("agent-server-trusted", "web"), cx));
}

#[gpui_kit::test]
fn records_that_cannot_be_read_are_reported_and_read_again_on_request(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    page.agent_access.fail_with(AgentAccessError::HostChanged);
    open_tab(&page, AGENTS, cx);
    assert!(page.shows("agent-access-failed", cx));

    page.agent_access.recover();
    page.click("agent-access-retry", cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(!page.shows("agent-access-failed", cx));
    assert!(page.shows(item("agent-server", "web"), cx));
}

fn locked_page(cx: &mut TestAppContext) -> Fixture {
    let page = page_with_access(cx);
    page.hub.lock_keys(&hub_id());
    open_tab(&page, AGENTS, cx);
    page
}

fn submit_password(page: &Fixture, password: &str, cx: &mut TestAppContext) {
    page.type_text(password, cx);
    page.click("account-keys-unlock", cx);
    cx.run_until_parked();
    page.render(cx);
}

#[gpui_kit::test]
fn closed_account_keys_ask_for_the_password_instead_of_showing_records(cx: &mut TestAppContext) {
    let page = locked_page(cx);

    assert!(page.shows("account-keys-locked", cx));
    assert!(page.shows("agent-access-failed", cx));
    assert!(!page.shows(item("agent-server", "web"), cx));
}

#[gpui_kit::test]
fn the_right_password_opens_the_keys_and_the_records_are_read_again(cx: &mut TestAppContext) {
    let page = locked_page(cx);

    submit_password(&page, crate::test_support::PASSWORD, cx);

    assert!(page.shows("account-keys-open", cx));
    assert!(!page.shows("account-keys-locked", cx));
    assert!(page.shows(item("agent-server", "web"), cx));
    assert_eq!(page.account_keys.attempts(), vec![true]);
}

#[gpui_kit::test]
fn a_wrong_password_is_refused_and_the_keys_stay_closed(cx: &mut TestAppContext) {
    let page = locked_page(cx);

    submit_password(&page, "not it", cx);

    assert!(page.shows("account-keys-refused", cx));
    assert!(page.shows("account-keys-locked", cx));
    assert!(!page.shows(item("agent-server", "web"), cx));
    assert_eq!(page.account_keys.attempts(), vec![false]);
}

#[gpui_kit::test]
fn locking_the_keys_hides_the_records_until_the_password_is_typed(cx: &mut TestAppContext) {
    let page = page_with_access(cx);
    open_tab(&page, AGENTS, cx);
    assert!(page.shows("account-keys-open", cx));
    assert!(page.shows(item("agent-server", "web"), cx));

    page.click("account-keys-lock", cx);
    cx.run_until_parked();
    page.render(cx);

    assert!(page.shows("account-keys-locked", cx));
    assert!(!page.shows(item("agent-server", "web"), cx));
}
