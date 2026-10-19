//! Paid allocation owners used by tempfile's single original anonymous engine.
use nocterm_ai::model::json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch};
use nocterm_core::fs::{
    AdmittedAnonymousError, AdmittedPath, CurrentDirectoryBuffer, CurrentDirectoryError,
    DirectoryObservation, PathAdmissionError, PreparedPathJoin,
};
use std::{io, path::Path};
use tempfile::anonymous::{AnonymousOperations, Name, NameAllocationError};

#[derive(Debug, thiserror::Error)]
pub(super) enum CreationError {
    #[error(transparent)]
    Admission(#[from] NativeAllocationError),
    #[error(transparent)]
    Path(#[from] PathAdmissionError),
    #[error(transparent)]
    Directory(#[from] CurrentDirectoryError),
    #[error(transparent)]
    Name(#[from] NameAllocationError),
    #[error(transparent)]
    Native(#[from] io::Error),
    #[error(transparent)]
    Exhausted(AdmittedAnonymousError<NativeScratch>),
}
impl CreationError {
    /// The original native error of a failed native call, without wrappers.
    pub(super) fn into_native(self) -> Result<io::Error, Self> {
        match self {
            Self::Native(error) | Self::Directory(CurrentDirectoryError::Native(error)) => {
                Ok(error)
            }
            cause => Err(cause),
        }
    }
    /// The concrete error this transparent wrapper displays.
    pub(super) fn concrete(&self) -> &(dyn std::error::Error + 'static) {
        match self {
            Self::Admission(error) => error,
            Self::Path(error) | Self::Directory(CurrentDirectoryError::Admission(error)) => error,
            Self::Directory(CurrentDirectoryError::Native(error)) | Self::Native(error) => error,
            Self::Name(error) => error,
            Self::Exhausted(error) => error,
        }
    }
}
pub(super) struct Operations<'a> {
    ledger: &'a DecodeLedger,
}
impl<'a> Operations<'a> {
    pub(super) fn new(ledger: &'a DecodeLedger) -> Self {
        Self { ledger }
    }
    pub(super) fn credit(&self, bytes: usize) -> Result<NativeScratch, CreationError> {
        Ok(NativeScratch::reserve_native(self.ledger, bytes)?)
    }
    pub(super) fn copy(&self, path: &Path) -> Result<AdmittedPath<NativeScratch>, CreationError> {
        let credit = self.credit(AdmittedPath::<NativeScratch>::copy_capacity(path))?;
        Ok(AdmittedPath::try_copy(path, credit)?)
    }
}
impl AnonymousOperations for Operations<'_> {
    type Path = AdmittedPath<NativeScratch>;
    type Credit = NativeScratch;
    type Error = CreationError;
    fn current_dir(&mut self) -> Result<Self::Path, Self::Error> {
        let mut source = CurrentDirectoryBuffer::new(1, self.credit(1)?)?;
        loop {
            match source.query()? {
                DirectoryObservation::Native(path) => return self.copy(path),
                DirectoryObservation::Required(capacity) => {
                    let replacement =
                        CurrentDirectoryBuffer::new(capacity, self.credit(capacity)?)?;
                    source = replacement;
                }
            }
        }
    }
    fn join(&mut self, base: &Path, right: &Path) -> Result<Self::Path, Self::Error> {
        let scratch = self.credit(PreparedPathJoin::<NativeScratch>::scratch_bytes(
            base, right,
        )?)?;
        let plan = PreparedPathJoin::new(base, right, scratch)?;
        let credit = self.credit(plan.capacity()?)?;
        Ok(plan.allocate(credit)?)
    }
    fn name_buffer(&mut self, capacity: usize) -> Result<Name<Self::Credit>, Self::Error> {
        let credit = self.credit(capacity)?;
        Ok(Name::try_with_capacity(capacity, credit)?)
    }
    fn native_path(&mut self, path: &Path) -> Result<Self::Credit, Self::Error> {
        Ok(super::super::managed::open_scratch(path, self.ledger)?)
    }
    fn native_error(&mut self, error: io::Error) -> Self::Error {
        CreationError::Native(error)
    }
    fn error_kind(error: &Self::Error) -> Option<io::ErrorKind> {
        match error {
            CreationError::Native(error) => Some(error.kind()),
            _ => None,
        }
    }
    fn exhausted(&mut self, base: &Path) -> Self::Error {
        let prepare = || {
            let path = self.copy(base)?;
            let bytes = AdmittedAnonymousError::<NativeScratch>::nested_allocation_bytes()?;
            let nested = self.credit(bytes)?;
            Ok::<_, CreationError>(AdmittedAnonymousError::exhaustion(path, nested))
        };
        match prepare() {
            Ok(error) => CreationError::Exhausted(error),
            Err(error) => error,
        }
    }
}
