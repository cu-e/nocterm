//! Managed preparation keeps the same supplied pending reply and original Source loan.
use super::*;
use crate::{
    AgentLaunch, AgentResourceSettings,
    connection::{ManagedStartup, ManagedStartupRequest},
    history::source::{SourceError, SourceProducer},
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
use futures::{FutureExt, task::noop_waker};
use std::task::{Context, Poll};

fn root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\managed\workspace")
    } else {
        PathBuf::from("/managed/workspace")
    }
}
fn startup() -> (SourceProducer, ManagedStartup) {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let launch = AgentLaunch {
        id: "agent".into(),
        name: "Agent".into(),
        command: "agent".into(),
        args: Vec::new(),
        inherit_env: Vec::new(),
        env: Default::default(),
    };
    let root = root();
    let resources = AgentResourceSettings::default();
    let facts = context
        .prepare_startup(ManagedStartupRequest {
            launch: &launch,
            working_directory: &root,
            create_private: false,
            isolation: None,
            terminal_auth: false,
            resources: &resources,
        })
        .unwrap();
    (producer, ManagedStartup::new(facts, lease).unwrap())
}

#[test]
fn managed_workspace_awaits_original_pending_reply_and_frees_work_before_source_return() {
    let resolver = ScriptedWorkdirResolver::default();
    let reply = resolver.pending(root());
    let (producer, startup) = startup();
    let mut future = resolver
        .prepare_managed(startup.handle(), startup.lifetime())
        .unwrap();
    assert!(
        future
            .poll_unpin(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
    assert_eq!(resolver.requests()[0].candidate, root());
    drop(startup);
    assert!(matches!(producer.capture(), Err(SourceError::Unavailable)));
    reply
        .send(Ok(PreparedWorkspace {
            root: root(),
            sandbox: None,
        }))
        .unwrap();
    let Poll::Ready(result) = future.poll_unpin(&mut Context::from_waker(&noop_waker())) else {
        panic!("original reply released")
    };
    let workspace = result.unwrap();
    assert_eq!(workspace.get().root, root());
    assert!(matches!(producer.capture(), Err(SourceError::Unavailable)));
    drop(future);
    let next = producer.capture().unwrap();
    assert_eq!(workspace.get().root, root());
    drop(next);
}

#[test]
fn cancelled_managed_workspace_drops_same_pending_receiver_without_blocking_executor() {
    let resolver = ScriptedWorkdirResolver::default();
    let reply = resolver.pending(root());
    let (producer, startup) = startup();
    let mut future = resolver
        .prepare_managed(startup.handle(), startup.lifetime())
        .unwrap();
    assert!(
        future
            .poll_unpin(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
    drop(startup);
    drop(future);
    assert!(reply.is_canceled());
    drop(producer.capture().unwrap());
}
