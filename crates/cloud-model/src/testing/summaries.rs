//! A non-cryptographic chat summary directory that records what a runner would publish.
use crate::{
    identity::ChatKeyEpoch,
    journal::{
        ChatActivity, ChatSummaryDirectory, ChatSummaryDraft, JournalError, JournalFuture,
        SealedChatSummary,
    },
    payload::OpaqueBytes,
};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

/// Seals each draft into distinct bytes (a seal counter is part of them, as the real box
/// is randomized), records drafts and publications, and fails the next publications or
/// reports the directory unavailable on request.
#[derive(Default)]
pub struct RecordedSummaryDirectory {
    unavailable: AtomicBool,
    failures: AtomicU32,
    seals: AtomicU32,
    drafts: Mutex<Vec<ChatSummaryDraft>>,
    published: Mutex<Vec<SealedChatSummary>>,
}
impl RecordedSummaryDirectory {
    pub fn set_available(&self, available: bool) {
        self.unavailable.store(!available, Ordering::SeqCst);
    }
    /// The next `count` publications fail as an unreachable hub would.
    pub fn fail_next(&self, count: u32) {
        self.failures.store(count, Ordering::SeqCst);
    }
    pub fn drafts(&self) -> Vec<ChatSummaryDraft> {
        self.drafts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    pub fn published(&self) -> Vec<SealedChatSummary> {
        self.published
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
impl ChatSummaryDirectory for RecordedSummaryDirectory {
    fn available(&self) -> bool {
        !self.unavailable.load(Ordering::SeqCst)
    }
    fn seal(&self, draft: ChatSummaryDraft) -> JournalFuture<SealedChatSummary> {
        let seal = self.seals.fetch_add(1, Ordering::SeqCst);
        let mut bytes = seal.to_be_bytes().to_vec();
        bytes.extend_from_slice(&draft.revision().to_be_bytes());
        bytes.extend_from_slice(draft.prompt_line().unwrap_or_default().as_bytes());
        let sealed = OpaqueBytes::new(bytes)
            .map_err(JournalError::from)
            .and_then(|bytes| {
                SealedChatSummary::new(
                    draft.stream().clone(),
                    ChatKeyEpoch::new(1)?,
                    draft.revision(),
                    draft.activity() == ChatActivity::Deleted,
                    bytes,
                )
            });
        self.drafts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(draft);
        Box::pin(async move { sealed })
    }
    fn publish(&self, sealed: SealedChatSummary) -> JournalFuture<()> {
        let failed = self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if !failed {
            self.published
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(sealed);
        }
        Box::pin(async move {
            if failed {
                Err(JournalError::Busy)
            } else {
                Ok(())
            }
        })
    }
}
