use crate::{command::Commands, inspect::inspect, ownership::CreationAttempt};
use futures::{FutureExt as _, future::join_all};
use nocterm_containers::{ContainersError, Engine};
use nocterm_project_model::{ProjectTarget, RootPath};
use nocterm_project_providers::{
    ContainerLocation, ContainerOrigin, CreatedWorkspaceEnvironment, EngineInventory,
    OwnedContainer, PreparedProject, ProjectPreparation, ProjectSetupError, ProjectSetupRequest,
    SetupFuture, SetupMode, SetupPhase, SetupProgress, WorkspaceImage,
};
use nocterm_session::{ExecRequest, HostExec, local_fs::CancelFlag};
use std::sync::Arc;

const DIRECTORY: &str =
    "set -eu; mkdir -p -- \"$1\"; test -d \"$1\"; test -r \"$1\"; test -x \"$1\"";
const CLONE: &str = "set -eu; export GIT_TERMINAL_PROMPT=0; command -v git >/dev/null 2>&1 || exit 76; test ! -L \"$2\" || exit 73; if test -e \"$2\"; then test -d \"$2\" || exit 74; test -z \"$(find \"$2\" -mindepth 1 -maxdepth 1 -print -quit)\" || exit 73; fi; git clone -- \"$1\" \"$2\"";

