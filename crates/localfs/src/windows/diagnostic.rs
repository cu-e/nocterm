//! Native operation context captured before error classification loses Win32 codes.
use nocterm_session::local_fs::{LocalFsError, LocalResult};
use std::{io, path::Path};

pub(crate) fn checked<T>(
    operation: &'static str,
    path: &Path,
    access: u32,
    share: u32,
    flags: u32,
    result: io::Result<T>,
) -> LocalResult<T> {
    let build_source = option_env!("NOCTERM_BUILD_REVISION").unwrap_or("unversioned");
    match result {
        Ok(value) => {
            tracing::debug!(
                build_source, pid = std::process::id(), operation, component_path = ?path,
                component = ?path.file_name(), requested_access = access,
                share_mode = share, requested_flags_or_options = flags,
                "native filesystem operation succeeded",
            );
            Ok(value)
        }
        Err(error) => {
            tracing::warn!(
                build_source, pid = std::process::id(), operation, component_path = ?path,
                component = ?path.file_name(), requested_access = access,
                share_mode = share, requested_flags_or_options = flags,
                native_error = ?error.raw_os_error(), kind = ?error.kind(),
                "native filesystem operation failed",
            );
            Err(LocalFsError::from_io(path, error))
        }
    }
}
