//! Anonymous private replay only when the original opened source cannot replay.
#![expect(
    clippy::result_large_err,
    reason = "exhaustion carries its prepaid path by value; a Box would be an unpaid allocation"
)]
#[cfg(target_os = "linux")]
mod creation;
#[cfg(target_os = "linux")]
mod operations;
use super::{HeldReadError, Operation, managed::ReadFaultContext, pass::PassReader};
use std::{
    fs::File,
    io::{Read as _, Seek as _, Write as _},
    path::Path,
};
pub(super) struct Captured {
    file: File,
    fingerprint: ([u8; 32], u64),
}
impl Captured {
    pub(super) fn into_parts(self) -> (File, ([u8; 32], u64)) {
        (self.file, self.fingerprint)
    }
}
#[cfg(test)]
pub(super) fn capture(source: File, path: &Path) -> Result<Captured, HeldReadError> {
    capture_with_context(source, path, None, None)
}
pub(super) fn capture_with_context(
    mut source: File,
    path: &Path,
    context: Option<&ReadFaultContext>,
    ledger: Option<&nocterm_ai::model::json::deferred::DecodeLedger>,
) -> Result<Captured, HeldReadError> {
    let mut replay = match ledger {
        #[cfg(target_os = "linux")]
        Some(ledger) => creation::create(path, ledger).map_err(HeldReadError::ManagedReplay)?,
        #[cfg(not(target_os = "linux"))]
        Some(_) => return Err(HeldReadError::ReplayPlatformUnavailable),
        None => tempfile::tempfile()
            .map_err(|error| super::native_fault(context, Operation::CreateReplay, path, error))?,
    };
    // Reuse exactly the same native bound, SHA and concrete read-cause owner.
    let mut input = PassReader::with_context(&mut source, context);
    let mut buffer = [0; 256];
    loop {
        let count = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => {
                input.check_fault(path)?;
                continue;
            }
        };
        replay
            .write_all(&buffer[..count])
            .map_err(|error| super::native_fault(context, Operation::WriteReplay, path, error))?;
    }
    input.check_fault(path)?;
    let fingerprint = input.finish();
    replay
        .rewind()
        .map_err(|error| super::native_fault(context, Operation::SeekReplay, path, error))?;
    // Original descriptor is consumed once and dropped here; never reopen path.
    Ok(Captured {
        file: replay,
        fingerprint,
    })
}
