use super::*;
use futures::FutureExt as _;
use nocterm_session::{ExecExit, ExecRequest};

fn attempt(exec: &FakeExec) -> String {
    exec.requests()
        .iter()
        .find(|request| request.args.first().is_some_and(|arg| arg == "create"))
        .unwrap()
        .args
        .iter()
        .find_map(|arg| arg.strip_prefix("io.nocterm.attempt=").map(str::to_owned))
        .unwrap()
}

fn inspection_with_attempt(token: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&inspect("created", true)).unwrap();
    value[0]["Config"]["Labels"]["io.nocterm.attempt"] = token.into();
    value.to_string()
}

fn removal_requests(exec: &FakeExec) -> Vec<ExecRequest> {
    exec.requests()
        .into_iter()
        .filter(|request| request.args.first().is_some_and(|arg| arg == "rm"))
        .collect()
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

#[test]
fn failed_later_setup_cleanup_requires_the_successful_create_receipt_attempt() {
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker pull", String::new());
    answer(&exec, "docker start", String::new());
    answer(&exec, "docker rm", String::new());
    answer(&exec, "docker exec", String::new());
    let install = exec.queue_stream(format!("docker exec --interactive {} sh -s", id()));
    let mut pending = runtime().prepare(
        Arc::new(exec.clone()),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            languages: vec![LanguageSelection::default_for(ProgrammingLanguage::Python)],
            root: None,
        }),
        CancelFlag::default(),
        Arc::new(|_| {}),
    );
    assert!(pending.as_mut().now_or_never().is_none());
    answer(
        &exec,
        "docker inspect",
        inspection_with_attempt(&attempt(&exec)),
    );
    install.finish(ExecExit {
        status: Some(1),
        stderr: String::new(),
    });
    drop(install);
    assert!(matches!(
        block_on(pending),
        Err(ProjectSetupError::Environment(_))
    ));
    assert_eq!(removal_requests(&exec).len(), 1);
}
