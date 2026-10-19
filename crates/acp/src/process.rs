use std::path::Path;

mod lifecycle;
pub(crate) use lifecycle::{CleanupError, ProcessGroup};
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub(crate) mod systemd;
#[cfg(target_os = "linux")]
pub use host::{AgentHostError, run_agent_host};

pub(crate) fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
