//! A private read-only local endpoint; no credential or execution operations.
use crate::RunnerError;
use nocterm_hub_io::cloud::RunnerHeadWork;
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};
const SOCKET: &str = "control.sock";
const WAIT: Duration = Duration::from_millis(500);
const RESPONSE_BYTES: usize = 256;

pub(super) struct Endpoint {
    listener: UnixListener,
    cleanup: SocketPath,
}
struct SocketPath {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl SocketPath {
    fn matches(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok_and(|meta| {
            meta.file_type().is_socket() && meta.dev() == self.device && meta.ino() == self.inode
        })
    }
}
impl Drop for SocketPath {
    fn drop(&mut self) {
        if self.matches() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
impl Endpoint {
    /// Called only after holding the native identity/material/journal locks.
    /// Those same process locks prevent unlinking a live peer's endpoint.
    pub(super) fn bind(directory: &Path) -> Result<Self, RunnerError> {
        let path = directory.join(SOCKET);
        let parent = std::fs::symlink_metadata(directory).map_err(RunnerError::Control)?;
        if !parent.is_dir() || parent.permissions().mode() & 0o077 != 0 {
            return Err(RunnerError::Value("private state directory"));
        }
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() && meta.uid() == parent.uid() => {
                std::fs::remove_file(&path).map_err(RunnerError::Control)?
            }
            Ok(_) => return Err(RunnerError::Value("control endpoint type or owner")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(RunnerError::Control(error)),
        }
        let listener = UnixListener::bind(&path).map_err(RunnerError::Control)?;
        let meta = std::fs::symlink_metadata(&path).map_err(RunnerError::Control)?;
        let cleanup = SocketPath {
            path,
            device: meta.dev(),
            inode: meta.ino(),
        };
        std::fs::set_permissions(&cleanup.path, std::fs::Permissions::from_mode(0o600))
            .map_err(RunnerError::Control)?;
        Ok(Self { listener, cleanup })
    }
    pub(super) async fn serve(
        self,
        ready: Arc<AtomicBool>,
        executing: Arc<AtomicBool>,
        work: Arc<RunnerHeadWork>,
    ) -> Result<(), RunnerError> {
        loop {
            let (mut socket, _) = self.listener.accept().await.map_err(RunnerError::Control)?;
            let _ = tokio::time::timeout(WAIT, async {
                let mut request = [0u8; 7];
                socket.read_exact(&mut request).await?;
                if &request != b"health\n" { return Ok::<_, std::io::Error>(()); }
                let response = format!("nocterm-runner-health-v1\nmode=control-plane\nexecution_ready={}\nready={}\nhead_reads={}\nhead_operations={}\n", executing.load(Ordering::Acquire), ready.load(Ordering::Acquire), work.in_flight_reads(), work.in_flight_operations());
                socket.write_all(response.as_bytes()).await?;
                socket.shutdown().await
            }).await;
            // Keep the exact socket inode owner through every accepted client.
            if !self.cleanup.matches() {
                return Err(RunnerError::Value("control endpoint replaced"));
            }
        }
    }
}
pub(super) async fn query(directory: &Path) -> Result<(), RunnerError> {
    let response = read(directory).await?;
    print!("{response}");
    Ok(())
}
async fn read(directory: &Path) -> Result<String, RunnerError> {
    tokio::time::timeout(WAIT, async {
        let mut socket = UnixStream::connect(directory.join(SOCKET))
            .await
            .map_err(RunnerError::Control)?;
        socket
            .write_all(b"health\n")
            .await
            .map_err(RunnerError::Control)?;
        let mut bytes = Vec::with_capacity(RESPONSE_BYTES + 1);
        socket
            .take((RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(RunnerError::Control)?;
        let text = String::from_utf8(bytes).map_err(|_| RunnerError::Value("control response"))?;
        if text.len() > RESPONSE_BYTES
            || !["true", "false"].iter().any(|ready| {
                text.starts_with(&format!(
                    "nocterm-runner-health-v1\nmode=control-plane\nexecution_ready={ready}\n"
                ))
            })
            || !text
                .bytes()
                .all(|value| value == b'\n' || value.is_ascii_graphic())
        {
            return Err(RunnerError::Value("control response"));
        }
        Ok(text)
    })
    .await
    .map_err(|_| RunnerError::Timeout)?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn runner_control_health_retains_actual_endpoint_and_removes_only_its_socket() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = Endpoint::bind(directory.path()).unwrap();
        let work = super::super::worker::work().unwrap();
        let ready = Arc::new(AtomicBool::new(true));
        let executing = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(endpoint.serve(ready, executing.clone(), work));
        let response = read(directory.path()).await.unwrap();
        assert!(response.contains("ready=true\n"));
        assert!(response.contains("execution_ready=false\n"));
        executing.store(true, Ordering::Release);
        let response = read(directory.path()).await.unwrap();
        assert!(response.contains("execution_ready=true\n"));
        task.abort();
        let _ = task.await;
        assert!(!directory.path().join(SOCKET).exists());
        std::fs::write(directory.path().join(SOCKET), b"unrelated").unwrap();
        assert!(Endpoint::bind(directory.path()).is_err());
        assert_eq!(
            std::fs::read(directory.path().join(SOCKET)).unwrap(),
            b"unrelated"
        );
    }
}
