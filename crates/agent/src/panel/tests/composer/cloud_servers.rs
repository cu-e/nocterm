//! The Servers control of a Cloud chat (ADR 0147): where each bound server stands, the server
//! the next prompts run on, binding for a chosen window and unbinding.
use super::cloud_delivery::{captured_panel, press, send, shown};
use super::*;
use nocterm_cloud_conversation_model::{
    BoundServer, ConversationAction as CloudAction, ServerAccess, ServerBinding, ServerChoice,
    ServerKey,
};

pub(super) const DB: ServerKey = ServerKey::from_bytes([0xdb; 16]);
const WEB: ServerKey = ServerKey::from_bytes([0xeb; 16]);

fn item(kind: &str, key: ServerKey) -> gpui_kit::ElementId {
    gpui_kit::ElementId::Name(format!("{kind}-{key}").into())
}
pub(super) fn binding() -> ServerBinding {
    let server = |server, host: &str, access, selectable| BoundServer {
        server,
        host: host.into(),
        port: 22,
        user: "deploy".into(),
        access,
        selectable,
    };
    ServerBinding {
        servers: vec![
            server(
                DB,
                "db",
                ServerAccess::Active {
                    until_ms: 4_000_000_000_000,
                },
                true,
            ),
            server(WEB, "web", ServerAccess::Unreachable, false),
        ],
        expires_at_ms: Some(4_100_000_000_000),
    }
}
fn choice(server: ServerKey, host: &str, port: Option<u16>) -> ServerChoice {
    ServerChoice {
        server,
        connection: host.into(),
        host: host.into(),
        user: "deploy".into(),
        port,
        needs_ca_update: false,
    }
}

#[gpui_kit::test]
fn a_cloud_chat_shows_its_bound_servers_and_runs_the_next_prompt_on_the_picked_one(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    assert!(shown(&f, "cloud-servers", cx));
    assert!(
        !shown(&f, "cloud-targets", cx),
        "nothing to pick while unbound"
    );
    source.set_servers(binding());
    cx.run_until_parked();
    assert!(shown(&f, item("cloud-server-active", DB), cx));
    assert!(shown(&f, item("cloud-server-unreachable", WEB), cx));
    assert!(!shown(&f, item("cloud-target", WEB), cx));

    press(&f, item("cloud-target", DB), cx);
    send(&f, "uptime", cx);
    assert_eq!(
        source.delivered().last(),
        Some(&CloudAction::Prompt {
            text: "uptime".into(),
            model: None,
            images: Vec::new(),
            server: Some(DB),
        })
    );
    press(&f, "cloud-target-default", cx);
    send(&f, "whoami", cx);
    assert_eq!(
        source.delivered().last(),
        Some(&CloudAction::Prompt {
            text: "whoami".into(),
            model: None,
            images: Vec::new(),
            server: None,
        })
    );
}

#[gpui_kit::test]
fn binding_names_the_chosen_trusted_servers_for_the_picked_window_and_unbind_revokes(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    source.set_server_choices(vec![choice(DB, "db", Some(22)), choice(WEB, "web", None)]);
    press(&f, "cloud-servers-edit", cx);
    assert!(shown(&f, "cloud-servers-form", cx));
    // A server whose port this device does not know cannot be chosen yet.
    press(&f, item("cloud-server-option", WEB), cx);
    press(&f, item("cloud-server-option", DB), cx);
    press(&f, "cloud-servers-days-30", cx);
    press(&f, "cloud-servers-bind", cx);
    let month = std::time::Duration::from_secs(30 * 86_400);
    assert_eq!(source.bindings(), [(vec![DB], month)]);
    assert!(!shown(&f, "cloud-servers-form", cx));

    // A new form starts at seven days with the bound servers chosen.
    source.set_servers(binding());
    cx.run_until_parked();
    press(&f, "cloud-servers-edit", cx);
    assert!(shown(&f, "cloud-servers-form", cx), "the form opens again");
    press(&f, "cloud-servers-bind", cx);
    let week = std::time::Duration::from_secs(7 * 86_400);
    assert_eq!(source.bindings().last(), Some(&(vec![DB], week)));

    press(&f, "cloud-servers-unbind", cx);
    assert_eq!(source.unbinds(), 1);
}

fn setup_fixture(
    opening: bool,
    cx: &mut TestAppContext,
) -> (
    Fixture,
    nocterm_cloud_conversation_model::testing::FakeConversation,
    nocterm_hubs_model::testing::FakeAgentAccess,
    nocterm_hubs_model::HubId,
) {
    setup_fixture_width(opening, 26., cx)
}

