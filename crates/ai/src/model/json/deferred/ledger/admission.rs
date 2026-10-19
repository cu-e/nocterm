//! Exact allocation admission failures contain no dynamically owned diagnosis.
use crate::model::retention::RetentionError;

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum NativeAllocationError {
    #[error("native allocation arithmetic overflow")]
    Arithmetic,
    #[error("native allocation requests {requested} bytes with {available} available")]
    Limit { requested: usize, available: usize },
    #[error("native document admission failed: {0}")]
    Retention(#[source] RetentionError),
    #[error("native allocation differs from the pinned capacity profile")]
    CapacityProfile,
}
impl From<RetentionError> for NativeAllocationError {
    fn from(error: RetentionError) -> Self {
        match error {
            RetentionError::Arithmetic => Self::Arithmetic,
            RetentionError::Limit {
                requested,
                available,
            } => Self::Limit {
                requested,
                available,
            },
            other => Self::Retention(other),
        }
    }
}
impl From<NativeAllocationError> for super::JsonDecodeError {
    fn from(error: NativeAllocationError) -> Self {
        match error {
            NativeAllocationError::Arithmetic => Self::Arithmetic,
            NativeAllocationError::Limit {
                requested,
                available,
            } => Self::Limit {
                requested,
                available,
            },
            NativeAllocationError::Retention(error) => Self::Retention(error),
            NativeAllocationError::CapacityProfile => Self::CapacityProfile,
        }
    }
}
