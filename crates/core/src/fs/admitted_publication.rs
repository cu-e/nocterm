//! Private staging keeps actual admitted paths and their credits inseparable.
mod error_layout;
pub use error_layout::io_error_custom_allocation_bytes;
#[cfg(unix)]
mod scratch;
#[cfg(unix)]
pub use scratch::publication_native_scratch_bytes;

use super::{AdmittedPath, AtomicPublication, FileError, private_open_options, publish_temporary};
use std::{error::Error, fmt, io, path::Path};

/// An original native error and its actual path, followed by its lifetime credit.
/// No owned error or path can escape separately from the supplied credit.
pub struct AdmittedFileError<Credit> {
    error: FileError,
    _credit: Credit,
    _context_credit: Option<Credit>,
    _wrapper_credit: Option<Credit>,
}
impl<Credit> AdmittedFileError<Credit> {
    /// Move a path admitted before the native operation into its original error.
    /// Separate allocations already owned by source remain the caller's liability.
    pub fn from_native(path: AdmittedPath<Credit>, source: io::Error) -> Self {
        let (path, credit) = path.into_parts();
        Self {
            error: FileError::Io { path, source },
            _credit: credit,
            _context_credit: None,
            _wrapper_credit: None,
        }
    }

    /// Retains original native diagnostic backing and wrapper bodies until the
    /// actual source physically drops. All credits precede those allocations.
    pub fn from_native_with_context(
        path: AdmittedPath<Credit>,
        source: io::Error,
        context_credit: Option<Credit>,
        wrapper_credit: Option<Credit>,
    ) -> Self {
        let (path, credit) = path.into_parts();
        Self {
            error: FileError::Io { path, source },
            _credit: credit,
            _context_credit: context_credit,
            _wrapper_credit: wrapper_credit,
        }
    }

    /// Construct the original policy io::Error only after its complete message
    /// backing and two actual std error boxes have been admitted by the caller.
    pub fn from_directory_policy(
        path: AdmittedPath<Credit>,
        reason: super::directory_policy::PrivateDirectoryReason,
        diagnostic_credit: Credit,
    ) -> Result<Self, super::PathAdmissionError> {
        let plan = super::directory_policy::DirectoryMessagePlan::new(path.as_path(), reason)?;
        let source = plan.allocate()?;
        Ok(Self::from_native_with_context(
            path,
            source,
            Some(diagnostic_credit),
            None,
        ))
    }

    pub fn native_source(&self) -> &io::Error {
        match &self.error {
            FileError::Io { source, .. } => source,
        }
    }
    pub fn kind(&self) -> io::ErrorKind {
        self.error.kind()
    }
    pub fn path(&self) -> &Path {
        match &self.error {
            FileError::Io { path, .. } => path,
        }
    }
}
impl<Credit> AsRef<FileError> for AdmittedFileError<Credit> {
    fn as_ref(&self) -> &FileError {
        &self.error
    }
}
impl<Credit> fmt::Debug for AdmittedFileError<Credit> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.error, formatter)
    }
}
impl<Credit> fmt::Display for AdmittedFileError<Credit> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
}
impl<Credit> Error for AdmittedFileError<Credit> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// A native stage failure retains its original path credit; producer errors
/// preserve their original concrete type and discard the partial stage.
#[derive(thiserror::Error)]
pub enum AdmittedStageError<E: Error + 'static, Credit> {
    #[error(transparent)]
    Native(AdmittedFileError<Credit>),
    #[error("snapshot producer failed: {0}")]
    Producer(
        #[source] E,
        AdmittedPath<Credit>,
        Option<DiagnosticCredits<Credit>>,
    ),
}

impl<E: Error + 'static, Credit> fmt::Debug for AdmittedStageError<E, Credit> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(error) => formatter.debug_tuple("Native").field(error).finish(),
            Self::Producer(error, _, _) => formatter.debug_tuple("Producer").field(error).finish(),
        }
    }
}

/// Original stage diagnostic-path backing and native wrapper allocation loans.
/// These tokens are consumed into an admitted error, with no raw error escape.
pub struct DiagnosticCredits<Credit> {
    context: Credit,
    wrappers: Credit,
}
impl<Credit> DiagnosticCredits<Credit> {
    pub fn into_parts(self) -> (Credit, Credit) {
        (self.context, self.wrappers)
    }
}

