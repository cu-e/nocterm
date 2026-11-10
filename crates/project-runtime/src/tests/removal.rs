use super::*;
use crate::NativeWorkspaceContainerActions;
fn source(exec: &FakeExec, engine: Engine) -> ContainerActionSource {
    ContainerActionSource::new(
        ContainerLocation::new(
            HostLocator::Local,
            engine,
            id(),
            RootPath::parse("/home").unwrap(),
            ContainerOrigin::Attached,
        )
        .unwrap(),
        &HostLocator::Local,
        Arc::new(exec.clone()),
    )
    .unwrap()
}
fn run(
    exec: &FakeExec,
    engine: Engine,
    action: ContainerAction,
) -> Result<ContainerActionOutcome, ContainerActionError> {
    block_on(NativeWorkspaceContainerActions.apply(
        source(exec, engine),
        action,
        CancelFlag::default(),
    ))
}
fn response(exec: &FakeExec, engine: Engine, command: &str, output: &str, status: u32) {
    exec.respond(
        format!("{} {command}", engine.name().to_lowercase()),
        Ok(ExecResponse::text(output, status, "engine diagnostic")),
    );
}
#[test]
fn explicit_delete_stops_then_removes_only_the_exact_saved_container_for_both_engines() {
    for engine in Engine::ALL {
        let exec = FakeExec::default();
        response(&exec, engine, "inspect", &inspect("running", false), 0);
        response(&exec, engine, "stop", "", 0);
        response(&exec, engine, "rm", "", 0);
        assert_eq!(
            run(&exec, engine, ContainerAction::Delete).unwrap().action,
            ContainerAction::Delete
        );
        let requests = exec.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].args, ["inspect", "--type", "container", &id()]);
        assert_eq!(requests[1].args, ["stop", "--time", "10", &id()]);
        assert_eq!(requests[2].args, ["rm", &id()]);
        assert!(
            requests
                .iter()
                .all(|request| request.program == engine.name().to_lowercase())
        );
    }
}
#[test]
fn already_stopped_container_stop_is_idempotent_and_delete_never_requests_volumes_or_force() {
    for action in [ContainerAction::Stop, ContainerAction::Delete] {
        let exec = FakeExec::default();
        response(
            &exec,
            Engine::Docker,
            "inspect",
            &inspect("exited", false),
            0,
        );
        response(&exec, Engine::Docker, "rm", "", 0);
        run(&exec, Engine::Docker, action).unwrap();
        assert_eq!(
            exec.requests().len(),
            if action == ContainerAction::Stop {
                1
            } else {
                2
            }
        );
        assert!(exec.requests().iter().all(|request| {
            !request
                .args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--force" | "--volumes" | "stop" | "prune"))
        }));
    }
}
#[test]
fn missing_mismatched_malformed_and_denied_inspection_never_mutate_resources() {
    for (output, status) in [
        ("[]".to_owned(), 0),
        (inspect("running", false).replace(&id(), &"b".repeat(64)), 0),
        ("not json".into(), 0),
        (String::new(), 1),
    ] {
        let exec = FakeExec::default();
        response(&exec, Engine::Docker, "inspect", &output, status);
        let error = run(&exec, Engine::Docker, ContainerAction::Delete).unwrap_err();
        assert!(matches!(
            error,
            ContainerActionError::IdentityMismatch | ContainerActionError::Runtime(_)
        ));
        assert_eq!(exec.requests().len(), 1);
    }
}
#[test]
fn failed_stop_prevents_removal_and_failed_removal_reports_the_acknowledged_stop() {
    for stop_ok in [false, true] {
        let exec = FakeExec::default();
        response(
            &exec,
            Engine::Docker,
            "inspect",
            &inspect("running", false),
            0,
        );
        response(&exec, Engine::Docker, "stop", "", u32::from(!stop_ok));
        response(&exec, Engine::Docker, "rm", "", 1);
        let error = run(&exec, Engine::Docker, ContainerAction::Delete).unwrap_err();
        assert_eq!(
            matches!(error, ContainerActionError::StoppedBeforeFailure { .. }),
            stop_ok
        );
        assert_eq!(exec.requests().len(), if stop_ok { 3 } else { 2 });
    }
}
#[test]
fn canceled_action_performs_no_inspection_or_mutation() {
    let exec = FakeExec::default();
    let cancel = CancelFlag::default();
    cancel.cancel();
    assert!(matches!(
        block_on(NativeWorkspaceContainerActions.apply(
            source(&exec, Engine::Docker),
            ContainerAction::Delete,
            cancel
        )),
        Err(ContainerActionError::Canceled)
    ));
    assert!(exec.requests().is_empty());
}
#[test]
fn lifecycle_failures_bound_diagnostics_and_keep_the_typed_command_failure() {
    let exec = FakeExec::default();
    exec.respond(
        "docker inspect",
        Ok(ExecResponse::text("", 125, "x".repeat(8000))),
    );
    assert!(
        matches!(run(&exec, Engine::Docker, ContainerAction::Stop), Err(ContainerActionError::Runtime(ProjectSetupError::CommandFailed { status: Some(125), stderr_tail, .. })) if stderr_tail.len() == 4096)
    );
}
