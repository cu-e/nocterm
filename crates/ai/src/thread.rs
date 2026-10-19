//! Pure owned transcript reduction; all timestamps are supplied by the host.
use crate::model::{content::Content, notice::PlanStep, tools::ToolRecord};
use serde::Serialize;
mod fork;
pub use fork::ForkStaging;
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "history accounting charges size_of::<Entry>() per row; boxing would hide tool records from it"
)]
pub enum Entry {
    User(Vec<Content>),
    Agent(String),
    Thought(String),
    Content(Content),
    Tool(ToolRecord),
}
pub struct ThreadState {
    live: crate::history::live::LiveOwner,
    reader: crate::history::ReaderSource,
    writer: crate::history::writing::WriterProducer,
    source: crate::history::source::SourceProducer,
    candidate: crate::model::retention::document::RetainedRole,
}
#[derive(Debug, thiserror::Error)]
pub enum ThreadStateError {
    #[error(transparent)]
    Live(#[from] crate::history::live::LiveError),
    #[error(transparent)]
    Retention(#[from] crate::model::retention::RetentionError),
    #[error(transparent)]
    Reader(#[from] crate::history::ReaderSourceError),
    #[error(transparent)]
    Writer(#[from] crate::history::writing::WriterCapabilityError),
    #[error(transparent)]
    Source(#[from] crate::history::source::SourceError),
    #[error("original document capability was already transferred")]
    MissingRole,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadChange {
    Transcript,
    Controls,
    Metadata,
    Ignored,
}
impl ThreadState {
    /// Prepare the original physical document before allocating a visible entity.
    pub fn new(
        authority: &crate::model::retention::RetentionAuthority,
        agent: &str,
        id: &str,
        updated: u64,
    ) -> Result<Self, ThreadStateError> {
        let (_, mut roles) = crate::model::retention::document::DocumentRetention::new(authority)?;
        let mut live = crate::history::live::LiveOwner::new(
            roles.take_live().ok_or(ThreadStateError::MissingRole)?,
            roles
                .take_foreground()
                .ok_or(ThreadStateError::MissingRole)?,
            roles
                .take_prompt_dispatch()
                .ok_or(ThreadStateError::MissingRole)?,
        )?;
        live.initialize(agent, id, updated)?;
        let reader = crate::history::ReaderSource::new(
            roles.take_reader().ok_or(ThreadStateError::MissingRole)?,
            roles.take_delivery().ok_or(ThreadStateError::MissingRole)?,
        )?;
        let writer = crate::history::writing::WriterProducer::new(
            roles
                .take_writer_latest()
                .ok_or(ThreadStateError::MissingRole)?,
            roles
                .take_writer_active()
                .ok_or(ThreadStateError::MissingRole)?,
            roles.take_writer().ok_or(ThreadStateError::MissingRole)?,
        )?;
        let source = crate::history::source::SourceProducer::new(
            roles.take_source().ok_or(ThreadStateError::MissingRole)?,
        )?;
        Ok(Self {
            live,
            reader,
            writer,
            source,
            candidate: roles
                .take_candidate()
                .ok_or(ThreadStateError::MissingRole)?,
        })
    }
    pub fn live(&self) -> &crate::history::live::LiveOwner {
        &self.live
    }
    pub fn live_mut(&mut self) -> &mut crate::history::live::LiveOwner {
        &mut self.live
    }
    pub fn rows(&self) -> &[crate::history::physical::RowSlot] {
        self.live.rows()
    }
    pub fn times(&self) -> &[u64] {
        self.live.times()
    }
    pub fn time(&self, index: usize) -> Option<u64> {
        self.times().get(index).copied().filter(|value| *value > 0)
    }
    pub fn mentions(&self, query: &str) -> Result<bool, crate::history::live::LiveError> {
        self.live.mentions(query)
    }
    pub fn title(&self) -> Option<crate::history::physical::TextGuard<'_>> {
        self.live
            .role()
            .snapshot()
            .and_then(|owner| owner.metadata().title())
    }
    pub fn usage(
        &self,
    ) -> Option<crate::history::physical::PayloadGuard<'_, crate::model::usage::ContextUsage>> {
        self.live.usage().and_then(|usage| usage.context())
    }
    pub fn tokens(&self) -> Option<&crate::model::usage::TokenTotals> {
        self.live.usage().and_then(|usage| usage.tokens())
    }
    pub fn limits(&self) -> impl Iterator<Item = &crate::history::physical::RateLimitSlot> {
        self.live
            .usage()
            .into_iter()
            .flat_map(|usage| usage.limits())
    }
    pub fn plan(&self) -> Option<crate::history::physical::PayloadGuard<'_, Vec<PlanStep>>> {
        self.live.plan()
    }
    pub fn reader(&self) -> &crate::history::ReaderSource {
        &self.reader
    }
    pub fn source(&self) -> &crate::history::source::SourceProducer {
        &self.source
    }
    pub fn prepare_candidate(&mut self) -> Result<(), ThreadStateError> {
        self.live
            .prepare_saved_candidate(self.live.local_admission(), &mut self.candidate)?;
        Ok(())
    }
    pub fn candidate(
        &self,
        intent: crate::history::writing::ManagedWriteIntent,
    ) -> Result<crate::history::writing::CandidateOffer<'_>, ThreadStateError> {
        Ok(self.writer.candidate(&self.candidate, intent)?)
    }
}
