//! Optional native exercise of the same preparation adapter installed by the app.
mod environment;
mod filesystem;
mod monitor;
use std::sync::Arc;

use futures::executor::block_on;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectMetadataSource as _};
use nocterm_project_providers::ContainerSessionFactory as _;
use nocterm_project_providers::{
    ContainerLocation, LanguageSelection, OwnedContainer, ProgrammingLanguage,
    ProjectPreparation as _, ProjectSetupRequest, SetupMode, WorkspaceImage,
};
use nocterm_session::{
    Auth, ConnectRequest, ExecRequest, HostExec as _, RemoteFs as _, Session, SessionDriver,
    ShellLaunch, Target,
    local_fs::CancelFlag,
    testing::{InMemoryFs, ScriptedTransport},
};
use std::time::Duration;

#[test]
#[ignore = "requires Docker and network access to download an image and toolchains"]
fn a_real_docker_workspace_installs_tools_and_attaches_without_owning_the_container() {
    let (runtime, exec, owned) = prepare_native(
        WorkspaceImage::Ubuntu,
        ProgrammingLanguage::ALL
            .into_iter()
            .map(LanguageSelection::default_for)
            .collect(),
    );
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        exercise_prepared(&runtime, exec.clone(), &owned)
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap().unwrap();
    removed.unwrap();
}

#[test]
#[ignore = "requires Docker and network access to pull the Ubuntu image"]
fn a_real_docker_creation_attempt_is_labeled_and_exactly_removed() {
    let (runtime, exec, owned) = prepare_native(
        WorkspaceImage::Custom("docker.io/library/ubuntu:24.04".into()),
        vec![],
    );
    let id = owned.location().container().to_owned();
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let listing = block_on(async {
            exec.exec(ExecRequest::new("docker").args([
                "ps",
                "--all",
                "--no-trunc",
                "--quiet",
                "--filter",
                &format!("label=io.nocterm.attempt={}", owned.attempt().as_str()),
            ]))
            .await
            .unwrap()
            .collect(8192)
            .await
        });
        assert!(listing.exit.success());
        assert_eq!(listing.text().trim(), id);
    }));
    let removed = block_on(runtime.discard_owned(exec.clone(), owned));
    checked.unwrap();
    removed.unwrap();
    let inspected = block_on(async {
        exec.exec(ExecRequest::new("docker").args(["inspect", &id]))
            .await
            .unwrap()
            .collect(8192)
            .await
    });
    assert!(
        !inspected.exit.success(),
        "owned container survived cleanup"
    );
}

fn prepare_native(
    image: WorkspaceImage,
    languages: Vec<LanguageSelection>,
) -> (
    nocterm_project_runtime::NativeProjectPreparation,
    Arc<nocterm_local::LocalExec>,
    OwnedContainer,
) {
    let project = nocterm_project_storage::NativeProjectMetadata
        .new_identity()
        .unwrap()
        .id;
    let runtime = nocterm_project_runtime::NativeProjectPreparation::new(Arc::new(
        nocterm_project_runtime::NativeWorkspaceEnvironment::new(Arc::new(
            nocterm_project_runtime::NativeEnvironmentCommands,
        )),
    ));
    let exec = Arc::new(nocterm_local::LocalExec);
    let input = ProjectSetupRequest {
        project,
        host: HostLocator::Local,
        mode: SetupMode::Create {
            engine: Engine::Docker,
            image,
            languages,
            root: None,
        },
        repository: None,
    };
    let prepared = block_on(runtime.prepare(
        exec.clone(),
        input,
        CancelFlag::default(),
        Arc::new(|phase| tracing::info!(phase = phase.label(), "Preparing native test workspace")),
    ))
    .unwrap();
    (runtime, exec, prepared.owned.unwrap())
}

fn exercise_prepared(
    runtime: &nocterm_project_runtime::NativeProjectPreparation,
    exec: Arc<nocterm_local::LocalExec>,
    owned: &OwnedContainer,
) -> Result<(), Box<dyn std::error::Error>> {
    let location = owned.location();
    let attached = block_on(runtime.prepare(
        exec.clone(),
        ProjectSetupRequest {
            project: owned.project(),
            host: HostLocator::Local,
            mode: SetupMode::Attach {
                engine: Engine::Docker,
                container: location.container().to_owned(),
                root: None,
            },
            repository: None,
        },
        CancelFlag::default(),
        Arc::new(|_| {}),
    ))?;
    assert!(attached.owned.is_none());
    assert_eq!(
        ContainerLocation::from_target(&attached.target)?.container(),
        location.container()
    );
    let versions = ExecRequest::new("docker").args([
        "exec", "--workdir", location.root().as_str(), location.container(), "sh", "-c",
        "set -eu; . \"$HOME/.local/share/nocterm/toolchains/env.sh\"; test \"$(pwd)\" = /workspace; python3 --version; rustc --version; bun --version; node --version; javac -version",
    ]);
    let output = block_on(async {
        Ok::<_, nocterm_session::ExecError>(exec.exec(versions).await?.collect(8192).await)
    })?;
    assert!(
        output.exit.success(),
        "tool verification failed: {:?}",
        output.exit
    );
    assert!(output.text().contains("Python"));
    Ok(())
}

