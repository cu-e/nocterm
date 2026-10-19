//! Public native startup exercises the actual managed SDK and subprocess owner.
use crate::AcpConnector;
use futures::executor::block_on;
use nocterm_ai::{
    connection::{
        ManagedAgentEvent, ManagedCommandsHandle, ManagedConnectRequest, ManagedEventReceiver,
        ManagedStartup, ManagedStartupRequest,
    },
    history::source::{ManagedOpened, SourceProducer},
    model::{
        native::NativeOwned,
        retention::{RetentionAuthority, document::DocumentRetention},
        session::SessionOpen,
    },
    registry::AgentLaunch,
};
use std::path::Path;
mod closing;
mod configuration;
mod launching;
mod prompting;
mod restoration;
mod startup;
fn request(
    authority: &RetentionAuthority,
    root: &Path,
    script: &str,
) -> (SourceProducer, ManagedConnectRequest) {
    launch_request(authority, root, &shell(script))
}
fn shell(script: &str) -> AgentLaunch {
    launch("/bin/sh", ["-c", script])
}
fn python<'a>(script: &'a str, args: impl IntoIterator<Item = &'a str>) -> AgentLaunch {
    launch("/usr/bin/python3", ["-c", script].into_iter().chain(args))
}
fn launch<'a>(command: &str, args: impl IntoIterator<Item = &'a str>) -> AgentLaunch {
    AgentLaunch {
        id: "test".into(),
        name: "Fallback".into(),
        command: command.into(),
        args: args.into_iter().map(Into::into).collect(),
        env: Default::default(),
        inherit_env: vec![],
    }
}
fn launch_request(
    authority: &RetentionAuthority,
    root: &Path,
    launch: &AgentLaunch,
) -> (SourceProducer, ManagedConnectRequest) {
    prepared_request(authority, root, launch, false)
}
/// An isolated request carries the original sandbox policy for `root`.
fn prepared_request(
    authority: &RetentionAuthority,
    root: &Path,
    launch: &AgentLaunch,
    isolated: bool,
) -> (SourceProducer, ManagedConnectRequest) {
    let (_, mut roles) = DocumentRetention::new(authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
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
    let sandbox = if isolated {
        NativeOwned::supplied(
            nocterm_ai::model::native::workspace::sandbox_policy(
                context.ledger(),
                root,
                None,
                &[],
                &[],
            )
            .unwrap(),
        )
    } else {
        NativeOwned::absent(context.ledger()).unwrap()
    };
    let workspace = nocterm_ai::model::native::workspace::prepared_workspace(
        context.copy_path(root).unwrap(),
        sandbox,
    )
    .unwrap();
    let startup = ManagedStartup::new(facts, lease).unwrap();
    (producer, startup.prepared(workspace).unwrap())
}
/// A connected subprocess agent; the producer keeps its source open.
struct Connected {
    _producer: SourceProducer,
    commands: ManagedCommandsHandle,
    events: ManagedEventReceiver,
}
impl Connected {
    fn new(root: &Path, launch: &AgentLaunch) -> Self {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let connector = AcpConnector::try_unmanaged().unwrap();
        let (producer, request) = launch_request(&authority, root, launch);
        let (_, commands, events) = block_on(connector.connect_managed(request).unwrap())
            .unwrap()
            .into_parts();
        Self {
            _producer: producer,
            commands,
            events,
        }
    }
    fn open(&self, root: &Path) -> ManagedOpened {
        block_on(
            self.commands
                .new_session(&SessionOpen {
                    working_directory: root.to_owned(),
                    endpoints: vec![],
                })
                .unwrap(),
        )
        .unwrap()
    }
    async fn next_event(&mut self) -> Option<ManagedAgentEvent> {
        futures::future::poll_fn(|cx| self.events.poll_event(cx)).await
    }
    fn finish(self) {
        block_on(self.commands.shutdown_finally().unwrap()).unwrap();
    }
}
const SCRIPT: &str = r#"
while IFS= read -r line; do
    id=${line#*'"id":'}
    id=${id%%,*}
    case "$line" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{},"fork":{},"close":{}}},"authMethods":[],"agentInfo":{"name":"Managed native","version":"1"}}}\n' "$id";;
        *'"method":"session/new"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"native-session"}}\n' "$id";;
        *'"method":"session/close"'*)
            printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"before close"}}}}\n'
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id";;
        *'"method":"session/cancel"'*) :;;
        *) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id";;
    esac
