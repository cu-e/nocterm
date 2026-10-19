//! Consuming history handoff returns the original now-empty delivery capability.
use super::SnapshotOwner;
use crate::model::retention::{RetentionError, document::RetainedRole};

/// Original role registration travels with its payload until installation.
pub struct ManagedParts {
    role: RetainedRole,
}
impl ManagedParts {
    pub fn from_role(role: RetainedRole) -> Self {
        Self { role }
    }
    pub fn snapshot(&self) -> Option<&SnapshotOwner> {
        self.role.snapshot()
    }
    /// Discard a stale delivery physically before returning its original empty capability.
    pub fn release(mut self) -> RetainedRole {
        self.role.clear();
        self.role
    }
    pub fn install_into(
        mut self,
        destination: &mut RetainedRole,
    ) -> Result<RetainedRole, ManagedInstallError> {
        if let Err(error) = destination.transfer_from(&mut self.role) {
            return Err(ManagedInstallError { parts: self, error });
        }
        Ok(self.role)
    }
}

/// Failed installation preserves both the actual source guard and typed cause.
#[derive(thiserror::Error)]
#[error("managed history installation failed: {error}")]
pub struct ManagedInstallError {
    parts: ManagedParts,
    #[source]
    error: RetentionError,
}
impl ManagedInstallError {
    pub fn error(&self) -> &RetentionError {
        &self.error
    }
    pub fn into_parts(self) -> (ManagedParts, RetentionError) {
        (self.parts, self.error)
    }
}
impl std::fmt::Debug for ManagedInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedInstallError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
