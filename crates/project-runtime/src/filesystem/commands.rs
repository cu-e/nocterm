use super::scripts;
use futures::{
    future::{Either, select},
    pin_mut,
};
use nocterm_project_providers::ContainerLocation;
use nocterm_session::{ExecError, ExecOutput, ExecRequest, FsError, HostExec};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub(super) struct FilesystemCommands {
    exec: Arc<dyn HostExec>,
    location: ContainerLocation,
}
impl FilesystemCommands {
    pub(super) fn new(exec: Arc<dyn HostExec>, location: ContainerLocation) -> Self {
        Self { exec, location }
    }
    pub(super) async fn open(
        &self,
        script: &str,
        args: &[String],
        stdin: Option<Vec<u8>>,
    ) -> Result<ExecOutput, FsError> {
        let source = format!("{}\n{}", scripts::COMMON, script);
        let mut program = ExecRequest::new("sh")
            .args(["-c", &source, "nocterm-files"])
            .args(args.iter().cloned());
        program.stdin = stdin;
        let mut scoped = self.location.exec_request(program, false);
        scoped.args.splice(2..2, ["--workdir".into(), "/".into()]);
        timed(self.exec.exec(scoped), Duration::from_secs(15))
            .await?
            .map_err(execution)
    }
    pub(super) async fn run(
        &self,
        script: &str,
        args: &[String],
        stdin: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<Vec<u8>, FsError> {
        let path = args.first().cloned().unwrap_or_else(|| "/".into());
        timed(
            async {
                let mut output = self.open(script, args, stdin).await?;
                let mut bytes = Vec::new();
                while let Some(chunk) = output.next().await {
                    if bytes.len().saturating_add(chunk.len()) > limit {
                        return Err(FsError::Other(
                            "container filesystem response exceeded its bound".into(),
                        ));
                    }
                    bytes.extend(chunk);
                }
                let exit = output.exit().await;
                if !exit.success() {
                    return Err(status(exit.status, &path));
                }
                Ok(bytes)
            },
            Duration::from_secs(15),
        )
        .await?
    }
}
pub(super) async fn timed<T>(
    future: impl std::future::Future<Output = T>,
    timeout: Duration,
) -> Result<T, FsError> {
    let deadline = async_io::Timer::after(timeout);
    pin_mut!(future, deadline);
    match select(future, deadline).await {
        Either::Left((value, _)) => Ok(value),
        Either::Right(_) => Err(FsError::Other(
            "container filesystem request timed out".into(),
        )),
    }
}
pub(super) fn status(code: Option<u32>, path: &str) -> FsError {
    match code {
    Some(71)=>FsError::PermissionDenied {path:path.into()},Some(72)=>FsError::NotFound {path:path.into()},Some(73)=>FsError::AlreadyExists {path:path.into()},
    Some(76)|Some(127)=>FsError::Unsupported("container requires Linux /proc and compatible sh/stat/find/ln/mv utilities; the requested object may also be unsupported".into()),
    Some(75)=>FsError::Other("container path or staged file changed identity; operation refused".into()),
    status=>FsError::Other(format!("container filesystem command failed (status {status:?})")),
}
}
fn execution(error: ExecError) -> FsError {
    match error {
        ExecError::Disconnected => FsError::NotConnected,
        ExecError::NotFound(_) | ExecError::Unsupported => FsError::Unsupported(
            "container execution or required filesystem utilities unavailable".into(),
        ),
        error => FsError::Other(error.to_string()),
    }
}
pub(super) fn checked_path(path: &str, mutation: bool) -> Result<String, FsError> {
    let trimmed = path.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { "/" } else { trimmed };
    if !path.starts_with('/')
        || path.contains('\0')
        || path.len() > 4096
        || path.split('/').any(|part| matches!(part, "." | ".."))
        || mutation && trimmed == "/"
    {
        return Err(FsError::Other(
            "container paths must be bounded absolute paths without NUL or dot components".into(),
        ));
    }
    Ok(trimmed.into())
}
