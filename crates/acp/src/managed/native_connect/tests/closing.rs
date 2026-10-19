//! Native close follows advertised support, waits for the foreground barrier
//! and bounds failed or hung requests.
use super::*;
use nocterm_ai::{CloseSessionOutcome, history::source::ManagedCommandError};
use std::time::{Duration, Instant};
const SCRIPT: &str = r#"
import json, sys
mode = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':1,'agentCapabilities': {} if mode == 'unsupported' else {'sessionCapabilities':{'close':{}}}}
    elif method == 'session/new': result = {'sessionId':'s1'}
    elif method == 'session/set_mode': result = {}
    elif method == 'session/close':
        if mode == 'unsupported': raise Exception('unadvertised close was sent')
        if mode == 'hang': continue
        if mode == 'error':
            print(json.dumps({'jsonrpc':'2.0','id':request['id'],'error':{'code':-32603,'message':'close rejected'}}),flush=True)
            continue
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'final chunk'}}}}),flush=True)
        result = {}
    else: raise Exception(method)
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#;
#[test]
fn acknowledged_close_waits_for_the_foreground_event_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = Connected::new(directory.path(), &python(SCRIPT, ["ok"]));
    let session = agent.open(directory.path()).opened().session.clone();
    let close = agent.commands.close_session(&session).unwrap();
    let close = match block_on(futures::future::select(
        close,
        Box::pin(async_io::Timer::after(Duration::from_millis(50))),
    )) {
        futures::future::Either::Left(_) => {
            panic!("close completed before earlier events were acknowledged")
        }
        futures::future::Either::Right((_, close)) => close,
    };
    assert!(matches!(
        block_on(agent.next_event()),
        Some(ManagedAgentEvent::Session(_))
    ));
    match block_on(agent.next_event()) {
        Some(ManagedAgentEvent::Barrier(barrier)) => barrier.acknowledge(),
        _ => panic!("expected the FIFO event barrier after the final session update"),
    }
    assert_eq!(block_on(close).unwrap(), CloseSessionOutcome::Closed);
    agent.finish();
}
#[test]
fn unsupported_failed_and_hung_close_have_distinct_bounded_results() {
    for mode in ["unsupported", "error", "hang"] {
        let directory = tempfile::tempdir().unwrap();
        let agent = Connected::new(directory.path(), &python(SCRIPT, [mode]));
        let session = agent.open(directory.path()).opened().session.clone();
        let started = Instant::now();
        let outcome = block_on(agent.commands.close_session(&session).unwrap());
        match mode {
            "unsupported" => assert_eq!(outcome.unwrap(), CloseSessionOutcome::Unsupported),
            "error" => assert!(matches!(outcome, Err(ManagedCommandError::Remote(_)))),
            "hang" => assert_eq!(outcome.unwrap(), CloseSessionOutcome::TimedOut),
            _ => unreachable!(),
        }
        assert!(started.elapsed() < Duration::from_secs(7));
        // The close task leaves the shared connection usable.
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
        agent.finish();
    }
}
