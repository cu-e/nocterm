//! Report output without the workspace-denied print macros.
use crate::RunnerError;
use std::io::Write;

/// Writes one complete report to standard output and flushes it.
pub(crate) fn report(text: &str) -> Result<(), RunnerError> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(RunnerError::Output)
}
