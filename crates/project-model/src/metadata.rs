//! Supplied native identity and time facts; reading a definition allocates neither.

use crate::{ProjectId, ProjectMetadataError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectIdentity {
    pub id: ProjectId,
    pub created: u64,
}

/// Request identity once for a genuinely new project; accepted edits request time.
pub trait ProjectMetadataSource: Send + Sync + 'static {
    fn new_identity(&self) -> Result<ProjectIdentity, ProjectMetadataError>;
    fn timestamp(&self) -> u64;
}
