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
    NativeProjectPreparation::new(Arc::new(crate::NativeWorkspaceEnvironment::new(Arc::new(
        crate::NativeEnvironmentCommands,
    ))))
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
    {
        exec.respond(
            format!(
                "{program} exec --interactive {} {}",
                id(),
                crate::activation::activate_request(
                    nocterm_session::ExecRequest::new("sh")
                        .args(["-c", crate::environment::probe::PROBE])
                )
                .command_line()
            ),
            Ok(ExecResponse::text(probe_fixture(), 0, "")),
        );
        if !prefix.contains(' ') || prefix.ends_with(" pull") {
            exec.respond(
                format!(r#"sh -c 'exec "$@" 2>&1' nocterm-pull {program} pull"#),
                Ok(ExecResponse::text(stdout.clone(), 0, "")),
            );
        }
    }
    exec.respond(prefix, Ok(ExecResponse::text(stdout, 0, "")));
}
pub(crate) fn probe_fixture() -> String {
    "kernel\x00Linux\x00architecture\x00x86_64\x00os_release\x00ID=ubuntu\nPRETTY_NAME=Ubuntu\x00libc\x00glibc 2.39\x00uid\x000\x00user\x00root\x00home\x00/root\x00writable\x00yes\x00program\x00apt-get\x00program\x00curl\x00program\x00bash\x00program\x00tar\x00program\x00gzip\x00program\x00xz\x00program\x00unzip\x00program\x00sha256sum\x00program\x00awk\x00program\x00mktemp\x00program\x00cc\x00python\x00Python 3.13.2\x00rust\x00rustc 1.90.0\x00bun\x001.3.3\x00node\x00v22.1.0\x00java\x00openjdk version 21.0.1\x00".into()
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
fn creating_installs_all_selected_languages_and_keeps_path_for_future_shells() {
    let exec = FakeExec::default();
    answer(&exec, "podman create", id());
    answer(&exec, "podman", String::new());
    let result = prepare(
        exec.clone(),
        request(SetupMode::Create {
            engine: Engine::Podman,
            image: WorkspaceImage::Ubuntu,
            languages: ProgrammingLanguage::ALL
                .into_iter()
                .map(LanguageSelection::default_for)
                .collect(),
            root: None,
        }),
    )
    .unwrap();
    assert!(result.owned.is_some());
    assert_eq!(
        ContainerLocation::from_target(&result.target)
            .unwrap()
            .root()
            .as_str(),
        "/workspace"
    );
    let requests = exec.requests();
    let create = requests
        .iter()
        .find(|r| r.args.first().is_some_and(|a| a == "create"))
        .unwrap();
    assert!(
        !create
            .args
            .iter()
            .any(|arg| arg.starts_with("CARGO_HOME=") || arg.starts_with("JAVA_HOME="))
    );
    let script = installation_scripts(&requests);
    assert!(script.contains("nocterm/toolchains"));
    assert!(!script.contains("PATH=/usr/sbin:"));
    for request in requests.iter().filter(|request| {
        request.stdin.is_some() && request.args[request.args.len() - 3] == "distribution"
    }) {
        let script = std::str::from_utf8(request.stdin.as_deref().unwrap()).unwrap();
        assert!(script.contains("PATH=/usr/bin:/bin command -v"));
    }
    let versions: Vec<_> = requests
        .iter()
        .filter(|request| request.stdin.is_some())
        .map(|request| request.args[request.args.len() - 3].as_str())
        .collect();
    assert_eq!(
        versions,
        [
            "distribution",
            "stable",
            "latest",
            "distribution",
            "distribution"
        ]
    );
}
#[test]
fn custom_images_keep_existing_tools_unless_languages_are_explicitly_selected() {
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker", String::new());
    prepare(
        exec.clone(),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Custom("alpine:3".into()),
            languages: vec![],
            root: None,
        }),
    )
    .unwrap();
    assert!(exec.requests().iter().all(|r| r.stdin.is_none()));
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker", String::new());
    prepare(
        exec.clone(),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Custom("ubuntu:24.04".into()),
            languages: vec![LanguageSelection::parse(ProgrammingLanguage::Python, "3.13").unwrap()],
            root: None,
        }),
    )
    .unwrap();
    assert!(
        exec.requests()
            .iter()
            .any(|request| request.stdin.is_some())
    );
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
fn selected_versions_travel_as_arguments_for_both_supported_images() {
    let selected: Vec<_> = [
        (ProgrammingLanguage::Python, "3.13"),
        (ProgrammingLanguage::Rust, "1.90.0"),
        (ProgrammingLanguage::Bun, "1.3.3"),
        (ProgrammingLanguage::Node, "22"),
        (ProgrammingLanguage::Java, "21"),
    ]
    .into_iter()
    .map(|(language, version)| LanguageSelection::parse(language, version).unwrap())
    .collect();
    for image in [WorkspaceImage::Ubuntu, WorkspaceImage::Debian] {
        let exec = FakeExec::default();
        answer(&exec, "docker create", id());
        answer(&exec, "docker", String::new());
        prepare(
            exec.clone(),
            request(SetupMode::Create {
                engine: Engine::Docker,
                image,
                languages: selected.clone(),
                root: None,
            }),
        )
        .unwrap();
        let requests = exec.requests();
        let installations: Vec<_> = requests
            .iter()
            .filter(|request| request.stdin.is_some())
            .collect();
        assert_eq!(installations.len(), selected.len());
        for (request, selection) in installations.iter().zip(&selected) {
            assert_eq!(
                request.args[request.args.len() - 3],
                selection.version().as_str()
            );
            assert_eq!(request.args[request.args.len() - 4], "/root");
            let script = std::str::from_utf8(request.stdin.as_deref().unwrap()).unwrap();
            assert!(!script.contains("1.90.0"));
            assert!(!script.contains("1.3.3"));
        }
        let script = installation_scripts(&requests);
        assert!(script.contains("sha256sum -c"));
        assert!(!script.contains("/usr/local/bin"));
        assert!(!script.contains("/root/.cargo"));
    }
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

#[test]
fn installer_downloads_follow_https_redirects_without_accepting_http_errors() {
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker", String::new());
    prepare(
        exec.clone(),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            languages: [
                (ProgrammingLanguage::Python, "3.13"),
                (ProgrammingLanguage::Rust, "1.90.0"),
                (ProgrammingLanguage::Bun, "1.3.3"),
                (ProgrammingLanguage::Node, "22"),
                (ProgrammingLanguage::Java, "21"),
            ]
            .into_iter()
            .map(|(language, version)| LanguageSelection::parse(language, version).unwrap())
            .collect(),
            root: None,
        }),
    )
    .unwrap();
    let requests = exec.requests();
    let script = installation_scripts(&requests);
    assert!(script.contains("--proto '=https' --proto-redir '=https'"));
    assert!(script.contains("-sSL"));
    assert!(script.contains("case \"$status\" in 200)"));
    assert!(script.contains("https://astral.sh/uv/install.sh"));
    assert!(script.contains("https://sh.rustup.rs"));
    assert!(script.contains("https://github.com/oven-sh/bun/releases/"));
}
fn installation_scripts(requests: &[nocterm_session::ExecRequest]) -> String {
    requests
        .iter()
        .filter_map(|request| request.stdin.as_deref())
        .map(|bytes| String::from_utf8_lossy(bytes))
        .collect::<Vec<_>>()
        .join("\n")
}

mod removal;

mod commands;
