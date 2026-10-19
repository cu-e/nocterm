//! Delivered payload stays registered until installation or physical stale release.
use super::{ReaderCompletion, ReaderOutcome, ReaderSource, ReaderSourceError, RetainedRole};
use crate::history::physical::SnapshotOwner;

pub struct ReaderDelivery {
    completion: Option<ReaderCompletion>,
    source: ReaderSource,
}
impl ReaderDelivery {
    pub(super) fn new(completion: ReaderCompletion, source: ReaderSource) -> Self {
        Self {
            completion: Some(completion),
            source,
        }
    }
    /// Search scratch borrows and reactivates the SAME original Reader role;
    /// its query phase closes before delivery release or installation.
    pub fn mentions(&mut self, query: &str) -> Result<bool, crate::history::NativeLoadError> {
        self.completion
            .as_mut()
            .ok_or(crate::model::retention::RetentionError::FrozenOwner)?
            .mentions(query)
    }
    pub fn snapshot(&self) -> Option<&SnapshotOwner> {
        match self.completion.as_ref()?.outcome() {
            ReaderOutcome::Ready(parts) => parts.snapshot(),
            ReaderOutcome::Failed(_) => None,
        }
    }
    pub fn error(&self) -> Option<&super::NativeLoadError> {
        match self.completion.as_ref()?.outcome() {
            ReaderOutcome::Ready(_) => None,
            ReaderOutcome::Failed(failed) => Some(failed.error()),
        }
    }
    /// Success returns the SAME now-empty delivery to the original source.
    /// A foreign destination retains the complete delivery for retry or release.
    pub fn install_into(
        mut self,
        destination: &mut RetainedRole,
    ) -> Result<(), ReaderInstallError> {
        let (reader, outcome) = self
            .completion
            .take()
            .expect("original delivery")
            .into_parts();
        let parts = match outcome {
            ReaderOutcome::Ready(parts) => parts,
            ReaderOutcome::Failed(failed) => {
                self.completion = Some(ReaderCompletion::with_outcome(
                    reader,
                    ReaderOutcome::Failed(failed),
                ));
                return Err(ReaderInstallError {
                    delivery: self,
                    error: ReaderSourceError::Unavailable,
                });
            }
        };
        match parts.install_into(destination) {
            Ok(delivery) => {
                if let Err(error) = self.source.restore(reader, delivery) {
                    return Err(ReaderInstallError {
                        delivery: self,
                        error,
                    });
                }
                Ok(())
            }
            Err(error) => {
                let (parts, error) = error.into_parts();
                self.completion = Some(ReaderCompletion::with_outcome(
                    reader,
                    ReaderOutcome::Ready(parts),
                ));
                Err(ReaderInstallError {
                    delivery: self,
                    error: error.into(),
                })
            }
        }
    }
    pub fn release(mut self) -> Result<(), ReaderSourceError> {
        self.source
            .recycle(self.completion.take().expect("original delivery"))
    }
}
impl Drop for ReaderDelivery {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            let _ = self.source.recycle(completion);
        }
    }
}
#[derive(thiserror::Error)]
#[error("history delivery installation failed: {error}")]
pub struct ReaderInstallError {
    delivery: ReaderDelivery,
    #[source]
    error: ReaderSourceError,
}
impl ReaderInstallError {
    pub fn error(&self) -> &ReaderSourceError {
        &self.error
    }
    pub fn into_parts(self) -> (ReaderDelivery, ReaderSourceError) {
        (self.delivery, self.error)
    }
}
impl std::fmt::Debug for ReaderInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderInstallError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
