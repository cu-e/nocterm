use crate::NativeProjectPreparation;
use futures::executor::block_on;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectId, RootPath};
use nocterm_project_providers::*;
use nocterm_session::{
    local_fs::CancelFlag,
    testing::{ExecResponse, FakeExec},
};
use std::sync::Arc;

pub(crate) fn runtime() -> NativeProjectPreparation {
    let environment =
        Arc::new(nocterm_project_providers::testing::FakeWorkspaceEnvironment::default());
    environment.respond_created(Ok(())).unwrap();
    NativeProjectPreparation::new(environment)
}
pub(crate) fn id() -> String {
    "a".repeat(64)
}
pub(crate) fn request(mode: SetupMode) -> ProjectSetupRequest {
    ProjectSetupRequest {
        project: ProjectId::from_bytes([1; 16]),
        host: HostLocator::Local,
        mode,
        repository: None,
    }
}
fn inspect(status: &str, owned: bool) -> String {
    serde_json::json!([{"Id":id(), "State":{"Status":status},"Config":{"WorkingDir":"/app", "Labels": if owned { serde_json::json!({"io.nocterm.project":ProjectId::from_bytes([1;16]).to_string()}) } else { serde_json::json!({}) }}}]).to_string()
}
pub(crate) fn answer(exec: &FakeExec, prefix: &str, stdout: String) {
    if let Some(program) = prefix
        .split_whitespace()
        .next()
        .filter(|name| matches!(*name, "docker" | "podman"))
        && (!prefix.contains(' ') || prefix.ends_with(" pull"))
    {
        exec.respond(
            format!(r#"sh -c 'exec "$@" 2>&1' nocterm-pull {program} pull"#),
            Ok(ExecResponse::text(stdout.clone(), 0, "")),
        );
    }
    exec.respond(prefix, Ok(ExecResponse::text(stdout, 0, "")));
}
pub(crate) fn prepare(
    exec: FakeExec,
    request: ProjectSetupRequest,
) -> Result<PreparedProject, ProjectSetupError> {
    block_on(runtime().prepare(
        Arc::new(exec),
        request,
        CancelFlag::default(),
        Arc::new(|_| {}),
    ))
}
#[test]
fn discovery_keeps_engines_and_image_failures_independent() {
    let exec = FakeExec::default();
    answer(
        &exec,
        "docker ps",
        r#"[{"ID":"one","Names":"same","Image":"ubuntu","State":"exited"}]"#.into(),
    );
    answer(
        &exec,
        "podman ps",
        r#"[{"ID":"one","Names":"same","Image":"debian","State":"running"}]"#.into(),
    );
    answer(&exec, "podman images", "[]".into());
    let results = block_on(runtime().discover(Arc::new(exec))).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].images.is_err());
    assert_eq!(
        results[0].containers.as_ref().unwrap()[0].state,
        nocterm_containers::State::Exited
    );
    assert_eq!(results[1].containers.as_ref().unwrap()[0].name, "same");
}
#[test]
fn attached_stopped_container_uses_exact_engine_and_inspected_directory() {
    for engine in Engine::ALL {
        let exec = FakeExec::default();
        answer(
            &exec,
            &format!("{} inspect", engine.name().to_lowercase()),
            inspect("exited", false),
        );
        answer(&exec, &engine.name().to_lowercase(), String::new());
        let result = prepare(
            exec.clone(),
            request(SetupMode::Attach {
                engine,
                container: id(),
                root: None,
            }),
        )
        .unwrap();
        let location = ContainerLocation::from_target(&result.target).unwrap();
        assert_eq!(location.engine(), engine);
        assert_eq!(location.root().as_str(), "/app");
        assert!(result.owned.is_none());
        assert!(
            exec.requests()
                .iter()
                .all(|r| r.program == engine.name().to_lowercase())
        );
        assert!(exec.ran().iter().any(|line| line.contains("start")));
    }
}
#[test]
fn unsupported_attach_state_never_starts_or_deletes_container() {
    let exec = FakeExec::default();
    answer(&exec, "docker inspect", inspect("paused", false));
    assert!(matches!(
        prepare(
            exec.clone(),
            request(SetupMode::Attach {
                engine: Engine::Docker,
                container: id(),
                root: None
            })
        ),
        Err(ProjectSetupError::UnsupportedState)
    ));
    assert_eq!(exec.requests().len(), 1);
}
#[test]
fn clone_failure_on_attached_container_never_deletes_external_resources() {
    let exec = FakeExec::default();
    answer(&exec, "docker inspect", inspect("running", false));
    exec.respond("docker exec", Ok(ExecResponse::text("", 1, "token secret")));
    let mut input = request(SetupMode::Attach {
        engine: Engine::Docker,
        container: id(),
        root: Some(RootPath::parse("/repo space $(literal)").unwrap()),
    });
    input.repository =
        Some(GitRepository::parse("https://example.test/team/repo.git".into()).unwrap());
    let error = prepare(exec.clone(), input).unwrap_err();
    assert!(matches!(error, ProjectSetupError::CommandFailed { .. }));
    assert!(!error.to_string().contains("secret"));
    assert!(
        exec.requests()
            .iter()
            .all(|r| !r.args.iter().any(|a| a == "rm"))
    );
    assert_eq!(
        exec.requests()[1].args.last().unwrap(),
        "/repo space $(literal)"
    );
}
#[test]
fn cancellation_before_mutation_runs_no_command() {
    let exec = FakeExec::default();
    let cancel = CancelFlag::default();
    cancel.cancel();
    let result = block_on(runtime().prepare(
        Arc::new(exec.clone()),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            languages: vec![],
            root: None,
        }),
        cancel,
        Arc::new(|_| {}),
    ));
    assert!(matches!(result, Err(ProjectSetupError::Canceled)));
    assert!(exec.requests().is_empty());
}
#[test]
fn cleanup_refuses_an_unowned_container() {
    let exec = FakeExec::default();
    answer(&exec, "docker inspect", inspect("running", false));
    let location = ContainerLocation::new(
        HostLocator::Local,
        Engine::Docker,
        id(),
        RootPath::parse("/").unwrap(),
        ContainerOrigin::Created,
    )
    .unwrap();
    assert!(matches!(
        block_on(runtime().discard_owned(
            Arc::new(exec.clone()),
            OwnedContainer::new(
                location,
                ProjectId::from_bytes([1; 16]),
                CreationToken::from_bytes([2; 16])
            )
        )),
        Err(ProjectSetupError::CleanupRefused)
    ));
    assert_eq!(exec.requests().len(), 1);
}

