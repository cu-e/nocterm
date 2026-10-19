//! Immutable owned document values; live revisions belong to the runtime owner.
use super::{HistoryCodecError, HistoryOrigin, HistoryView, PreparedHistory, SavedAttachment};
use crate::{model::content::Content, session_config::Choices, thread::Entry};
use serde::{Deserialize, Serialize};
mod value;
use std::{path::PathBuf, sync::Arc};
pub use value::SavedDocument;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatMetadata {
    pub agent_id: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub pinned: bool,
    pub updated: u64,
    pub model: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResumeDescriptor {
    pub session_id: Option<String>,
    pub workdir: Option<PathBuf>,
    pub config: Choices,
    pub mode: Option<String>,
    pub pending_history: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TranscriptData {
    #[serde(deserialize_with = "super::decode::entries")]
    pub entries: Vec<Entry>,
    #[serde(default, deserialize_with = "super::decode::times")]
    pub times: Vec<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ComposerData {
    pub draft: Option<String>,
    #[serde(default, deserialize_with = "super::decode::prompts")]
    pub queue: Vec<SavedPrompt>,
    #[serde(default, deserialize_with = "super::decode::attachments")]
    pub attachments: Vec<SavedAttachment>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SavedPrompt {
    pub id: u64,
    pub text: String,
    #[serde(default, deserialize_with = "super::decode::images")]
    pub images: Vec<Content>,
    #[serde(default, deserialize_with = "super::decode::attachments")]
    pub attachments: Vec<SavedAttachment>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SavedChat {
    #[serde(skip)]
    retention: crate::model::retention::RetentionAuthority,
    id: String,
    metadata: ChatMetadata,
    resume: ResumeDescriptor,
    transcript: TranscriptData,
    composer: ComposerData,
    #[serde(skip)]
    prepared: Option<PreparedHistory>,
}
impl SavedChat {
    pub fn from_parts(
        retention: &crate::model::retention::RetentionAuthority,
        id: String,
        metadata: ChatMetadata,
        resume: ResumeDescriptor,
        transcript: TranscriptData,
        composer: ComposerData,
    ) -> Result<Self, HistoryCodecError> {
        let chat = Self {
            retention: retention.clone(),
            id,
            metadata,
            resume,
            transcript,
            composer,
            prepared: None,
        };
        // Named owned intake fixes physical ID affinity independently of later
        // admission success; a rejected budget never rewrites this provenance.
        for row in &chat.transcript.entries {
            if let Entry::Tool(tool) = row {
                tool.adopt_retention(retention).map_err(|error| {
                    HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
                })?;
            }
        }
        chat.view()?;
        Ok(chat)
    }
    /// A new document accepts already supplied identity/time, never allocates native facts.
    pub fn for_identity(
        retention: &crate::model::retention::RetentionAuthority,
        agent_id: String,
        id: String,
        updated: u64,
    ) -> Self {
        Self {
            retention: retention.clone(),
            id,
            metadata: ChatMetadata {
                agent_id,
                title: None,
                name: None,
                pinned: false,
                updated,
                model: None,
            },
            resume: ResumeDescriptor::default(),
            transcript: TranscriptData::default(),
            composer: ComposerData::default(),
            prepared: None,
        }
    }
    pub fn bounded_entries(entries: &[Entry]) -> Vec<Entry> {
        super::wire::clone_entries(entries)
    }
    pub fn bounded_times(entries_len: usize, times: &[u64]) -> Vec<u64> {
        times
            .iter()
            .skip(entries_len.saturating_sub(super::MAX_SAVED_ENTRIES))
            .copied()
            .collect()
    }
    pub fn saved_entry_start(entries_len: usize) -> usize {
        entries_len.saturating_sub(super::MAX_SAVED_ENTRIES)
    }
    pub(super) fn id_owned(&self) -> &String {
        &self.id
    }
    pub fn retention(&self) -> &crate::model::retention::RetentionAuthority {
        &self.retention
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn metadata(&self) -> &ChatMetadata {
        &self.metadata
    }
    pub fn resume(&self) -> &ResumeDescriptor {
        &self.resume
    }
    pub fn transcript(&self) -> &TranscriptData {
        &self.transcript
    }
    pub fn composer(&self) -> &ComposerData {
        &self.composer
    }
    pub fn prepared(&self) -> Option<&PreparedHistory> {
        self.prepared.as_ref()
    }
    /// Unique shared allocations plus actual exclusive reserve and retained
    /// cache/grammar owners. This is separate from the unchanged format policy.
    pub fn retained_owner_bytes(&self) -> Result<usize, crate::model::retention::RetentionError> {
        crate::model::retention::graph::measure(&self.retention, self)
    }
    pub fn resident_bytes(&self) -> usize {
        self.prepared
            .as_ref()
            .map_or(0, PreparedHistory::resident_bytes)
    }
    pub fn origin(&self) -> HistoryOrigin {
        self.prepared
            .as_ref()
            .map_or(HistoryOrigin::New, PreparedHistory::origin)
    }
    pub fn view(&self) -> Result<HistoryView<'_>, HistoryCodecError> {
        HistoryView::new(self, &self.composer.queue)
    }
    pub(crate) fn into_document(self) -> SavedDocument {
        SavedDocument {
            id: self.id,
            metadata: self.metadata,
            resume: self.resume,
            transcript: self.transcript,
            composer: self.composer,
        }
    }
    pub fn into_parts(
        self,
    ) -> (
        String,
        ChatMetadata,
        ResumeDescriptor,
        TranscriptData,
        ComposerData,
        Option<PreparedHistory>,
    ) {
        (
            self.id,
            self.metadata,
            self.resume,
            self.transcript,
            self.composer,
            self.prepared,
        )
    }
    /// A named content transition preserves the imported format context while
    /// invalidating/rebuilding its scalar decision for the actual new draft.
    pub fn with_archived_draft(
        mut self,
        patch: &ArchivedDraftPatch,
    ) -> Result<Self, HistoryCodecError> {
        if self.id != patch.id {
            return Err(HistoryCodecError::IdentityMismatch);
        }
        self.composer.draft = patch.text.clone();
        self.metadata.updated = patch.updated;
        super::codec::prepare(self)
    }
    /// Native history adapters derive inert description ranges without changing
    /// serialized values, prepared origin, revision or admission. These fixed
    /// inline facts never renew live execution authority.
    pub fn describe_imported_rows(
        mut self,
        describe: impl Fn(&crate::model::tools::ToolRecord) -> crate::model::literal::LiteralEvidence,
    ) -> Self {
        for row in &mut self.transcript.entries {
            if let crate::thread::Entry::Tool(call) = row {
                call.literal = describe(call);
                call.claims = Default::default();
            }
        }
        self
    }
    pub fn decision(&self) -> Option<super::EncodingDecision> {
        self.prepared.as_ref().and_then(PreparedHistory::decision)
    }
    pub(super) fn set_decision(
        &mut self,
        decision: super::EncodingDecision,
    ) -> Result<(), HistoryCodecError> {
        self.prepared
            .as_mut()
            .ok_or(HistoryCodecError::CandidateMismatch)?
            .set_decision(decision);
        Ok(())
    }
    pub(super) fn retain_at_least(&mut self, bytes: usize) {
        if let Some(prepared) = &mut self.prepared {
            prepared.retain_at_least(bytes);
        }
    }
    pub(super) fn attach_prepared(&mut self, prepared: PreparedHistory) {
        self.prepared = Some(prepared);
    }
}
/// Shared prompts remain complete; this owner offers no mutable or Deref projection.
#[derive(Clone, Debug)]
pub struct SharedChat {
    snapshot: SavedChat,
    prompts: Vec<Arc<SavedPrompt>>,
}
impl SharedChat {
    pub fn new(
        snapshot: SavedChat,
        prompts: Vec<Arc<SavedPrompt>>,
    ) -> Result<Self, HistoryCodecError> {
        if !snapshot.composer.queue.is_empty() {
            return Err(HistoryCodecError::CandidateMismatch);
        }
        let snapshot = super::codec::prepare(snapshot)?;
        let mut chat = Self { snapshot, prompts };
        chat.view()?;
        let decision = super::codec::decision_shared(&chat)?;
        chat.snapshot.set_decision(decision)?;
        Ok(chat)
    }
    pub fn from_saved(mut snapshot: SavedChat) -> Result<Self, HistoryCodecError> {
        let prompts = std::mem::take(&mut snapshot.composer.queue)
            .into_iter()
            .map(Arc::new)
            .collect();
        Self::new(snapshot, prompts)
    }
    pub fn decision(&self) -> Result<super::EncodingDecision, HistoryCodecError> {
        self.snapshot
            .decision()
            .ok_or(HistoryCodecError::CandidateMismatch)
    }
    pub fn snapshot(&self) -> &SavedChat {
        &self.snapshot
    }
    pub fn prompts(&self) -> &[Arc<SavedPrompt>] {
        &self.prompts
    }
    pub fn id(&self) -> &str {
        self.snapshot.id()
    }
    pub fn view(&self) -> Result<HistoryView<'_>, HistoryCodecError> {
        HistoryView::new(&self.snapshot, &self.prompts)
    }
}
/// Explicit intent for an archived draft; no empty transcript stands in for unloaded data.
#[derive(Clone, Debug)]
pub struct ArchivedDraftPatch {
    pub id: String,
    pub agent_id: String,
    pub text: Option<String>,
    pub updated: u64,
}
#[derive(Clone, Debug)]
pub enum ChatWrite {
    Snapshot(Box<SharedChat>),
    ArchiveDraft(ArchivedDraftPatch),
}
impl ChatWrite {
    pub fn id(&self) -> &str {
        match self {
            Self::Snapshot(chat) => chat.id(),
            Self::ArchiveDraft(patch) => &patch.id,
        }
    }
    pub fn snapshot(&self) -> Option<&SharedChat> {
        match self {
            Self::Snapshot(chat) => Some(chat),
            Self::ArchiveDraft(_) => None,
        }
    }
}

use crate::model::decode::{field, missing, seeded_struct, unknown};
seeded_struct!(TranscriptData{entries:Vec<Entry> => bound2000 required,times:Vec<u64> => bound2000 default});
seeded_struct!(ComposerData{draft:Option<String> => plain optional,queue:Vec<SavedPrompt> => bound1024 default,attachments:Vec<SavedAttachment> => bound1024 default});
seeded_struct!(SavedPrompt{id:u64 => plain required,text:String => plain required,images:Vec<Content> => bound128 default,attachments:Vec<SavedAttachment> => bound1024 default});
seeded_struct!(SavedChat{retention:crate::model::retention::RetentionAuthority => context context,id:String => plain required,metadata:ChatMetadata => plain required,resume:ResumeDescriptor => plain required,transcript:TranscriptData => owned required,composer:ComposerData => owned required}; ignored{prepared});

impl crate::model::retention::graph::sealed::Source for SavedChat {}
impl crate::model::retention::graph::OwnerGraph for SavedChat {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        super::allocations::count_chat(self, count)?;
        if let Some(prepared) = &self.prepared {
            prepared.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        super::allocations::cleanup_chat(self, permit);
        if let Some(prepared) = &self.prepared {
            prepared.cleanup(permit);
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        super::allocations::adopt_chat(self, publication);
        if let Some(prepared) = &self.prepared {
            prepared.adopt(publication);
        }
    }
}
