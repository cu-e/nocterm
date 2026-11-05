//! The Servers control of a Cloud chat (ADR 0147): where each bound server stands, the server
//! the next prompts run on, binding for a chosen window and unbinding.
use super::cloud_delivery::{captured_panel, press, send, shown};
use super::*;
use nocterm_cloud_conversation_model::{
    BoundServer, ConversationAction as CloudAction, ServerAccess, ServerBinding, ServerChoice,
    ServerKey,
};

const DB: ServerKey = ServerKey::from_bytes([0xdb; 16]);
const WEB: ServerKey = ServerKey::from_bytes([0xeb; 16]);

fn item(kind: &str, key: ServerKey) -> gpui_kit::ElementId {
    gpui_kit::ElementId::Name(format!("{kind}-{key}").into())
}
fn binding() -> ServerBinding {
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
            server: Some(DB),
        })
    );
    press(&f, "cloud-target-default", cx);
    send(&f, "whoami", cx);
    assert_eq!(
        source.delivered().last(),
        Some(&CloudAction::Prompt {
            text: "whoami".into(),
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