fn setup_fixture_width(
    opening: bool,
    width: f32,
    cx: &mut TestAppContext,
) -> (
    Fixture,
    nocterm_cloud_conversation_model::testing::FakeConversation,
    nocterm_hubs_model::testing::FakeAgentAccess,
    nocterm_hubs_model::HubId,
) {
    let mut f = fixture_with_width(cx, width, workspace_resolver(), image_deps());
    let trust = nocterm_hubs_model::testing::FakeAgentAccess::default();
    let address = "home.test:7443";
    let access = crate::panel::tests::cloud_agents::access_with_trust(
        &nocterm_cloud_agents_model::testing::FakeCloudAgents::default(),
        &[address],
        Some(address),
        &trust,
        cx,
    );
    let (handle, workspace, panel, terminal) = cx.update(|cx| {
        let installed = f.installation.installed.with_cloud_agents(Some(access));
        open_host(
            &f._directory,
            &installed,
            &f.installation.layout,
            &f.access,
            cx,
        )
    });
    f.handle = handle;
    f.workspace = workspace;
    f.panel = panel;
    f.terminal = terminal;
    if opening {
        crate::panel::tests::servers::install_servers(
            &f,
            vec![crate::panel::tests::servers::summary("db", None)],
            cx,
        );
    } else {
        let saved = crate::panel::tests::servers::summary("db", None);
        cx.update(|cx| {
            f.workspace.update(cx, |workspace, _| {
                workspace.set_connection_directory(Rc::new(Directory(std::cell::RefCell::new(
                    vec![saved],
                ))));
            });
        });
    }
    let (source, _) = captured_panel(&f, cx);
    let hub =
        nocterm_hubs_model::HubId::from(nocterm_hubs_model::HubAddress::parse(address).unwrap());
    (f, source, trust, hub)
}

fn connected(trust: &nocterm_hubs_model::testing::FakeAgentAccess) {
    trust.set_connected_hosts(vec![nocterm_hubs_model::HostChoice {
        connection: "db".into(),
        user: "deploy".into(),
        host: "db".into(),
    }]);
}

fn registered(
    trust: &nocterm_hubs_model::testing::FakeAgentAccess,
    hub: &nocterm_hubs_model::HubId,
    cx: &mut TestAppContext,
) -> bool {
    use nocterm_hubs_model::AgentHostTrust as _;
    cx.foreground_executor()
        .block_test(trust.overview(hub))
        .unwrap()
        .servers
        .iter()
        .any(|server| server.connection == "db")
}

#[gpui_kit::test]
fn an_empty_cloud_binding_offers_saved_servers_and_requires_explicit_access_before_binding(
    cx: &mut TestAppContext,
) {
    let (f, source, trust, hub) = setup_fixture(true, cx);
    press(&f, "cloud-servers-edit", cx);
    assert!(shown(&f, "cloud-server-setup", cx));
    assert!(shown(&f, "cloud-server-connect-db", cx));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "cloud-server-connect-db",
            "cloud-server-allow-db",
            "cloud-servers-bind",
        ] {
            assert!(
                window.find(id).visible(),
                "{id} must be usable before scrolling"
            );
        }
    })
    .unwrap();
    press(&f, "cloud-server-allow-db", cx);
    assert!(
        !registered(&trust, &hub, cx),
        "a disconnected server cannot be authorized"
    );
    press(&f, "cloud-server-connect-db", cx);
    cx.update(|cx| assert_eq!(f.workspace.read(cx).items(cx).count(), 2));
    assert!(
        !registered(&trust, &hub, cx),
        "opening SSH grants no agent access"
    );
    connected(&trust);
    press(&f, "cloud-server-refresh", cx);
    assert!(
        !registered(&trust, &hub, cx),
        "listing a server never grants access"
    );
    press(&f, "cloud-server-allow-db", cx);
    assert!(
        registered(&trust, &hub, cx),
        "the explicit action installs the CA"
    );
    assert!(
        source.bindings().is_empty(),
        "trust does not bind any chat by itself"
    );
    source.set_server_choices(vec![choice(DB, "db", Some(22))]);
    press(&f, "cloud-server-refresh", cx);
    press(&f, item("cloud-server-option", DB), cx);
    press(&f, "cloud-servers-bind", cx);
    assert_eq!(
        source.bindings(),
        [(vec![DB], std::time::Duration::from_secs(7 * 86_400))]
    );
}