mod creation;

#[test]
fn git_preparation_reports_nonempty_and_missing_tool_as_actionable_typed_refusals() {
    for (status, expected) in [(73, "nonempty"), (74, "directory"), (76, "git")] {
        let exec = FakeExec::default();
        exec.respond(
            "sh",
            Ok(ExecResponse::text("", status, "private diagnostic")),
        );
        let mut input = request(SetupMode::Direct {
            root: RootPath::parse("/existing").unwrap(),
        });
        input.repository =
            Some(GitRepository::parse("https://example.test/repo.git".into()).unwrap());
        let error = prepare(exec, input).unwrap_err();
        assert!(matches!(
            (expected, error),
            ("nonempty", ProjectSetupError::CloneDestinationNotEmpty)
                | ("directory", ProjectSetupError::DirectoryUnavailable)
                | ("git", ProjectSetupError::GitUnavailable)
        ));
    }
}

#[test]
fn cleanup_failure_reports_exact_recoverable_container_id_without_removing_unowned_resources() {
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker inspect", inspect("running", false));
    exec.respond("docker exec", Ok(ExecResponse::text("", 1, "")));
    answer(&exec, "docker", String::new());
    let error = prepare(
        exec.clone(),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            languages: vec![],
            root: None,
        }),
    )
    .unwrap_err();
    assert!(
        matches!(error, ProjectSetupError::CleanupFailed { container, .. } if container == id())
    );
    assert!(!exec.ran().iter().any(|line| line.contains(" rm ")));
}

#[test]
fn cancellation_stops_inflight_container_command_and_keeps_attached_resource() {
    use futures::FutureExt as _;
    let exec = FakeExec::default();
    answer(&exec, "docker inspect", inspect("running", false));
    let sink = exec.queue_stream("docker exec");
    let cancel = CancelFlag::default();
    let mut operation = runtime().prepare(
        Arc::new(exec.clone()),
        request(SetupMode::Attach {
            engine: Engine::Docker,
            container: id(),
            root: None,
        }),
        cancel.clone(),
        Arc::new(|_| {}),
    );
    assert!(operation.as_mut().now_or_never().is_none());
    cancel.cancel();
    assert!(matches!(
        block_on(operation),
        Err(ProjectSetupError::Canceled)
    ));
    assert!(sink.is_closed());
    assert!(!exec.ran().iter().any(|line| line.contains(" rm ")));
}

#[test]
fn duplicate_languages_are_refused_before_any_execution() {
    let exec = FakeExec::default();
    let selection = LanguageSelection::default_for(ProgrammingLanguage::Rust);
    assert!(matches!(
        prepare(
            exec.clone(),
            request(SetupMode::Create {
                engine: Engine::Docker,
                image: WorkspaceImage::Ubuntu,
                languages: vec![selection.clone(), selection],
                root: None
            })
        ),
        Err(ProjectSetupError::InvalidInput(_))
    ));
    assert!(exec.requests().is_empty());
}

mod removal;

mod commands;