#[test]
#[ignore = "requires Docker and network access for five explicitly selected toolchains"]
fn a_real_ubuntu_workspace_installs_selected_language_versions() {
    selected_versions(WorkspaceImage::Ubuntu);
}

#[test]
#[ignore = "requires Docker and network access for five explicitly selected toolchains"]
fn a_real_debian_workspace_installs_selected_language_versions() {
    selected_versions(WorkspaceImage::Debian);
}

fn selected_versions(image: WorkspaceImage) {
    use ProgrammingLanguage::{Bun, Java, Node, Python, Rust};
    let languages = [
        (Python, "3.13"),
        (Rust, "1.90.0"),
        (Bun, "1.3.3"),
        (Node, "22"),
        (Java, "21"),
    ]
    .into_iter()
    .map(|(language, version)| LanguageSelection::parse(language, version).unwrap())
    .collect();
    let (runtime, exec, owned) = prepare_native(image, languages);
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let output = block_on(async {
            exec.exec(ExecRequest::new("docker").args([
                "exec",
                owned.location().container(),
                "sh",
                "-c",
                r#"set -eu
. "$HOME/.local/share/nocterm/toolchains/env.sh"
python --version
python3 --version
python3 -m venv /tmp/nocterm-version-venv
/tmp/nocterm-version-venv/bin/python -m pip --version
python3 -m pip --version
rustc --version
bun --version
node --version
npm --version
java -version 2>&1
javac -version
test -n "$JAVA_HOME"
test -x "$JAVA_HOME/bin/java"
"$JAVA_HOME/bin/java" -version 2>&1
"#,
            ]))
            .await
            .unwrap()
            .collect(32 * 1024)
            .await
        });
        let text = output.text();
        assert!(
            output.exit.success(),
            "selected tool verification failed: {text}"
        );
        assert!(text.matches("Python 3.13.").count() >= 2, "{text}");
        assert!(
            text.contains("pip ") && text.contains("python 3.13"),
            "{text}"
        );
        assert!(text.contains("rustc 1.90.0"), "{text}");
        assert!(text.lines().any(|line| line.trim() == "1.3.3"), "{text}");
        assert!(text.lines().any(|line| line.starts_with("v22.")), "{text}");
        assert!(
            text.lines().any(|line| line.starts_with("javac 21.")),
            "{text}"
        );
        assert!(text.matches("version \"21.").count() >= 2, "{text}");
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

#[test]
#[ignore = "requires Docker and a live Docker Hub repository search"]
fn a_real_docker_registry_search_returns_repository_metadata() {
    use nocterm_project_providers::{ImageSearch as _, ImageSearchQuery};
    let images = block_on(nocterm_project_runtime::NativeImageSearch.search(
        Arc::new(nocterm_local::LocalExec),
        Engine::Docker,
        ImageSearchQuery::parse("debian").unwrap(),
        CancelFlag::default(),
    ))
    .unwrap();
    assert!(!images.is_empty());
    assert!(
        images
            .iter()
            .any(|image| image.reference == "docker.io/library/debian" && image.official)
    );
}

#[test]
#[ignore = "requires Docker and a cached Ubuntu image; creates by immutable local image ID"]
fn a_real_cached_image_id_creates_without_a_registry_pull() {
    let exec = nocterm_local::LocalExec;
    let inspected = block_on(async {
        exec.exec(ExecRequest::new("docker").args([
            "image",
            "inspect",
            "--format",
            "{{.Id}}",
            "docker.io/library/ubuntu:24.04",
        ]))
        .await
        .unwrap()
        .collect(8192)
        .await
    });
    assert!(inspected.exit.success(), "cached Ubuntu image required");
    let image = inspected.text().trim().to_owned();
    assert!(image.starts_with("sha256:"));
    let (runtime, exec, owned) = prepare_native(WorkspaceImage::Custom(image.clone()), vec![]);
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let inspected = block_on(async {
            exec.exec(ExecRequest::new("docker").args([
                "inspect",
                "--format",
                "{{.Image}}",
                owned.location().container(),
            ]))
            .await
            .unwrap()
            .collect(8192)
            .await
        });
        assert!(inspected.exit.success());
        assert_eq!(inspected.text().trim(), image);
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

fn container_session(
    exec: Arc<dyn nocterm_session::HostExec>,
    owned: &OwnedContainer,
) -> (Session, SessionDriver) {
    let host_files = Arc::new(InMemoryFs::default());
    block_on(host_files.create_dir("/host-only-sentinel")).unwrap();
    host_files.set_home("/host-only-sentinel").unwrap();
    let base = ScriptedTransport::default();
    let driver = base.queue(Some(host_files), Some(exec));
    let transport = nocterm_project_container_fs::NativeContainerSessions::new(Arc::new(
        nocterm_project_runtime::NativeWorkspaceSessions,
    ))
    .wrap(Arc::new(base), owned.location().clone());
    let session = transport.open(ConnectRequest {
        proxy: Default::default(),
        launch: ShellLaunch::default(),
        target: Target::new("native-test", "original-host", 22),
        auth: Auth::Auto,
        jumps: vec![],
        term: "xterm".into(),
        size: Default::default(),
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: None,
    });
    (session, driver)
}
