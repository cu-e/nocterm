//! Opt-in tests use the adapter's environment policy and private empty workdir.
use futures::{
    FutureExt,
    future::{Either, select},
};
use nocterm_acp::AcpConnector;
use nocterm_ai::{AgentConnector, AgentError, AgentEvent, AgentRegistry, ConnectRequest, acp};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[test]
#[ignore = "requires an installed and authenticated real ACP agent"]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn real_agent_initialize_and_session() {
    futures::executor::block_on(async {
        let id =
            std::env::var("NOCTERM_ACP_SMOKE").expect("Set NOCTERM_ACP_SMOKE=claude|codex|hermes");
        let settings = nocterm_ai::AiSettings::default();
        let registry = AgentRegistry::new(&settings);
        let launch = registry.get(&id).expect("Known agent").clone();
        let directory = tempfile::tempdir().unwrap();
        let connection = AcpConnector::unmanaged()
            .connect(ConnectRequest {
                terminal_auth: false,
                sandbox: None,
                resources: Default::default(),
                cancellation: Default::default(),
                launch,
                working_directory: directory.path().to_owned(),
            })
            .await
            .expect("Initialize real agent");
        let session = connection
            .commands
            .new_session(acp::NewSessionRequest::new(directory.path().to_owned()))
            .await
            .expect("Create session");
        assert!(!session.session_id.to_string().is_empty());
        if std::env::var_os("NOCTERM_ACP_SMOKE_PROMPT").is_some() {
            let commands = connection.commands.clone();
            let events = connection.events.clone();
            let received = Arc::new(Mutex::new(String::new()));
            let streamed = received.clone();
            let drain_events = events.clone();
            let prompt = commands.prompt(acp::PromptRequest::new(
                session.session_id,
                vec![acp::ContentBlock::Text(acp::TextContent::new(
                    "Reply exactly NOCTERM_OK without tools.",
                ))],
            ));
            let drain = async move {
                loop {
                    match drain_events.recv().await {
                        Ok(AgentEvent::Permission { respond, .. }) => {
                            respond.respond(acp::RequestPermissionOutcome::Cancelled);
                        }
                        Ok(AgentEvent::Exited { .. }) | Err(_) => break,
                        Ok(event) => collect_message(event, &streamed),
                    }
                }
            };
            let run = async {
                match select(prompt, drain.boxed()).await {
                    Either::Left((result, _)) => result.map(|_| ()),
                    Either::Right(_) => Err(AgentError::Io("Agent exited before response".into())),
                }
            };
            match select(
                run.boxed(),
                Box::pin(async_io::Timer::after(Duration::from_secs(90))),
            )
            .await
            {
                Either::Left((result, _)) => match result {
                    Ok(()) => {}
                    Err(AgentError::AuthRequired(_)) => panic!("Prompt requires authentication"),
                    Err(AgentError::Rpc(error)) => {
                        panic!(
                            "Prompt RPC failed with code {:?}: {}",
                            error.code, error.message
                        )
                    }
                    Err(_) => panic!("Prompt transport failed"),
                },
                Either::Right(_) => {
                    commands.shutdown();
                    panic!("Prompt timed out");
                }
            }
            while let Ok(event) = events.try_recv() {
                collect_message(event, &received);
            }
            assert!(
                received.lock().unwrap().contains("NOCTERM_OK"),
                "Agent did not stream the requested response marker"
            );
        }
        connection.commands.shutdown();
    });
}
fn collect_message(event: AgentEvent, received: &Mutex<String>) {
    if let AgentEvent::Session(notification) = event
        && let acp::SessionUpdate::AgentMessageChunk(chunk) = notification.update
        && let acp::ContentBlock::Text(text) = chunk.content
    {
        let mut buffer = received.lock().unwrap();
        for ch in text.text.chars() {
            if buffer.len() + ch.len_utf8() > 4096 {
                break;
            }
            buffer.push(ch);
        }
    }
}
