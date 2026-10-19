//! Opt-in tests use the adapter's environment policy and private empty workdir.
use futures::{
    FutureExt,
    future::{Either, select},
};
use nocterm_acp::AcpConnector;
use nocterm_ai::{
    AgentRegistry,
    connection::{
        ManagedAgentEvent, ManagedCommandsHandle, ManagedEventReceiver, ManagedStartup,
        ManagedStartupRequest,
    },
    history::{
        dispatch::DispatchFraming,
        live::LiveOwner,
        source::{ManagedCommandError, SourceContext, SourceProducer},
    },
    model::{
        auth::PermissionOutcome,
        content::Content,
        ids::SessionId,
        native::{NativeCopies, NativeOwned},
        notice::SessionUpdate,
        retention::{
            RetentionAuthority,
            document::{DocumentRetention, DocumentRoles},
        },
        session::SessionOpen,
    },
};
use std::time::Duration;

#[test]
#[ignore = "requires an installed and authenticated real ACP agent"]
fn real_agent_initialize_and_session() {
    let id = std::env::var("NOCTERM_ACP_SMOKE").expect("Set NOCTERM_ACP_SMOKE=claude|codex|hermes");
    let settings = nocterm_ai::AiSettings::default();
    let registry = AgentRegistry::new(&settings);
    let launch = registry.get(&id).expect("Known agent").clone();
    let directory = tempfile::tempdir().unwrap();
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let connector = AcpConnector::try_unmanaged().unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let resources = nocterm_ai::AgentResourceSettings::default();
    let facts = context
        .prepare_startup(ManagedStartupRequest {
            launch: &launch,
            working_directory: directory.path(),
            create_private: false,
            isolation: None,
            terminal_auth: false,
            resources: &resources,
        })
        .unwrap();
    let workspace = nocterm_ai::model::native::workspace::prepared_workspace(
        context.copy_path(directory.path()).unwrap(),
        NativeOwned::absent(context.ledger()).unwrap(),
    )
    .unwrap();
    let request = ManagedStartup::new(facts, lease)
        .unwrap()
        .prepared(workspace)
        .unwrap();
    let (_, commands, mut events) =
        futures::executor::block_on(connector.connect_managed(request).unwrap())
            .expect("Initialize real agent")
            .into_parts();
    let opened = futures::executor::block_on(
        commands
            .new_session(&SessionOpen {
                working_directory: directory.path().to_owned(),
                endpoints: Vec::new(),
            })
            .unwrap(),
    )
    .expect("Create session");
    let session = opened.opened().session.clone();
    assert!(!session.as_str().is_empty());
    if std::env::var_os("NOCTERM_ACP_SMOKE_PROMPT").is_some() {
        smoke_prompt(&id, &mut roles, &context, &commands, &mut events, &session);
    }
    futures::executor::block_on(commands.shutdown_finally().unwrap()).unwrap();
}
/// Streams one exact-marker prompt through the real staged dispatch path.
fn smoke_prompt(
    id: &str,
    roles: &mut DocumentRoles,
    context: &SourceContext,
    commands: &ManagedCommandsHandle,
    events: &mut ManagedEventReceiver,
    session: &SessionId,
) {
    let mut live = LiveOwner::new(
        roles.take_live().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    live.initialize(id, "8384f339-e1b0-4a76-af5a-d5f66379162f", 0)
        .unwrap();
    let admission = live.local_admission();
    live.queue_text(
        admission,
        1,
        "Reply exactly NOCTERM_OK without tools.",
        &[],
        None,
    )
    .unwrap();
    let framing = DispatchFraming::new(
        nocterm_ai::model::native::controls::session_id(
            NativeCopies::new(context.ledger())
                .text(session.as_str())
                .unwrap(),
        )
        .unwrap(),
        NativeOwned::absent(context.ledger()).unwrap(),
        NativeOwned::absent(context.ledger()).unwrap(),
        false,
    );
    let admission = live.local_admission();
    let pending = live.stage_dispatch(admission, framing, 1).unwrap();
    let native = commands.prepare_prompt(pending.payload()).unwrap();
    let prompt = native.bind(pending.commit().unwrap());
    let mut received = String::new();
    let drain = drain(events, &mut received);
    let run = async {
        match select(prompt, drain.boxed()).await {
            Either::Left((result, _)) => result.map(|_| ()),
            Either::Right(_) => panic!("Agent exited before response"),
        }
    };
    let result = match futures::executor::block_on(select(
        run.boxed(),
        Box::pin(async_io::Timer::after(Duration::from_secs(90))),
    )) {
        Either::Left((result, _)) => result,
        Either::Right(_) => {
            commands.shutdown();
            panic!("Prompt timed out");
        }
    };
    match result {
        Ok(()) => {}
        Err(error) if error.authentication_required() => {
            panic!("Prompt requires authentication")
        }
        Err(ManagedCommandError::Remote(fault)) => panic!(
            "Prompt RPC failed with code {:?}: {}",
            fault.get().code(),
            fault.get().message()
        ),
        Err(_) => panic!("Prompt transport failed"),
    }
    assert!(
        received.contains("NOCTERM_OK"),
        "Agent did not stream the requested response marker"
    );
}
async fn drain(events: &mut ManagedEventReceiver, received: &mut String) {
    loop {
        match futures::future::poll_fn(|cx| events.poll_event(cx)).await {
            Some(ManagedAgentEvent::Permission { respond, .. }) => {
                respond.respond(PermissionOutcome::Cancelled);
            }
            Some(ManagedAgentEvent::Session(notice)) => {
                if let SessionUpdate::AgentChunk(Content::Text(text)) = &notice.notice().update {
                    for ch in text.text.chars() {
                        if received.len() + ch.len_utf8() > 4096 {
                            break;
                        }
                        received.push(ch);
                    }
                }
            }
            Some(ManagedAgentEvent::Barrier(barrier)) => barrier.acknowledge(),
            Some(ManagedAgentEvent::Exited { .. } | ManagedAgentEvent::Failed(_)) | None => break,
        }
    }
}
