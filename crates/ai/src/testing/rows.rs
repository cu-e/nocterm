//! Full-live row fixtures use genuine physical ownership without persisted suffix truncation.
use crate::{
    history::{
        self,
        physical::{PhysicalError, RowSlot, SnapshotBuilder},
    },
    model::{
        json::deferred::DecodeLedger,
        retention::{
            RetentionAuthority, RetentionError,
            document::{ConstructionRole, DocumentRetention, RetainedRole},
        },
    },
    thread::Entry,
};

pub struct ManagedRows {
    authority: RetentionAuthority,
    live: RetainedRole,
    construction: ConstructionRole,
}
#[derive(Debug, thiserror::Error)]
pub enum ManagedRowsError {
    #[error(transparent)]
    History(#[from] history::HistoryCodecError),
    #[error(transparent)]
    Physical(#[from] PhysicalError),
    #[error(transparent)]
    Retention(#[from] RetentionError),
    #[error(transparent)]
    Decode(#[from] crate::model::json::deferred::JsonDecodeError),
    #[error("fixture document is missing its original role")]
    MissingRole,
}
impl ManagedRows {
    pub fn new(rows: Vec<Entry>) -> Result<Self, ManagedRowsError> {
        let authority = RetentionAuthority::new(usize::MAX)?;
        Self::with_authority(&authority, rows)
    }
    pub fn with_authority(
        authority: &RetentionAuthority,
        rows: Vec<Entry>,
    ) -> Result<Self, ManagedRowsError> {
        let (_, mut roles) = DocumentRetention::new(authority)?;
        let mut fixture = Self {
            authority: authority.clone(),
            live: roles.take_live().ok_or(ManagedRowsError::MissingRole)?,
            construction: roles
                .take_foreground()
                .ok_or(ManagedRowsError::MissingRole)?,
        };
        fixture.replace(rows)?;
        Ok(fixture)
    }
    pub fn replace(&mut self, rows: Vec<Entry>) -> Result<(), ManagedRowsError> {
        self.construction.begin_phase()?;
        let result = (|| {
            let ledger =
                DecodeLedger::for_operation(&self.construction, history::MAX_RESIDENT_BYTES)?;
            // Input values are explicit test fixtures, not native preallocation evidence.
            let chat = history::SavedChat::from_parts(
                &self.authority,
                "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c".into(),
                history::ChatMetadata {
                    agent_id: "fixture".into(),
                    title: None,
                    name: None,
                    pinned: false,
                    updated: 0,
                    model: None,
                },
                history::ResumeDescriptor::default(),
                history::TranscriptData {
                    entries: rows,
                    times: Vec::new(),
                },
                history::ComposerData::default(),
            )?;
            let document =
                history::native::NativeChat::from_fixture(chat, &ledger)?.into_document();
            let staging =
                SnapshotBuilder::from_document(&self.construction, document)?.finish(None)?;
            self.live.replace(staging)?;
            Ok(())
        })();
        self.construction.clear_source();
        result
    }
    pub fn rows(&self) -> &[RowSlot] {
        self.live.snapshot().map_or(&[], |snapshot| snapshot.rows())
    }
}
