#[cfg(unix)]
use std::path::Path;

mod lifecycle;
pub(crate) use lifecycle::{CleanupError, ProcessGroup};
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub(crate) mod systemd;
#[cfg(target_os = "linux")]
pub use host::{AgentHostError, run_agent_host};

#[cfg(unix)]
pub(crate) fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}
