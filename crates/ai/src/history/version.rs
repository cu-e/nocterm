//! Transcript format facts are independent of the rebuildable cache schema.
use super::{ContextId, HistoryCodecError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct TranscriptVersion(u32);
impl TranscriptVersion {
    pub const LEGACY: Self = Self(super::LEGACY_VERSION);
    pub const SCOPED: Self = Self(super::VERSION);
    pub const fn wire(self) -> u32 {
        self.0
    }
    pub(super) fn observed(version: u32) -> Self {
        Self(version)
    }
    pub(super) fn require_scope(self, scope: ContextId) -> Result<(), HistoryCodecError> {
        super::header::resolve_scope(self.0, Some(scope)).map(|_| ())
    }
}
impl TryFrom<u32> for TranscriptVersion {
    type Error = HistoryCodecError;
    fn try_from(version: u32) -> Result<Self, Self::Error> {
        match version {
            super::LEGACY_VERSION | super::VERSION => Ok(Self(version)),
            other => Err(HistoryCodecError::UnsupportedVersion(other)),
        }
    }
}
impl From<TranscriptVersion> for u32 {
    fn from(version: TranscriptVersion) -> Self {
        version.wire()
    }
}
