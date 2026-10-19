//! A fork is an independent document sharing the original rows it keeps.
use super::{ThreadState, ThreadStateError};
use crate::{
    history::live::{ForkFacts, LiveFork},
    model::retention::RetentionAuthority,
};

pub struct ForkStaging<'a> {
    source: &'a ThreadState,
    len: usize,
}
impl<'a> ForkStaging<'a> {
    /// Keep the first `len` rows of `source`; a longer length keeps them all.
    pub fn new(source: &'a ThreadState, len: usize) -> Self {
        Self { source, len }
    }
    pub fn finish(
        self,
        authority: &RetentionAuthority,
        id: &str,
        name: &str,
        pending_history: bool,
        now: u64,
    ) -> Result<ThreadState, ThreadStateError> {
        let agent = self
            .source
            .live()
            .role()
            .snapshot()
            .map(|snapshot| snapshot.metadata().agent_id().to_string())
            .ok_or(ThreadStateError::MissingRole)?;
        let mut target = ThreadState::new(authority, &agent, id, now)?;
        LiveFork::new(target.live_mut(), self.source.live()).adopt(ForkFacts {
            id,
            name,
            len: self.len,
            pending_history,
            now,
        })?;
        Ok(target)
    }
}
