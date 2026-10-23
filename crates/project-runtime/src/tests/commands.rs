//! Environment command scopes retain original cancellation and progress authority.
use super::*;
use futures::FutureExt as _;
use nocterm_session::ExecRequest;

#[test]
fn scoped_and_observed_commands_keep_original_cancellation_and_progress() {
    let original = FakeExec::default();
    let scoped = FakeExec::default();
    answer(&original, "echo", "original".into());
    answer(&scoped, "echo", "scoped".into());
    let phases = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = phases.clone();
    let cancel = CancelFlag::default();
    let commands = crate::NativeEnvironmentCommands.create(
        Arc::new(original.clone()),
        cancel.clone(),
        Arc::new(move |update| recorded.lock().unwrap().push(update.phase)),
    );
    let replacement = commands.with_executor(Arc::new(scoped.clone()));
    let observed = replacement.observation(None);
    assert_eq!(
        block_on(commands.run(SetupPhase::Inspecting, ExecRequest::new("echo"))).unwrap(),
        "original"
    );
    assert_eq!(
        block_on(observed.run(SetupPhase::InstallingTools, ExecRequest::new("echo"))).unwrap(),
        "scoped"
    );
    cancel.cancel();
    for commands in [commands, replacement, observed] {
        assert!(matches!(commands.check(), Err(ProjectSetupError::Canceled)));
        assert!(matches!(
            block_on(commands.run(SetupPhase::Inspecting, ExecRequest::new("echo"))),
            Err(ProjectSetupError::Canceled)
        ));
    }
    assert_eq!(
        *phases.lock().unwrap(),
        [SetupPhase::Inspecting, SetupPhase::InstallingTools]
    );
    assert_eq!(original.requests().len(), 1);
    assert_eq!(scoped.requests().len(), 1);
}

#[test]
fn created_installer_receives_the_stored_owned_receipt_and_original_command_context() {
    let environment =
        Arc::new(nocterm_project_providers::testing::FakeWorkspaceEnvironment::default());
    environment.respond_created(Ok(())).unwrap();
    let exec = FakeExec::default();
    answer(&exec, "podman create", id());
    answer(&exec, "podman", String::new());
    let cancel = CancelFlag::default();
    let runtime = NativeProjectPreparation::new(environment.clone());
    let result = block_on(runtime.prepare(
        Arc::new(exec.clone()),
        request(SetupMode::Create {
            engine: Engine::Podman,
            image: WorkspaceImage::Ubuntu,
            languages: vec![],
            root: None,
        }),
        cancel.clone(),
        Arc::new(|_| {}),
    ))
    .unwrap();
    let calls = environment.created_calls().unwrap();
    assert_eq!(calls.len(), 1);
    let receipt = result.owned.unwrap();
    assert_eq!(calls[0].owned().location(), receipt.location());
    assert_eq!(calls[0].owned().attempt(), receipt.attempt());
    assert!(calls[0].selections().is_empty());
    assert!(calls[0].commands().check().is_ok());
    cancel.cancel();
    assert!(matches!(
        calls[0].commands().check(),
        Err(ProjectSetupError::Canceled)
    ));
    assert!(!exec.ran().iter().any(|line| line.contains(" rm ")));
}

#[test]
fn canceled_observation_interrupts_the_original_inflight_stream() {
    let exec = FakeExec::default();
    let sink = exec.queue_stream("probe");
    let cancel = CancelFlag::default();
    let commands =
        crate::NativeEnvironmentCommands.create(Arc::new(exec), cancel.clone(), Arc::new(|_| {}));
    let mut pending = commands
        .observation(None)
        .run(SetupPhase::Inspecting, ExecRequest::new("probe"));
    assert!(pending.as_mut().now_or_never().is_none());
    cancel.cancel();
    assert!(matches!(
        block_on(pending),
        Err(ProjectSetupError::Canceled)
    ));
    assert!(sink.is_closed());
}
