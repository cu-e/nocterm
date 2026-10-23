use futures::{FutureExt as _, pin_mut, select_biased};
use nocterm_project_providers::{ProjectSetupError, SetupPhase, SetupProgress};
use nocterm_session::{ExecRequest, HostExec, local_fs::CancelFlag};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(crate) struct Commands {
    exec: Arc<dyn HostExec>,
    cancel: CancelFlag,
    progress: SetupProgress,
    started: Instant,
    command_timeout: Duration,
    total_timeout: Duration,
}
impl Commands {
    pub(crate) fn new(
        exec: Arc<dyn HostExec>,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> Self {
        Self {
            exec,
            cancel,
            progress,
            started: Instant::now(),
            command_timeout: COMMAND_TIMEOUT,
            total_timeout: TOTAL_TIMEOUT,
        }
    }
    pub(crate) fn lifecycle(self) -> Self {
        Self {
            command_timeout: Duration::from_secs(30),
            total_timeout: Duration::from_secs(75),
            ..self
        }
    }
    pub(crate) fn recovery(&self) -> Self {
        Self {
            exec: self.exec.clone(),
            cancel: CancelFlag::default(),
            progress: std::sync::Arc::new(|_| {}),
            started: Instant::now(),
            command_timeout: Duration::from_secs(30),
            total_timeout: Duration::from_secs(30),
        }
    }
    pub(crate) async fn run(
        &self,
        phase: SetupPhase,
        request: ExecRequest,
    ) -> Result<String, ProjectSetupError> {
        self.check()?;
        (self.progress)(phase.into());
        let operation = async {
            let mut output = self.exec.exec(request).await?;
            let mut bytes = Vec::new();
            let mut decoder = crate::pull_progress::PullProgressDecoder::new();
            while let Some(chunk) = output.next().await {
                if bytes.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
                    return Err(ProjectSetupError::InvalidInput(
                        "command output exceeded its bound",
                    ));
                }
                if matches!(
                    phase,
                    SetupPhase::PullingImage | SetupPhase::CreatingContainer
                ) && let Some(transfer) = decoder.push(&chunk)
                {
                    (self.progress)(nocterm_project_providers::SetupUpdate {
                        phase,
                        transfer: Some(transfer),
                    });
                }
                bytes.extend(chunk);
            }
            let exit = output.exit().await;
            if !exit.success() {
                return Err(command_error(phase, exit.status, exit.stderr));
            }
            if let Some(transfer) = decoder.latest() {
                (self.progress)(nocterm_project_providers::SetupUpdate {
                    phase,
                    transfer: Some(transfer),
                });
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        .fuse();
        let deadline = async_io::Timer::after(self.command_timeout).fuse();
        pin_mut!(operation, deadline);
        loop {
            let tick = async_io::Timer::after(Duration::from_millis(100)).fuse();
            pin_mut!(tick);
            select_biased! {
                result = operation => { self.check()?; return result; },
                _ = deadline => return Err(ProjectSetupError::TimedOut),
                _ = tick => self.check()?,
            }
        }
    }
    pub(crate) fn executor(&self) -> Arc<dyn HostExec> {
        self.exec.clone()
    }
    pub(crate) fn with_executor(&self, exec: Arc<dyn HostExec>) -> Self {
        Self {
            exec,
            cancel: self.cancel.clone(),
            progress: self.progress.clone(),
            started: self.started,
            command_timeout: self.command_timeout,
            total_timeout: self.total_timeout,
        }
    }
    pub(crate) fn observation(&self, exec: Option<Arc<dyn HostExec>>) -> Self {
        Self {
            exec: exec.unwrap_or_else(|| self.exec.clone()),
            cancel: self.cancel.clone(),
            progress: self.progress.clone(),
            started: self.started,
            command_timeout: Duration::from_secs(15),
            total_timeout: self.total_timeout,
        }
    }
    pub(crate) fn check(&self) -> Result<(), ProjectSetupError> {
        if self.cancel.is_cancelled() {
            return Err(ProjectSetupError::Canceled);
        }
        if self.started.elapsed() >= self.total_timeout {
            return Err(ProjectSetupError::TimedOut);
        }
        Ok(())
    }
}

fn command_error(phase: SetupPhase, status: Option<u32>, stderr_tail: String) -> ProjectSetupError {
    match (phase, status) {
        (SetupPhase::CloningRepository, Some(73)) => ProjectSetupError::CloneDestinationNotEmpty,
        (SetupPhase::CloningRepository, Some(74)) => ProjectSetupError::DirectoryUnavailable,
        (SetupPhase::CloningRepository, Some(76)) => ProjectSetupError::GitUnavailable,
        _ => ProjectSetupError::CommandFailed {
            phase: phase.label(),
            status,
            stderr_tail,
        },
    }
}