#[gpui_kit::test]
fn a_failed_server_authorization_keeps_the_form_and_can_be_retried(cx: &mut TestAppContext) {
    let (f, _source, trust, hub) = setup_fixture(true, cx);
    connected(&trust);
    trust.fail_next_allow(nocterm_hubs_model::AgentAccessError::HostChanged);
    press(&f, "cloud-servers-edit", cx);
    press(&f, "cloud-server-allow-db", cx);
    assert!(shown(&f, "cloud-server-setup-error", cx));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let error = window.find("cloud-server-setup-error");
        assert!(
            error.visible(),
            "failures stay outside the scrollable choices"
        );
        assert!(error.bounds().bottom() <= window.find("cloud-servers-options").bounds().origin.y);
        assert!(
            window.find("cloud-server-allow-db").visible(),
            "the retry remains visible"
        );
    })
    .unwrap();
    assert!(!registered(&trust, &hub, cx));
    press(&f, "cloud-server-allow-db", cx);
    assert!(registered(&trust, &hub, cx));
    assert!(!shown(&f, "cloud-server-setup-error", cx));
}

#[gpui_kit::test]
fn a_saved_server_whose_opening_is_unavailable_reports_it_in_the_cloud_form(
    cx: &mut TestAppContext,
) {
    let (f, _source, trust, hub) = setup_fixture(false, cx);
    press(&f, "cloud-servers-edit", cx);
    press(&f, "cloud-server-connect-db", cx);
    assert!(shown(&f, "cloud-server-setup-error", cx));
    assert!(!registered(&trust, &hub, cx));
}

#[gpui_kit::test]
fn a_server_control_of_an_ended_cloud_account_cannot_authorize_a_server(cx: &mut TestAppContext) {
    let (f, source, trust, hub) = setup_fixture(true, cx);
    connected(&trust);
    press(&f, "cloud-servers-edit", cx);
    source.end();
    cx.run_until_parked();
    assert!(
        !shown(&f, "cloud-server-allow-db", cx),
        "an ended source offers no authorization"
    );
    assert!(!registered(&trust, &hub, cx));
}

#[gpui_kit::test]
fn a_late_server_authorization_result_cannot_bind_an_ended_conversation(cx: &mut TestAppContext) {
    let (f, source, trust, hub) = setup_fixture(true, cx);
    connected(&trust);
    press(&f, "cloud-servers-edit", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("cloud-server-allow-db", cx);
    })
    .unwrap();
    // The action was explicitly authorized; its completion has not been handled yet.
    source.end();
    cx.run_until_parked();
    assert!(
        registered(&trust, &hub, cx),
        "already authorized work may finish for its original account"
    );
    assert!(
        source.bindings().is_empty(),
        "a late completion grants no binding"
    );
    assert!(
        !shown(&f, "cloud-server-allow-db", cx),
        "the retired source has no active controls"
    );
}

#[gpui_kit::test]
fn cloud_binding_actions_stay_visible_below_a_scrollable_list_in_a_narrow_panel(
    cx: &mut TestAppContext,
) {
    let (f, source, _trust, _hub) = setup_fixture_width(true, 18., cx);
    let mut choices = vec![choice(DB, "db", Some(22))];
    choices.extend((1..=24).map(|index| {
        choice(
            ServerKey::from_bytes([index; 16]),
            &format!("server-{index}"),
            Some(22),
        )
    }));
    source.set_server_choices(choices);
    press(&f, "cloud-servers-edit", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = window.find("agent-panel").bounds();
        let list = window.find("cloud-servers-options").bounds();
        let actions = window.find("cloud-servers-actions");
        let bind = window.find("cloud-servers-bind");
        assert!(
            actions.visible() && bind.visible(),
            "the binding action needs no scrolling"
        );
        assert!(actions.bounds().origin.y >= list.bottom());
        assert!(bind.bounds().right() <= panel.right());
        assert!(bind.bounds().right() <= window.find("cloud-servers").bounds().right());
        assert!(bind.bounds().bottom() <= window.find("agent-composer").bounds().origin.y);
        assert!(list.size.height <= window.viewport_size().height / 3.);
    })
    .unwrap();
    press(&f, item("cloud-server-option", DB), cx);
    press(&f, "cloud-servers-bind", cx);
    assert_eq!(
        source.bindings(),
        [(vec![DB], std::time::Duration::from_secs(7 * 86_400))]
    );
}