/// Original destination spelling and an operation-prepared exclusive stage path.
///
/// The caller selects an unpredictable leaf in the original operation's parent.
/// For relative parents it captures cwd per operation, preserving native joining
/// (including Windows drive-relative semantics). Creation always refuses an
/// existing file or symlink; no destination is opened during staging.
pub struct PreparedPrivateStage<DestinationCredit, StageCredit> {
    destination: AdmittedPath<DestinationCredit>,
    stage: AdmittedPath<StageCredit>,
    diagnostics: Option<DiagnosticCredits<StageCredit>>,
}
impl<Credit> PreparedPrivateStage<Credit, Credit> {
    pub fn new(destination: AdmittedPath<Credit>, stage: AdmittedPath<Credit>) -> Self {
        Self {
            destination,
            stage,
            diagnostics: None,
        }
    }

    /// Caller admits one original stage-path clone and the actual PathError/std
    /// custom bodies before either create or first producer failure can occur.
    pub fn with_error_credits(mut self, context: Credit, wrappers: Credit) -> Self {
        self.diagnostics = Some(DiagnosticCredits { context, wrappers });
        self
    }

    /// Streams into the existing AtomicPublication using the original tempfile
    /// writer, preserving its native PathError Display/source context.
    pub fn stage_with<E: Error + Send + Sync + 'static>(
        self,
        produce: impl FnOnce(&mut dyn io::Write) -> Result<(), E>,
    ) -> Result<AdmittedPublication<Credit, Credit>, AdmittedStageError<E, Credit>> {
        let Self {
            destination,
            stage,
            diagnostics,
        } = self;
        let options = private_open_options(true);
        #[cfg(windows)]
        let options = {
            let mut options = options;
            use std::os::windows::fs::OpenOptionsExt;
            // Same attribute as tempfile's original named private-file creation.
            const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
            options.custom_flags(FILE_ATTRIBUTE_TEMPORARY);
            options
        };
        let file = match options.open(stage.as_path()) {
            Ok(file) => file,
            Err(source) => {
                let (stage, context_credit) = stage.into_parts();
                let source = tempfile::error_with_owned_path(source, stage);
                let wrapper_credit = diagnostics.map(|credit| credit.wrappers);
                return Err(AdmittedStageError::Native(
                    AdmittedFileError::from_native_with_context(
                        destination,
                        source,
                        Some(context_credit),
                        wrapper_credit,
                    ),
                ));
            }
        };
        let (path, stage_credit) = stage.into_parts();
        // AdmittedPath always has exact encoded capacity: conversion does not
        // allocate/shrink. Keep its credit until the TempPath's actual drop.
        let path = tempfile::TempPath::from_prepared_path(path.into_boxed_path());
        let mut temporary = tempfile::NamedTempFile::from_parts(file, path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Original stage_private_with reapplies 0600 after exclusive creation,
            // so the final stage mode does not depend on the process umask.
            if let Err(source) = temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
            {
                drop(temporary);
                drop(stage_credit);
                return Err(native_error(destination, source));
            }
        }
        if let Err(error) = produce(&mut temporary) {
            drop(temporary);
            drop(stage_credit);
            return Err(AdmittedStageError::Producer(
                error,
                destination,
                diagnostics,
            ));
        }
        if let Err(source) = temporary.as_file().sync_all() {
            drop(temporary);
            drop(stage_credit);
            return Err(native_error(destination, source));
        }
        let (destination, destination_credit) = destination.into_parts();
        Ok(AdmittedPublication {
            publication: AtomicPublication {
                temporary,
                destination,
            },
            destination_credit,
            stage_credit,
        })
    }
}

fn native_error<E: Error + 'static, Credit>(
    destination: AdmittedPath<Credit>,
    source: io::Error,
) -> AdmittedStageError<E, Credit> {
    AdmittedStageError::Native(AdmittedFileError::from_native(destination, source))
}

/// The original AtomicPublication physically drops before both path credits.
#[must_use = "publish the admitted snapshot or drop it to discard its stage"]
pub struct AdmittedPublication<DestinationCredit, StageCredit> {
    publication: AtomicPublication,
    destination_credit: DestinationCredit,
    stage_credit: StageCredit,
}
impl<DestinationCredit, StageCredit> AdmittedPublication<DestinationCredit, StageCredit> {
    /// The same native persist and best-effort directory sync as AtomicPublication.
    /// Failure moves the original destination allocation into the concrete error.
    pub fn publish(self) -> Result<(), AdmittedFileError<DestinationCredit>> {
        let Self {
            publication,
            destination_credit,
            stage_credit,
        } = self;
        let AtomicPublication {
            temporary,
            destination,
        } = publication;
        let result = publish_temporary(temporary, &destination);
        drop(stage_credit);
        match result {
            Ok(()) => {
                drop(destination);
                drop(destination_credit);
                Ok(())
            }
            Err(source) => Err(AdmittedFileError {
                error: FileError::Io {
                    path: destination,
                    source,
                },
                _credit: destination_credit,
                _context_credit: None,
                _wrapper_credit: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests;
