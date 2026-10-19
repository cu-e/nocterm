//! Pure legacy measurement capability; grammar and native repository stay private in storage.
use super::{
    FormatSize, HistoryCodecError, HistoryFraming, HistoryOrigin, HistoryShape, HistoryView,
    PreparedBudget, SavedPrompt,
};
use crate::thread::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatibilityReason {
    ContentKind,
    ContentField,
    ToolKind,
    ToolField,
    ConfigurationKind,
    ConfigurationField,
    ExtensionField,
}
#[derive(Debug, thiserror::Error)]
pub enum LegacySizingError {
    #[error("Legacy formatted key measurement failed: {0}")]
    Formatting(#[from] std::fmt::Error),
    #[error("retained legacy ownership failed: {0}")]
    Retention(#[from] crate::model::retention::RetentionError),
    #[error("Legacy history cannot represent {0:?}")]
    UnsupportedRecord(CompatibilityReason),
    #[error("Legacy JSON measurement failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Legacy JSON measurement arithmetic overflow")]
    ArithmeticOverflow,
}
pub trait LegacySizing: std::any::Any + Send + Sync {
    fn retention(&self) -> Option<&super::retention::LegacyIdentity> {
        None
    }
    fn retained_heap_bytes(&self) -> Result<usize, LegacySizingError>;
    fn entry(&self, row: &Entry) -> Result<FormatSize, LegacySizingError>;
    fn prompt(&self, prompt: &SavedPrompt) -> Result<FormatSize, LegacySizingError>;
    fn framing(
        &self,
        frame: HistoryFraming<'_>,
        shape: HistoryShape,
    ) -> Result<FormatSize, LegacySizingError>;
    fn measure(&self, candidate: HistoryView<'_>) -> Result<FormatSize, LegacySizingError>;
    fn managed_entry(
        &self,
        row: &super::physical::RowSlot,
    ) -> Result<FormatSize, LegacySizingError>;
    fn managed_prompt(
        &self,
        prompt: &super::physical::PromptSlot,
    ) -> Result<FormatSize, LegacySizingError>;
    fn managed_measure(
        &self,
        snapshot: &super::physical::SnapshotOwner,
    ) -> Result<FormatSize, LegacySizingError>;
}
#[derive(Clone)]
pub struct PreparedHistory {
    scalars: PreparedBudget,
    origin: HistoryOrigin,
    legacy: Option<super::retention::LegacyFormat>,
    retention: Option<super::retention::LegacyIdentity>,
}
impl std::fmt::Debug for PreparedHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedHistory")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl PreparedHistory {
    pub fn new(
        scalars: PreparedBudget,
        origin: HistoryOrigin,
        legacy: Option<super::retention::LegacyFormat>,
    ) -> Result<Self, HistoryCodecError> {
        if (origin == HistoryOrigin::LegacyV1) != legacy.is_some() {
            return Err(HistoryCodecError::CandidateMismatch);
        }
        let retention = legacy
            .as_ref()
            .and_then(|grammar| grammar.retention())
            .cloned();
        Ok(Self {
            retention,
            scalars,
            origin,
            legacy,
        })
    }
    /// Actual cache slabs contain scalar facts and weak authentic child receipts;
    /// this projection pins metadata, without retaining the format payload owner.
    pub fn retention_projection(&self) -> PreparedProjection {
        PreparedProjection {
            scalars: self.scalars.clone(),
            legacy: self.retention.clone(),
        }
    }
    pub fn origin(&self) -> HistoryOrigin {
        self.origin
    }
    pub fn legacy(&self) -> Option<&super::retention::LegacyFormat> {
        self.legacy.as_ref()
    }
    pub fn scalars(&self) -> &PreparedBudget {
        &self.scalars
    }
    pub fn resident_bytes(&self) -> usize {
        self.scalars.resident_bytes
    }
    pub fn decision(&self) -> Option<super::EncodingDecision> {
        self.scalars.decision
    }
    pub(super) fn retain_at_least(&mut self, bytes: usize) {
        self.scalars.resident_bytes = self.scalars.resident_bytes.max(bytes);
    }
    pub(super) fn set_decision(&mut self, decision: super::EncodingDecision) {
        self.scalars.decision = Some(decision);
    }
}

impl crate::model::retention::graph::sealed::Source for PreparedHistory {}
impl crate::model::retention::graph::OwnerGraph for PreparedHistory {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        self.scalars.count(count)?;
        if self.legacy.is_some() {
            self.retention
                .as_ref()
                .ok_or(crate::model::retention::RetentionError::MissingAuthority)?
                .count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.scalars.cleanup(permit);
        if let Some(retention) = &self.retention {
            retention.cleanup(permit);
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.scalars.adopt(publication);
        if let Some(retention) = &self.retention {
            retention.adopt(publication);
        }
    }
}

/// Metadata-only topology of the actual immutable prepared caches and grammar.
#[derive(Clone)]
pub struct PreparedProjection {
    scalars: PreparedBudget,
    legacy: Option<super::retention::LegacyIdentity>,
}
impl crate::model::retention::graph::sealed::Source for PreparedProjection {}
impl crate::model::retention::graph::OwnerGraph for PreparedProjection {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        self.scalars.count(count)?;
        if let Some(legacy) = &self.legacy {
            legacy.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.scalars.cleanup(permit);
        if let Some(legacy) = &self.legacy {
            legacy.cleanup(permit);
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.scalars.adopt(publication);
        if let Some(legacy) = &self.legacy {
            legacy.adopt(publication);
        }
    }
}
