//! Native environment reads write only into genuinely prepaid caller storage.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{Buffer, ManagedError};
use nocterm_ai::model::json::deferred::JsonDecodeError;
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error(transparent)]
    Native(#[from] ManagedError<JsonDecodeError>),
    #[cfg(unix)]
    #[error("Cannot read native environment: {0}")]
    Observation(#[from] nix::errno::Errno),
    #[cfg(not(unix))]
    #[error("managed native environment is unavailable on this platform")]
    Platform,
}
pub(super) struct Snapshot {
    bytes: Buffer<u8, SourceBudget>,
    used: usize,
}
impl Snapshot {
    #[cfg(unix)]
    pub(super) fn current(budget: SourceBudget) -> Result<Self, Error> {
        let mut bytes = Buffer::new(budget);
        loop {
            match nix::unistd::environment_into(bytes.as_mut_slice())? {
                nix::unistd::EnvironmentSnapshot::Required { bytes: needed, .. } => {
                    bytes.reserve(
                        needed
                            .checked_sub(bytes.len())
                            .ok_or(ManagedError::CapacityOverflow)?,
                    )?;
                    while bytes.len() < needed {
                        bytes.push(0).map_err(|(error, _)| error)?;
                    }
                }
                nix::unistd::EnvironmentSnapshot::Copied { bytes: used, .. } => {
                    return Ok(Self { bytes, used });
                }
            }
        }
    }
    #[cfg(not(unix))]
    pub(super) fn current(_: SourceBudget) -> Result<Self, Error> {
        Err(Error::Platform)
    }
    pub(super) fn raw(&self) -> &[u8] {
        &self.bytes.as_slice()[..self.used]
    }
}
