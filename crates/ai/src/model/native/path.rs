//! Pure native path construction reuses foundation's exact borrowed join plan.
use super::{DecodeLedger, JsonDecodeError, NativeOwned};
use nocterm_core::fs::{PathAdmissionError, PreparedPathJoin};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
#[derive(Debug, thiserror::Error)]
pub enum NativePathError {
    #[error(transparent)]
    Admission(#[from] JsonDecodeError),
    #[error(transparent)]
    Plan(#[from] PathAdmissionError),
}
pub(super) struct PathConstruction<'a> {
    ledger: &'a DecodeLedger,
}
impl<'a> PathConstruction<'a> {
    pub(super) fn new(ledger: &'a DecodeLedger) -> Self {
        Self { ledger }
    }
    pub(super) fn copy(&self, path: &Path) -> Result<NativeOwned<PathBuf>, JsonDecodeError> {
        let capacity = path.as_os_str().as_encoded_bytes().len();
        let allocation = self.ledger.reserve(capacity)?;
        let mut copy = OsString::new();
        copy.try_reserve_exact(capacity)?;
        if copy.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile);
        }
        copy.push(path.as_os_str());
        Ok(NativeOwned::from_parts(copy.into(), allocation))
    }
    pub(super) fn join(
        &self,
        base: &Path,
        right: &Path,
    ) -> Result<NativeOwned<PathBuf>, NativePathError> {
        let scratch_bytes = PreparedPathJoin::<()>::scratch_bytes(base, right)?;
        let scratch = self.ledger.reserve(scratch_bytes)?;
        let prepared = PreparedPathJoin::new(base, right, scratch)?;
        let capacity = prepared.capacity()?;
        let allocation = self.ledger.reserve(capacity)?;
        let mut output = OsString::new();
        output
            .try_reserve_exact(capacity)
            .map_err(JsonDecodeError::from)?;
        if output.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile.into());
        }
        prepared.fill(&mut output)?;
        Ok(NativeOwned::from_parts(output.into(), allocation))
    }
}
