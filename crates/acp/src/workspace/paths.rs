//! Original metadata/realpath observations precede their exact admitted copies.
use super::managed::Cause;
use crate::managed::SourceBudget;
use nocterm_ai::{
    model::{
        json::deferred::{NativeAllocationError, NativeScratch},
        native::{NativeList, NativeOwned},
    },
    sandbox::{PathKind, ResolvedPath, ResolvedSandboxPaths, SandboxPolicy},
    workspace::WorkdirError,
};
use nocterm_core::fs::{
    AdmittedFileError, AdmittedPath, DirectoryMessagePlan, PrivateDirectoryCheckError,
};
use std::{
    io,
    path::{Path, PathBuf},
};
pub(crate) struct Paths {
    budget: SourceBudget,
}
#[expect(
    clippy::result_large_err,
    reason = "every path step returns ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
impl Paths {
    pub(crate) fn new(budget: SourceBudget) -> Self {
        Self { budget }
    }
    fn diagnostic_credit(&self, path: &Path) -> Result<NativeScratch, Cause> {
        Ok(NativeScratch::reserve_native(
            self.budget.ledger(),
            AdmittedPath::<NativeScratch>::copy_capacity(path),
        )?)
    }
    fn diagnostic(
        &self,
        path: &Path,
        credit: NativeScratch,
        error: io::Error,
    ) -> Result<AdmittedFileError<NativeScratch>, Cause> {
        Ok(AdmittedFileError::from_native(
            AdmittedPath::try_copy(path, credit)?,
            error,
        ))
    }
    /// Pinned std1.98 run_path_with_cstr uses stack384, else one len+1 CString.
    fn std_scratch(&self, path: &Path) -> Result<NativeScratch, Cause> {
        let len = path.as_os_str().as_encoded_bytes().len();
        let bytes = if len < 384 {
            0
        } else {
            len.checked_add(1)
                .ok_or(NativeAllocationError::Arithmetic)?
        };
        Ok(NativeScratch::reserve_native(self.budget.ledger(), bytes)?)
    }
    pub(super) fn private(&self, path: &Path) -> Result<(), Cause> {
        let diagnostic = self.diagnostic_credit(path)?;
        let scratch = self.std_scratch(path)?;
        let result = nocterm_core::paths::ensure_private_dir_checked(path);
        drop(scratch);
        match result {
            Ok(()) => Ok(()),
            Err(PrivateDirectoryCheckError::Native(error)) => {
                Err(Cause::Create(self.diagnostic(path, diagnostic, error)?))
            }
            Err(PrivateDirectoryCheckError::Policy(reason)) => {
                let path = AdmittedPath::try_copy(path, diagnostic)?;
                let plan = DirectoryMessagePlan::new(path.as_path(), reason)?;
                let credit = NativeScratch::reserve_native(
                    self.budget.ledger(),
                    plan.requested_error_bytes()?,
                )?;
                Err(Cause::Create(AdmittedFileError::from_directory_policy(
                    path, reason, credit,
                )?))
            }
        }
    }
    pub(super) fn required(&self, path: &Path) -> Result<NativeOwned<ResolvedPath>, Cause> {
        if !path.is_absolute() {
            return Err(WorkdirError::InvalidDirectory.into());
        }
        let value = self.optional(path)?;
        if value.get().kind != PathKind::Directory {
            return Err(WorkdirError::InvalidDirectory.into());
        }
        Ok(value)
    }
    fn optional(&self, path: &Path) -> Result<NativeOwned<ResolvedPath>, Cause> {
        let diagnostic = self.diagnostic_credit(path)?;
        let scratch = self.std_scratch(path)?;
        let metadata = std::fs::metadata(path);
        drop(scratch);
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(nocterm_ai::model::native::workspace::resolved_path(
                    self.budget.context().copy_path(path)?,
                    self.budget.context().copy_path(path)?,
                    PathKind::Absent,
                )?);
            }
            Err(error) => {
                return Err(Cause::Resolve(self.diagnostic(path, diagnostic, error)?));
            }
        };
        let resolved = match self.canonical(path) {
            Ok(path) => path,
            Err(CanonicalError::Native(error)) => {
                return Err(Cause::Resolve(self.diagnostic(path, diagnostic, error)?));
            }
            Err(CanonicalError::Cause(cause)) => return Err(cause),
        };
        Ok(nocterm_ai::model::native::workspace::resolved_path(
            self.budget.context().copy_path(path)?,
            resolved,
            if metadata.is_dir() {
                PathKind::Directory
            } else {
                PathKind::NonDirectory
            },
        )?)
    }
    pub(crate) fn resolve(
        &self,
        policy: &SandboxPolicy,
        workdir: &Path,
    ) -> Result<NativeOwned<ResolvedSandboxPaths>, Cause> {
        Ok(nocterm_ai::model::native::workspace::resolved_sandbox(
            self.required(workdir)?,
            self.list(&policy.credentials)?,
            self.list(&policy.writable)?,
            self.list(&policy.hidden)?,
            self.list(&policy.readable)?,
        )?)
    }
    pub(crate) fn existing_file(&self, path: &Path) -> Result<bool, Cause> {
        let diagnostic = self.diagnostic_credit(path)?;
        let scratch = self.std_scratch(path)?;
        let result = std::fs::metadata(path);
        drop(scratch);
        match result {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(Cause::Resolve(self.diagnostic(path, diagnostic, error)?)),
        }
    }
    pub(crate) fn executable(&self, path: &Path) -> Result<Option<NativeOwned<PathBuf>>, Cause> {
        if !self.is_executable(path)? {
            return Ok(None);
        }
        match self.canonical(path) {
            Ok(path) => Ok(Some(path)),
            Err(CanonicalError::Native(_)) => Ok(None),
            Err(CanonicalError::Cause(error)) => Err(error),
        }
    }
    pub(crate) fn is_executable(&self, path: &Path) -> Result<bool, Cause> {
        let scratch = self.std_scratch(path)?;
        let executable = crate::process::executable(path);
        drop(scratch);
        Ok(executable)
    }
    fn list(&self, paths: &[PathBuf]) -> Result<NativeOwned<Vec<ResolvedPath>>, Cause> {
        let mut result = NativeList::new(self.budget.ledger(), paths.len())?;
        for path in paths {
            result.push(self.optional(path)?)?;
        }
        Ok(result.finish())
    }
    #[cfg(not(unix))]
    fn canonical(&self, _: &Path) -> Result<NativeOwned<PathBuf>, CanonicalError> {
        Err(CanonicalError::Cause(Cause::Platform))
    }
    #[cfg(unix)]
    fn canonical(&self, path: &Path) -> Result<NativeOwned<PathBuf>, CanonicalError> {
        use std::{ffi::CStr, os::unix::ffi::OsStrExt};
        let input = Input::new(path.as_os_str().as_bytes(), self.budget.clone())
            .map_err(CanonicalError::Cause)?;
        let input = CStr::from_bytes_with_nul(input.bytes())
            .map_err(|_| CanonicalError::Cause(Cause::Input))?;
        let native = nix::unistd::realpath_owned(input)
            .map_err(|error| CanonicalError::Native(io::Error::from_raw_os_error(error as i32)))?;
        let path = Path::new(std::ffi::OsStr::from_bytes(native.as_c_str().to_bytes()));
        self.budget
            .context()
            .copy_path(path)
            .map_err(|error| CanonicalError::Cause(error.into()))
    }
}
#[expect(
    clippy::large_enum_variant,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
enum CanonicalError {
    Native(io::Error),
    Cause(Cause),
}
#[cfg(unix)]
#[expect(
    clippy::large_enum_variant,
    reason = "short paths use an inline stack buffer instead of a heap allocation"
)]
enum Input {
    Stack { bytes: [u8; 384], len: usize },
    Heap(agent_client_protocol::managed::Buffer<u8, SourceBudget>),
}
#[cfg(unix)]
impl Input {
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    fn new(input: &[u8], budget: SourceBudget) -> Result<Self, Cause> {
        let len = input
            .len()
            .checked_add(1)
            .ok_or(NativeAllocationError::Arithmetic)?;
        if input.len() < 384 {
            let mut bytes = [0; 384];
            bytes[..input.len()].copy_from_slice(input);
            return Ok(Self::Stack { bytes, len });
        }
        let mut bytes = agent_client_protocol::managed::Buffer::new(budget);
        bytes.reserve(len)?;
        bytes.extend_bytes(input)?;
        bytes.push(0).map_err(|(error, _)| error)?;
        Ok(Self::Heap(bytes))
    }
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Stack { bytes, len } => &bytes[..*len],
            Self::Heap(bytes) => bytes.as_slice(),
        }
    }
}