done
"#;
#[test]
fn public_managed_native_connection_uses_original_driver_and_foreground_close_barrier() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let connector = AcpConnector::try_unmanaged().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (producer, request) = request(&authority, directory.path(), SCRIPT);
    let connected = block_on(connector.connect_managed(request).unwrap()).unwrap();
    let (info, commands, mut events) = connected.into_parts();
    assert_eq!(info.info().name, "Managed native");
    assert!(info.info().capabilities.restore);
    let opened = block_on(
        commands
            .new_session(&SessionOpen {
                working_directory: directory.path().to_owned(),
                endpoints: vec![],
            })
            .unwrap(),
    )
    .unwrap();
    let session = opened.opened().session.clone();
    let close = commands.close_session(&session).unwrap();
    let drain = async {
        let mut seen = false;
        loop {
            match futures::future::poll_fn(|cx| events.poll_event(cx)).await {
                Some(ManagedAgentEvent::Session(notice)) => {
                    assert_eq!(notice.notice().session.as_str(), session.as_str());
                    seen = true;
                }
                Some(ManagedAgentEvent::Barrier(barrier)) => {
                    assert!(seen);
                    barrier.acknowledge();
                    break;
                }
                _ => panic!("native connection stopped before close fence"),
            }
        }
    };
    let (close, ()) = block_on(futures::future::join(close, drain));
    assert_eq!(close.unwrap(), nocterm_ai::CloseSessionOutcome::Closed);
    block_on(commands.shutdown_gracefully().unwrap()).unwrap();
    loop {
        match block_on(futures::future::poll_fn(|cx| events.poll_event(cx))) {
            Some(ManagedAgentEvent::Exited { .. }) | None => break,
            Some(ManagedAgentEvent::Failed(error)) => panic!("{error}"),
            _ => {}
        }
    }
    assert!(producer.capture().is_err());
    drop((commands, events, info, opened));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if producer.capture().is_ok() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn cancelled_managed_startup_never_launches_the_configured_native_program() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let connector = AcpConnector::try_unmanaged().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (producer, request) = request(&authority, directory.path(), "touch must-not-exist");
    request.cancellation().cancel();
    let error = block_on(connector.connect_managed(request).unwrap())
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "Agent startup cancelled");
    assert!(!directory.path().join("must-not-exist").exists());
    drop(error);
    assert!(producer.capture().is_ok());
}
#[test]
fn malformed_native_notices_and_permission_keep_the_original_connection_alive() {
    let script = r#"
while IFS= read -r line; do
    id=${line#*'"id":'}; id=${id%%,*}
    case "$line" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[]}}\n' "$id";;
        *'"method":"session/new"'*)
            pending=$id
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":false}'
            printf '%s\n' '{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":[]}}'
            printf '%s\n' '{"jsonrpc":"2.0","id":"bad-permission","method":"session/request_permission","params":{"sessionId":"native-session","toolCall":{"toolCallId":"t"},"options":false}}';;
        *'"id":"bad-permission"'*)
            case "$line" in
                *'"code":-32602'*'"error":"invalid type: boolean `false`, expected a sequence"'*'"phase":"deserialization"'*) :;;
                *) exit 42;;
            esac
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"still connected"}}}}'
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"native-session"}}\n' "$pending";;
        *'"id":null,"error":{"code":-32600,"message":"Invalid request"}'*) :;;
        *) exit 43;;
    esac
done
"#;
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let connector = AcpConnector::try_unmanaged().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (_, request) = request(&authority, directory.path(), script);
    let connected = block_on(connector.connect_managed(request).unwrap()).unwrap();
    let (_, commands, mut events) = connected.into_parts();
    let opened = block_on(
        commands
            .new_session(&SessionOpen {
                working_directory: directory.path().to_owned(),
                endpoints: vec![],
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(opened.opened().session.as_str(), "native-session");
    match block_on(futures::future::poll_fn(|cx| events.poll_event(cx))) {
        Some(ManagedAgentEvent::Session(notice)) => {
            assert_eq!(notice.notice().session.as_str(), "native-session")
        }
        _ => panic!("malformed input displaced the valid later notice"),
    }
    block_on(commands.shutdown_finally().unwrap()).unwrap();
}