#[derive(Clone)]
pub struct NativeProjectPreparation {
    environment: Arc<dyn CreatedWorkspaceEnvironment>,
}
impl NativeProjectPreparation {
    pub fn new(environment: Arc<dyn CreatedWorkspaceEnvironment>) -> Self {
        Self { environment }
    }
}
impl ProjectPreparation for NativeProjectPreparation {
    fn discover(&self, exec: Arc<dyn HostExec>) -> SetupFuture<Vec<EngineInventory>> {
        async move {
            Ok(join_all(Engine::ALL.into_iter().map(|engine| {
                let exec = exec.clone();
                async move {
                    let (containers, images) = futures::join!(
                        listing(exec.as_ref(), engine.container_listing()),
                        listing(exec.as_ref(), engine.image_listing())
                    );
                    EngineInventory {
                        engine,
                        containers: containers.and_then(|output| {
                            nocterm_containers::decode_containers(&output)
                                .map_err(ContainersError::Parse)
                        }),
                        images: images.and_then(|output| {
                            nocterm_containers::decode_images(&output)
                                .map_err(ContainersError::Parse)
                        }),
                    }
                }
            }))
            .await)
        }
        .boxed()
    }
    fn prepare(
        &self,
        exec: Arc<dyn HostExec>,
        request: ProjectSetupRequest,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> SetupFuture<PreparedProject> {
        let runtime = self.clone();
        async move {
            request.validate()?;
            let commands = Commands::new(exec.clone(), cancel, progress);
            let mut owned = None;
            let result =
                prepare(&commands, runtime.environment.as_ref(), request, &mut owned).await;
            match (result, owned) {
                (Ok(target), owned) => Ok(PreparedProject { target, owned }),
                (Err(error), Some(receipt)) => {
                    let container = receipt.location().container().to_owned();
                    match runtime.discard_owned(exec, receipt).await {
                        Ok(()) => Err(error),
                        Err(cleanup) => Err(ProjectSetupError::CleanupFailed {
                            container,
                            failure: Box::new(error),
                            cleanup: Box::new(cleanup),
                        }),
                    }
                }
                (Err(error), None) => Err(error),
            }
        }
        .boxed()
    }
    fn discard_owned(&self, exec: Arc<dyn HostExec>, receipt: OwnedContainer) -> SetupFuture<()> {
        async move {
            let commands = Commands::new(exec, CancelFlag::default(), Arc::new(|_| {}));
            let location = receipt.location();
            let current = inspect(&commands, location.engine(), location.container()).await?;
            if !current.owned_by(receipt.project(), receipt.attempt()) {
                return Err(ProjectSetupError::CleanupRefused);
            }
            commands
                .run(
                    SetupPhase::CreatingContainer,
                    location
                        .engine()
                        .command()
                        .args(["rm", "--force", location.container()]),
                )
                .await?;
            Ok(())
        }
        .boxed()
    }
}

async fn prepare(
    commands: &Commands,
    environment: &dyn CreatedWorkspaceEnvironment,
    request: ProjectSetupRequest,
    owned: &mut Option<OwnedContainer>,
) -> Result<ProjectTarget, ProjectSetupError> {
    commands.check()?;
    match &request.mode {
        SetupMode::Direct { root } => direct(commands, &request, root).await,
        SetupMode::Attach {
            engine,
            container,
            root,
        } => attach(commands, &request, *engine, container, root.as_ref()).await,
        SetupMode::Create {
            engine,
            image,
            languages,
            root,
        } => {
            let root = root
                .clone()
                .map(Ok)
                .unwrap_or_else(|| RootPath::parse("/workspace"))?;
            let receipt = create(commands, *engine, image, &request, root).await?;
            let location = receipt.location().clone();
            *owned = Some(receipt.clone());
            ensure_started(commands, *engine, location.container(), "created").await?;
            if !languages.is_empty() || !matches!(image, WorkspaceImage::Custom(_)) {
                environment
                    .install_created(Arc::new(commands.clone()), receipt, languages.clone())
                    .await
                    .map_err(|error| ProjectSetupError::Environment(Box::new(error)))?;
            }
            prepare_folder(commands, &location, request.repository.as_ref()).await?;
            location.to_target()
        }
    }
}
async fn direct(
    commands: &Commands,
    request: &ProjectSetupRequest,
    root: &RootPath,
) -> Result<ProjectTarget, ProjectSetupError> {
    if let Some(repository) = &request.repository {
        commands
            .run(
                SetupPhase::CloningRepository,
                ExecRequest::new("sh").args([
                    "-c",
                    CLONE,
                    "nocterm-clone",
                    repository.as_str(),
                    root.as_str(),
                ]),
            )
            .await?;
    }
    Ok(ProjectTarget::HostFolder {
        host: request.host.clone(),
        root: root.clone(),
    })
}
async fn attach(
    commands: &Commands,
    request: &ProjectSetupRequest,
    engine: Engine,
    container: &str,
    root: Option<&RootPath>,
) -> Result<ProjectTarget, ProjectSetupError> {
    let state = inspect(commands, engine, container).await?;
    ensure_started(commands, engine, container, &state.state.status).await?;
    let root = root.cloned().map(Ok).unwrap_or_else(|| {
        RootPath::parse(if state.config.workdir.is_empty() {
            "/"
        } else {
            &state.config.workdir
        })
    })?;
    let location = ContainerLocation::new(
        request.host.clone(),
        engine,
        container.to_owned(),
        root,
        ContainerOrigin::Attached,
    )?;
    prepare_folder(commands, &location, request.repository.as_ref()).await?;
    location.to_target()
}
async fn create(
    commands: &Commands,
    engine: Engine,
    image: &WorkspaceImage,
    source: &ProjectSetupRequest,
    root: RootPath,
) -> Result<OwnedContainer, ProjectSetupError> {
    let custom = matches!(image, WorkspaceImage::Custom(_));
    if !custom {
        commands
            .run(
                SetupPhase::PullingImage,
                pull_request(engine, image.reference()),
            )
            .await?;
    }
    let attempt = CreationAttempt::new(source.host.clone(), root, engine, source.project)?;
    let mut request = engine.command().args([
        "create",
        "--name",
        &attempt.name(),
        "--label",
        &format!("io.nocterm.project={}", source.project),
        "--label",
        &attempt.label(),
        "--entrypoint",
        "/bin/sh",
    ]);
    if custom {
        request = request.arg("--pull=missing");
    } else {
        request = request.args(["--user", "0"]);
    }
    let result = commands
        .run(
            SetupPhase::CreatingContainer,
            request.args([image.reference(), "-c", "while :; do sleep 3600; done"]),
        )
        .await
        .and_then(|output| attempt.receipt(output.trim()));
    match result {
        Ok(receipt) => Ok(receipt),
        Err(failure) => Err(attempt.reconcile(commands, failure).await),
    }
}

async fn ensure_started(
    commands: &Commands,
    engine: Engine,
    id: &str,
    state: &str,
) -> Result<(), ProjectSetupError> {
    match state {
        "running" => Ok(()),
        "created" | "configured" | "initialized" | "exited" | "stopped" => commands
            .run(
                SetupPhase::StartingContainer,
                engine.command().args(["start", id]),
            )
            .await
            .map(drop),
        _ => Err(ProjectSetupError::UnsupportedState),
    }
}
async fn prepare_folder(
    commands: &Commands,
    location: &ContainerLocation,
    repository: Option<&nocterm_project_providers::GitRepository>,
) -> Result<(), ProjectSetupError> {
    let (phase, request) = if let Some(repository) = repository {
        (
            SetupPhase::CloningRepository,
            ExecRequest::new("sh").args([
                "-c",
                CLONE,
                "nocterm-clone",
                repository.as_str(),
                location.root().as_str(),
            ]),
        )
    } else {
        (
            SetupPhase::PreparingDirectory,
            ExecRequest::new("sh").args([
                "-c",
                DIRECTORY,
                "nocterm-directory",
                location.root().as_str(),
            ]),
        )
    };
    commands
        .run(phase, location.exec_request(request, false))
        .await
        .map(drop)
}
async fn listing(exec: &dyn HostExec, request: ExecRequest) -> Result<String, ContainersError> {
    let operation = async {
        let output = exec.exec(request).await?.collect(16 * 1024 * 1024).await;
        match output.exit.status {
            Some(0) => Ok(output.text()),
            Some(127) => Err(ContainersError::NotInstalled),
            status => Err(ContainersError::Exit {
                status,
                stderr_tail: output.exit.stderr,
            }),
        }
    };
    let timeout = async_io::Timer::after(std::time::Duration::from_secs(30));
    futures::pin_mut!(operation, timeout);
    match futures::future::select(operation, timeout).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => Err(ContainersError::Execution(
            nocterm_session::ExecError::Failed("container discovery timed out".into()),
        )),
    }
}

pub(crate) fn pull_request(engine: Engine, image: &str) -> ExecRequest {
    ExecRequest::new("sh")
        .args(["-c", r#"exec "$@" 2>&1"#, "nocterm-pull"])
        .arg(engine.command().program)
        .args(["pull", image])
}
