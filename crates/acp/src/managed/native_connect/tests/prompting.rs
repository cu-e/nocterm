//! Native prompts keep the original dispatch, cancellation and permission routes.
use super::*;
use nocterm_ai::{
    history::{
        dispatch::{ActivePromptScope, DispatchFraming},
        live::LiveOwner,
        source::{ManagedCommandError, ManagedDispatchFuture, SourceContext},
    },
    model::{
        auth::PermissionOutcome,
        native::NativeCopies,
        session::{EndOfTurn, PromptOutcome},
    },
};
type Prompt = ManagedDispatchFuture<Result<NativeOwned<PromptOutcome>, ManagedCommandError>>;
/// One live document dispatching prompts to a subprocess agent.
pub(super) struct Prompting {
    live: LiveOwner,
    context: SourceContext,
    commands: ManagedCommandsHandle,
    events: ManagedEventReceiver,
    _producer: SourceProducer,
}
impl Prompting {
    pub(super) fn new(root: &Path, launch: &AgentLaunch) -> Self {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let connector = AcpConnector::try_unmanaged().unwrap();
        let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
        let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
        let lease = producer.capture().unwrap();
        let context = lease.context().unwrap();
        let mut live = LiveOwner::new(
            roles.take_live().unwrap(),
            roles.take_foreground().unwrap(),
            roles.take_prompt_dispatch().unwrap(),
        )
        .unwrap();
        live.initialize("agent", "8384f339-e1b0-4a76-af5a-d5f66379162f", 0)
            .unwrap();
        let resources = nocterm_ai::AgentResourceSettings::default();
        let facts = context
            .prepare_startup(ManagedStartupRequest {
                launch,
                working_directory: root,
                create_private: false,
                isolation: None,
                terminal_auth: false,
                resources: &resources,
            })
            .unwrap();
        let workspace = nocterm_ai::model::native::workspace::prepared_workspace(
            context.copy_path(root).unwrap(),
            NativeOwned::absent(context.ledger()).unwrap(),
        )
        .unwrap();
        let request = ManagedStartup::new(facts, lease)
            .unwrap()
            .prepared(workspace)
            .unwrap();
        let (_, commands, events) = block_on(connector.connect_managed(request).unwrap())
            .unwrap()
            .into_parts();
        Self {
            live,
            context,
            commands,
            events,
            _producer: producer,
        }
    }
    pub(super) fn prompt(&mut self, text: &str) -> (Prompt, ActivePromptScope) {
        let admission = self.live.local_admission();
        self.live.queue_text(admission, 1, text, &[], None).unwrap();
        let framing = DispatchFraming::new(
            nocterm_ai::model::native::controls::session_id(
                NativeCopies::new(self.context.ledger())
                    .text("native-session")
                    .unwrap(),
            )
            .unwrap(),
            NativeOwned::absent(self.context.ledger()).unwrap(),
            NativeOwned::absent(self.context.ledger()).unwrap(),
            false,
        );
        let admission = self.live.local_admission();
        let pending = self.live.stage_dispatch(admission, framing, 1).unwrap();
        let native = self.commands.prepare_prompt(pending.payload()).unwrap();
        let original = pending.commit().unwrap();
        let scope = original.scope();
        (native.bind(original), scope)
    }
    async fn next_event(&mut self) -> Option<ManagedAgentEvent> {
        futures::future::poll_fn(|cx| self.events.poll_event(cx)).await
    }
    pub(super) fn finish(self) {
        block_on(self.commands.shutdown_finally().unwrap()).unwrap();
    }
}
#[test]
fn public_native_prompt_keeps_original_dispatch_until_reply_and_active_scope_drop() {
    let script = r#"
while IFS= read -r line; do
    id=${line#*'"id":'}; id=${id%%,*}
    case "$line" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[]}}\n' "$id";;
        *'"method":"session/prompt"'*)
            case "$line" in *'"text":"hello native"'*) :;; *) exit 42;; esac
            pending=$id
            printf '%s\n' '{"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"sessionId":"native-session","toolCall":{"toolCallId":"tool"},"options":[{"optionId":"allow","name":"Allow","kind":{"allow_once":null}}]}}';;
        *'"id":"permission"'*)
            case "$line" in *'"outcome":"selected","optionId":"allow"'*) :;; *) exit 43;; esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$pending";;
        *) exit 44;;
    esac
