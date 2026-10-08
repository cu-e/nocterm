#![cfg(unix)]
use super::*;
use nocterm_ai::{AgentLaunch, CloseSessionOutcome};

async fn connection(mode: &str, directory: &std::path::Path) -> AgentConnection {
    let script = r#"
import json, sys
mode = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':1,'agentCapabilities': {} if mode == 'unsupported' else {'sessionCapabilities':{'close':{}}}}
    elif method == 'session/new': result = {'sessionId':'s1'}
    elif method == 'session/close':
        if mode == 'hang': continue
        if mode == 'error':
            print(json.dumps({'jsonrpc':'2.0','id':request['id'],'error':{'code':-32603,'message':'close rejected'}}),flush=True)
            continue
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'final chunk'}}}}),flush=True)
        result = {}
    else: raise Exception(method)
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#;
    AcpConnector::unmanaged()
        .connect(ConnectRequest {
            launch: AgentLaunch {
                id: "close-test".into(),
                name: "Close".into(),
                command: "/usr/bin/python3".into(),
                args: vec!["-c".into(), script.into(), mode.into()],
                env: Default::default(),
                inherit_env: Vec::new(),
            },
            working_directory: directory.into(),
            terminal_auth: false,
            sandbox: None,
            resources: Default::default(),
            cancellation: Default::default(),
        })
        .await
        .unwrap()
}
#[cfg(unix)]
#[test]
fn acknowledged_close_waits_for_the_foreground_event_barrier() {
    futures::executor::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let connection = connection("ok", directory.path()).await;
        let session = connection
            .commands
            .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
            .await
            .unwrap();
        let close = connection.commands.close_session(session.session_id);
        let close = match select(
            close,
            Box::pin(async_io::Timer::after(Duration::from_millis(50))),
        )
        .await
        {
            Either::Left(_) => panic!("Close completed before earlier events were acknowledged"),
            Either::Right((_, close)) => close,
        };
        assert!(matches!(
            connection.events.recv().await.unwrap(),
            AgentEvent::Session(_)
        ));
        match connection.events.recv().await.unwrap() {
            AgentEvent::Barrier(ack) => {
                ack.send(()).unwrap();
            }
            _ => panic!("Expected FIFO event barrier after final session update"),
        }
        assert_eq!(close.await.unwrap(), CloseSessionOutcome::Closed);
        connection.commands.shutdown_gracefully().await.unwrap();
    });
}
#[cfg(unix)]
#[test]
fn unsupported_failed_and_hung_close_have_distinct_bounded_results() {
    futures::executor::block_on(async {
        for mode in ["unsupported", "error", "hang"] {
            let directory = tempfile::tempdir().unwrap();
            let connection = connection(mode, directory.path()).await;
            let session = connection
                .commands
                .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                .await
                .unwrap();
            let started = std::time::Instant::now();
            let outcome = connection.commands.close_session(session.session_id).await;
            match mode {
                "unsupported" => assert_eq!(outcome.unwrap(), CloseSessionOutcome::Unsupported),
                "error" => assert!(matches!(outcome, Err(AgentError::Rpc(_)))),
                "hang" => assert_eq!(outcome.unwrap(), CloseSessionOutcome::TimedOut),
                _ => unreachable!(),
            }
            assert!(started.elapsed() < Duration::from_secs(7));
            connection.commands.shutdown_gracefully().await.unwrap();
        }
    });
}
