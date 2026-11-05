//! A chat's servers (ADR 0147): the binding it shows, the server its prompts run on, and binding
//! and unbinding through its conversation.
use super::{actions, opened};
use crate::CloudChat;
use gpui::TestAppContext;
use nocterm_agent_chat::conversation::{BoundServerState, ConversationError, ServerProblem};
use nocterm_ai::conversation::ConversationAction;
use nocterm_cloud_conversation_model::{
    BoundServer, CommandKey, CommandState, ConversationAction as CloudAction, ServerAccess,
    ServerBinding, ServerChoice, ServerKey,
};
use std::time::Duration;

const DB: ServerKey = ServerKey::from_bytes([0xdb; 16]);
const WEB: ServerKey = ServerKey::from_bytes([0xeb; 16]);

fn bound(server: ServerKey, host: &str, access: ServerAccess, selectable: bool) -> BoundServer {
    BoundServer {
        server,
        host: host.into(),
        port: 22,
        user: "deploy".into(),
        access,
        selectable,
    }
}
fn binding() -> ServerBinding {
    ServerBinding {
        servers: vec![
            bound(DB, "db", ServerAccess::Active { until_ms: 9_000 }, true),
            bound(WEB, "web", ServerAccess::Unreachable, false),
        ],
        expires_at_ms: Some(50_000),
    }
}

#[gpui::test]
fn the_chat_shows_each_bound_server_and_its_prompts_run_on_the_one_picked(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.set_servers(binding());
    cx.run_until_parked();
    let servers = cx.update(|cx| CloudChat::conversation(&chat).servers(cx).unwrap());
    let shown = cx.update(|cx| servers.bound(cx));
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0].label, "deploy@db:22");
    assert_eq!(shown[0].state, BoundServerState::ActiveUntil(9_000));
    assert_eq!(shown[1].state, BoundServerState::Unreachable);
    assert_eq!(cx.update(|cx| servers.expires_at_ms(cx)), Some(50_000));

    // A server the runner cannot reach is not offered for prompts.
    let refused = cx.update(|cx| servers.set_target(Some(shown[1].id.clone()), cx));
    assert!(matches!(
        refused,
        Err(ConversationError::Servers(ServerProblem::NotSelectable))
    ));
    cx.update(|cx| servers.set_target(Some(shown[0].id.clone()), cx))
        .unwrap();
    assert_eq!(
        cx.update(|cx| servers.target(cx)),
        Some(shown[0].id.clone())
    );
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("df -h".into()), cx));
    cx.run_until_parked();
    drop(sent);
    assert_eq!(
        fake.delivered().last(),
        Some(&CloudAction::Prompt {
            text: "df -h".into(),
            server: Some(DB),
            model: None,
            images: Vec::new(),
        })
    );

    // Once the server cannot run prompts, they go to the default destination again.
    let mut lapsed = binding();
    lapsed.servers[0].selectable = false;
    fake.set_servers(lapsed);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| servers.target(cx)), None);
}

#[gpui::test]
async fn a_server_change_invalidates_the_old_draft_and_retry_keeps_its_original_target(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.set_servers(binding());
    cx.run_until_parked();
    let source = CloudChat::conversation(&chat);
    let (servers, composer) =
        cx.update(|cx| (source.servers(cx).unwrap(), source.composer(cx).unwrap()));
    cx.update(|cx| servers.set_target(Some(DB.to_string()), cx))
        .unwrap();
    let original = cx.update(|cx| composer.revision(cx)).unwrap();
    let controls = actions(&chat, cx);
    let first = cx.update(|cx| controls.perform(ConversationAction::Prompt("same".into()), cx));
    cx.run_until_parked();
    cx.update(|cx| servers.set_target(None, cx)).unwrap();
    let changed = cx.update(|cx| composer.revision(cx)).unwrap();
    assert!(changed > original);
    assert!(matches!(
        cx.update(|cx| composer.clear_images(original, cx)),
        Err(ConversationError::Stale)
    ));
    cx.update(|cx| servers.set_target(None, cx)).unwrap();
    assert_eq!(cx.update(|cx| composer.revision(cx)).unwrap(), changed);
    let second = cx.update(|cx| controls.perform(ConversationAction::Prompt("same".into()), cx));
    cx.run_until_parked();
    let delivered = fake.delivered();
    assert!(
        matches!(&delivered[0], CloudAction::Prompt { server: Some(server), .. } if *server == DB)
    );
    assert!(matches!(
        &delivered[1],
        CloudAction::Prompt { server: None, .. }
    ));
    fake.settle(CommandKey::from_bytes([1; 16]), CommandState::Refused);
    cx.run_until_parked();
    assert!(matches!(first.await, Err(ConversationError::Rejected)));
    let refused = super::view(&chat, cx, |view| view.pending_prompts[0].key);
    let retry = cx.update(|cx| controls.retry_prompt(refused, cx));
    cx.run_until_parked();
    assert_eq!(fake.delivered()[0], fake.delivered()[2]);
    fake.end();
    cx.run_until_parked();
    assert!(matches!(
        cx.update(|cx| servers.set_target(Some(DB.to_string()), cx)),
        Err(ConversationError::Stale)
    ));
    drop((second, retry));
}

#[gpui::test]
async fn binding_names_the_chosen_servers_and_unbinding_forgets_the_picked_one(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.set_server_choices(vec![ServerChoice {
        server: DB,
        connection: "db".into(),
        host: "db".into(),
        user: "deploy".into(),
        port: Some(22),
        needs_ca_update: true,
    }]);
    let servers = cx.update(|cx| CloudChat::conversation(&chat).servers(cx).unwrap());
    let options = cx.update(|cx| servers.options(cx)).await.unwrap();
    assert_eq!(options.len(), 1);
    assert!(options[0].bindable && options[0].needs_ca_update);
    let week = Duration::from_secs(7 * 86_400);
    cx.update(|cx| servers.bind(vec![options[0].id.clone()], week, cx))
        .await
        .unwrap();
    assert_eq!(fake.bindings(), [(vec![DB], week)]);
    let too_long = cx.update(|cx| servers.bind(vec![options[0].id.clone()], 5 * week, cx));
    assert!(matches!(
        too_long.await,
        Err(ConversationError::Servers(ServerProblem::Window))
    ));

    fake.set_servers(binding());
    cx.run_until_parked();
    let db = cx.update(|cx| servers.bound(cx))[0].id.clone();
    cx.update(|cx| servers.set_target(Some(db), cx)).unwrap();
    cx.update(|cx| servers.unbind(cx)).await.unwrap();
    assert_eq!(fake.unbinds(), 1);
    assert_eq!(cx.update(|cx| servers.target(cx)), None);
}
