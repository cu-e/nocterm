//! Workspace port results reach startup without native filesystem work in Runtime.
use super::*;
use nocterm_ai::{
    testing::{ScriptedUsageSource, ScriptedWorkdirResolver},
    workspace::{PreparedWorkspace, WorkdirError},
};
fn candidate() -> std::path::PathBuf {
    if cfg!(windows) {
        r"C:\candidate".into()
    } else {
        "/candidate".into()
    }
}
fn canonical() -> std::path::PathBuf {
    if cfg!(windows) {
        r"C:\canonical\work".into()
    } else {
        "/canonical/work".into()
    }
}
fn runtime_with_workspace(
    cx: &mut TestAppContext,
    connector: Arc<ScriptedConnector>,
    resolver: Arc<ScriptedWorkdirResolver>,
) -> Entity<Runtime> {
    let repository = Arc::new(repository());
    let storage = AgentStorage::prepared(repository.clone(), repository, Ok(Default::default()));
    runtime_with_usage_source(
        cx,
        connector,
        storage,
        &candidate(),
        Arc::new(ScriptedUsageSource::default()),
        resolver,
    )
}
fn connect(runtime: &Entity<Runtime>, cx: &mut TestAppContext) {
    let client = ProbeClient::new(cx, snapshot());
    let launch = nocterm_ai::AgentLaunch {
        id: "test".into(),
        name: "Test".into(),
        command: "not-executed".into(),
        args: Vec::new(),
        env: Default::default(),
        inherit_env: Vec::new(),
    };
    runtime.update(cx, |runtime, cx| {
        runtime.connect(client.clone(), "test-chat".into(), launch, cx)
    });
}
#[gpui::test]
fn prepared_canonical_root_is_forwarded_to_connector_and_resolver_refusal_stops_startup(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let resolver = Arc::new(ScriptedWorkdirResolver::default());
    resolver.supply(
        candidate().join("work"),
        Ok(PreparedWorkspace {
            root: canonical(),
            sandbox: None,
        }),
    );
    connector.push_failed();
    let runtime = runtime_with_workspace(cx, connector.clone(), resolver.clone());
    connect(&runtime, cx);
    cx.run_until_parked();
    assert_eq!(connector.working_directories().len(), 1);
    assert_eq!(connector.working_directories()[0], canonical());
    assert_eq!(resolver.requests()[0].candidate, candidate().join("work"));
    let refusing_connector = Arc::new(ScriptedConnector::default());
    let refusing = Arc::new(ScriptedWorkdirResolver::default());
    refusing.supply(
        candidate().join("work"),
        Err(WorkdirError::Protected(
            nocterm_ai::sandbox::ProtectedWorkdirError,
        )),
    );
    let runtime = runtime_with_workspace(cx, refusing_connector.clone(), refusing);
    connect(&runtime, cx);
    cx.run_until_parked();
    assert!(refusing_connector.working_directories().is_empty());
}
#[gpui::test]
fn cancellation_after_pending_workspace_preparation_never_calls_connector(cx: &mut TestAppContext) {
    let connector = Arc::new(ScriptedConnector::default());
    let resolver = Arc::new(ScriptedWorkdirResolver::default());
    let answer = resolver.pending(candidate().join("work"));
    let runtime = runtime_with_workspace(cx, connector.clone(), resolver.clone());
    connect(&runtime, cx);
    cx.run_until_parked();
    assert_eq!(resolver.requests().len(), 1);
    let _drain = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    answer
        .send(Ok(PreparedWorkspace {
            root: canonical(),
            sandbox: None,
        }))
        .unwrap();
    cx.run_until_parked();
    assert!(connector.working_directories().is_empty());
}
