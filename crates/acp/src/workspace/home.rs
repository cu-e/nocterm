//! HOME and the original one-call passwd fallback use admitted caller buffers.
use super::{Home, managed::Cause};
use crate::managed::SourceBudget;
#[cfg(unix)]
use agent_client_protocol::managed::Buffer;
use nocterm_ai::model::native::NativeOwned;
use std::path::PathBuf;
pub(super) enum HomeFacts {
    Current,
    Supplied(NativeOwned<Option<PathBuf>>),
}
impl HomeFacts {
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn prepare(home: &Home, budget: &SourceBudget) -> Result<Self, Cause> {
        Ok(match home {
            Home::Current => Self::Current,
            Home::Supplied(Some(path)) => {
                Self::Supplied(NativeOwned::supplied(budget.context().copy_path(path)?))
            }
            Home::Supplied(None) => Self::Supplied(NativeOwned::absent(budget.ledger())?),
        })
    }
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn resolve(
        self,
        budget: &SourceBudget,
    ) -> Result<NativeOwned<Option<PathBuf>>, Cause> {
        match self {
            Self::Supplied(path) => Ok(path),
            Self::Current => current(budget),
        }
    }
}
#[cfg(unix)]
#[expect(
    clippy::result_large_err,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
fn current(budget: &SourceBudget) -> Result<NativeOwned<Option<PathBuf>>, Cause> {
    use nix::unistd::{EnvironmentObservation, getenv_into};
    use std::{os::unix::ffi::OsStrExt, path::Path};
    let mut bytes = Buffer::new(budget.clone());
    loop {
        match getenv_into(c"HOME", bytes.as_mut_slice()) {
            EnvironmentObservation::Missing | EnvironmentObservation::Copied(0) => {
                return passwd(budget);
            }
            EnvironmentObservation::Copied(count) => {
                let path = Path::new(std::ffi::OsStr::from_bytes(&bytes.as_slice()[..count]));
                return Ok(NativeOwned::supplied(budget.context().copy_path(path)?));
            }
            EnvironmentObservation::Required(count) => bytes = zeroes(budget, count)?,
        }
    }
}
#[cfg(unix)]
#[expect(
    clippy::result_large_err,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
fn zeroes(budget: &SourceBudget, count: usize) -> Result<Buffer<u8, SourceBudget>, Cause> {
    let mut result = Buffer::new(budget.clone());
    result.reserve(count)?;
    for _ in 0..count {
        result.push(0).map_err(|(error, _)| error)?;
    }
    Ok(result)
}
#[cfg(all(
    unix,
    not(any(
        target_os = "android",
        target_os = "emscripten",
        target_os = "redox",
        target_os = "vxworks",
        target_os = "espidf",
        target_os = "horizon",
        target_os = "vita",
        target_os = "nuttx",
        all(target_vendor = "apple", not(target_os = "macos"))
    ))
))]
#[expect(
    clippy::result_large_err,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
fn passwd(budget: &SourceBudget) -> Result<NativeOwned<Option<PathBuf>>, Cause> {
    use std::{os::unix::ffi::OsStrExt, path::Path};
    let mut scratch = zeroes(budget, nix::unistd::passwd_home_buffer_size())?;
    match nix::unistd::passwd_home_into(scratch.as_mut_slice()) {
        Ok(Some(bytes)) => {
            Ok(NativeOwned::supplied(budget.context().copy_path(
                Path::new(std::ffi::OsStr::from_bytes(bytes)),
            )?))
        }
        _ => Ok(NativeOwned::absent(budget.ledger())?),
    }
}
#[cfg(all(
    unix,
    any(
        target_os = "android",
        target_os = "emscripten",
        target_os = "redox",
        target_os = "vxworks",
        target_os = "espidf",
        target_os = "horizon",
        target_os = "vita",
        target_os = "nuttx",
        all(target_vendor = "apple", not(target_os = "macos"))
    )
))]
#[expect(
    clippy::result_large_err,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
fn passwd(budget: &SourceBudget) -> Result<NativeOwned<Option<PathBuf>>, Cause> {
    Ok(NativeOwned::absent(budget.ledger())?)
}

#[cfg(not(unix))]
#[expect(
    clippy::result_large_err,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
fn current(_: &SourceBudget) -> Result<NativeOwned<Option<PathBuf>>, Cause> {
    Err(Cause::Platform)
}