done
"#;
    let directory = tempfile::tempdir().unwrap();
    let mut agent = Prompting::new(directory.path(), &shell(script));
    let (prompt, scope) = agent.prompt("hello native");
    assert!(agent.live.dispatch().capture().is_err());
    let permission = async {
        match agent.next_event().await {
            Some(ManagedAgentEvent::Permission { request, respond }) => {
                assert_eq!(request.permission().choices.len(), 1);
                respond.respond(PermissionOutcome::Selected(
                    request.permission().choices[0].id.clone(),
                ));
            }
            _ => panic!("native prompt did not publish its original permission request"),
        }
    };
    let (outcome, ()) = block_on(futures::future::join(prompt, permission));
    assert_eq!(outcome.unwrap().get().ending, EndOfTurn::Finished);
    assert!(agent.live.dispatch().capture().is_err());
    drop(scope);
    assert!(agent.live.dispatch().capture().is_ok());
    block_on(agent.commands.shutdown_finally().unwrap()).unwrap();
}
#[test]
fn cancel_remains_dispatchable_while_a_prompt_is_generating() {
    let script = r#"
while IFS= read -r line; do
    id=${line#*'"id":'}; id=${id%%,*}
    case "$line" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[]}}\n' "$id";;
        *'"method":"session/prompt"'*)
            pending=$id
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"started"}}}}';;
        *'"method":"session/cancel"'*)
            case "$line" in *'"sessionId":"native-session"'*) :;; *) exit 42;; esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"cancelled"}}\n' "$pending";;
        *) exit 43;;
    esac
done
"#;
    let directory = tempfile::tempdir().unwrap();
    let mut agent = Prompting::new(directory.path(), &shell(script));
    let (prompt, _scope) = agent.prompt("generate");
    let commands = agent.commands.clone();
    let cancel = async {
        assert!(matches!(
            agent.next_event().await,
            Some(ManagedAgentEvent::Session(_))
        ));
        let session = nocterm_ai::model::ids::SessionId::new("native-session");
        commands.cancel(&session).unwrap();
    };
    let (outcome, ()) = block_on(futures::future::join(prompt, cancel));
    assert_eq!(outcome.unwrap().get().ending, EndOfTurn::Cancelled);
    block_on(commands.shutdown_finally().unwrap()).unwrap();
}
#[test]
fn prompt_streams_before_a_cancelled_permission_and_controls_reach_the_agent() {
    let script = r#"
while IFS= read -r line; do
    id=${line#*'"id":'}; id=${id%%,*}
    case "$line" in
        *'"method":"initialize"'*)
            case "$line" in *'"terminal":true'*|*'"readTextFile":true'*|*'"writeTextFile":true'*) exit 41;; esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"oauth","name":"OAuth"}]}}\n' "$id";;
        *'"method":"session/set_mode"'*)
            case "$line" in *'"modeId":"mode"'*) :;; *) exit 42;; esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id";;
        *'"method":"authenticate"'*)
            case "$line" in *'"methodId":"oauth"'*) :;; *) exit 43;; esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id";;
        *'"method":"session/prompt"'*)
            pending=$id
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"streamed"}}}}'
            printf '%s\n' '{"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"sessionId":"native-session","toolCall":{"toolCallId":"tool"},"options":[{"optionId":"yes","name":"Allow","kind":"allow_once"}]}}';;
        *'"id":"permission"'*)
            case "$line" in *'"outcome":"cancelled"'*) :;; *) exit 44;; esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$pending";;
        *) exit 45;;
    esac
done
"#;
    let directory = tempfile::tempdir().unwrap();
    let mut agent = Prompting::new(directory.path(), &shell(script));
    let session = nocterm_ai::model::ids::SessionId::new("native-session");
    block_on(
        agent
            .commands
            .set_mode(&nocterm_ai::model::session::ModeSelection {
                session: session.clone(),
                mode: nocterm_ai::model::ids::ModeId::new("mode"),
            })
            .unwrap(),
    )
    .unwrap();
    block_on(
        agent
            .commands
            .authenticate(&nocterm_ai::model::ids::AuthMethodId::new("oauth"))
            .unwrap(),
    )
    .unwrap();
    let (prompt, _scope) = agent.prompt("test");
    let foreground = async {
        let mut streamed = false;
        loop {
            match agent.next_event().await {
                Some(ManagedAgentEvent::Session(_)) => streamed = true,
                Some(ManagedAgentEvent::Permission { respond, .. }) => {
                    assert!(streamed);
                    respond.respond(PermissionOutcome::Cancelled);
                    break;
                }
                _ => panic!("unexpected event before the permission request"),
            }
        }
    };
    let (outcome, ()) = block_on(futures::future::join(prompt, foreground));
    assert_eq!(outcome.unwrap().get().ending, EndOfTurn::Finished);
    block_on(agent.commands.shutdown_finally().unwrap()).unwrap();
}
