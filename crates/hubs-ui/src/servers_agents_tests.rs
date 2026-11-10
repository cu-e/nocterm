//! The Agents and Agent access tabs of a server read and write the account's agents through the
//! agents service.
use super::{ServersPage, item};
use crate::test_support::{Fixture, fixture_over, hub_id, pinned_record, server};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_hub_agents_model::{HubServers as _, NewServer};
use nocterm_hubs_model::Environment;
use nocterm_hubs_model::testing::FakeHub;
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;

const AGENTS: usize = 3;
const ACCESS: usize = 4;

fn page(cx: &mut TestAppContext) -> Fixture {
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
    let agents = setup.installed.hub_agents().cloned();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| ServersPage::new(hubs, agents, cx))
        })
        .unwrap()
    });
    let page = Fixture { window, ..setup };
    page.render(cx);
    page
}

fn open_tab(page: &Fixture, index: usize, cx: &mut TestAppContext) {
    page.render(cx);
    cx.update_window(page.window, |_, window, cx| {
        window.within("servers-tabs").click(index, cx);
    })
    .unwrap();
    cx.run_until_parked();
    page.render(cx);
    page.render(cx);
}

#[gpui_kit::test]
fn the_agents_tab_lists_the_accounts_agents_and_providers(cx: &mut TestAppContext) {
    let page = page(cx);
    let _ = page
        .agents
        .clone()
        .with_provider(&hub_id(), "Models", &["m1"])
        .with_agent(&hub_id(), "Ops", "m1");
    open_tab(&page, AGENTS, cx);
    assert!(page.shows(item("server-agents-tab", &hub_id().to_string()), cx));
    assert!(page.shows(item("server-agent", "agent-2"), cx));
    assert!(page.shows(item("server-provider", "provider-1"), cx));
}

#[gpui_kit::test]
fn deleting_an_agent_removes_it_on_the_hub_and_from_the_list(cx: &mut TestAppContext) {
    let page = page(cx);
    let _ = page
        .agents
        .clone()
        .with_provider(&hub_id(), "Models", &["m1"])
        .with_agent(&hub_id(), "Ops", "m1");
    open_tab(&page, AGENTS, cx);
    page.click(item("server-agent-delete", "agent-2"), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(page.agents.calls().contains(&"delete_agent".to_owned()));
    assert!(!page.shows(item("server-agent", "agent-2"), cx));
}

#[gpui_kit::test]
fn the_new_agent_form_opens_and_closes(cx: &mut TestAppContext) {
    let page = page(cx);
    open_tab(&page, AGENTS, cx);
    page.click(item("server-agent-new", &hub_id().to_string()), cx);
    assert!(page.shows("server-agent-form", cx));
    page.click("server-agent-cancel", cx);
    assert!(!page.shows("server-agent-form", cx));
}

#[gpui_kit::test]
fn the_agent_access_tab_shows_the_key_and_disconnects_a_server(cx: &mut TestAppContext) {
    let page = page(cx);
    let added = futures::executor::block_on(page.agents.add_server(
        &hub_id(),
        NewServer {
            name: "prod".into(),
            host: "prod.example".into(),
            port: 22,
            user: "root".into(),
            host_key: "ssh-ed25519 AAAAhost".into(),
        },
    ))
    .unwrap();
    open_tab(&page, ACCESS, cx);
    assert!(page.shows(item("agent-key", &hub_id().to_string()), cx));
    assert!(page.shows(item("agent-server", added.id.as_str()), cx));
    page.click(item("agent-server-disconnect", added.id.as_str()), cx);
    cx.run_until_parked();
    page.render(cx);
    assert!(!page.shows(item("agent-server", added.id.as_str()), cx));
}

#[gpui_kit::test]
fn replacing_the_agent_key_asks_first(cx: &mut TestAppContext) {
    let page = page(cx);
    open_tab(&page, ACCESS, cx);
    page.click(item("agent-key-rotate", &hub_id().to_string()), cx);
    page.settle_dialog(cx);
    assert!(page.has_dialog(cx));
    assert!(!page.agents.calls().contains(&"rotate_agent_key".to_owned()));
    page.click("agent-key-rotate-confirm", cx);
    cx.run_until_parked();
    assert!(page.agents.calls().contains(&"rotate_agent_key".to_owned()));
}
