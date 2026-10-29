//! Account/worker-bound native catalog encryption; independent of any chat key.
use super::JournalError;
use crate::{
    authority::StreamBinding,
    identity::{AccountScope, RunnerId, RunnerKeyEpoch},
};

pub const MAX_JOURNAL_CATALOG_CHATS: usize = 100_000;
pub const MAX_JOURNAL_CATALOG_PLAINTEXT: usize = 2 * 1024 * 1024;
pub const MAX_JOURNAL_CATALOG_CIPHERTEXT: usize = MAX_JOURNAL_CATALOG_PLAINTEXT + 64 * 1024;

/// Known account and worker supplied by trusted composition, never discovered
/// from a database selector or inferred from an address/certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCatalogBinding {
    scope: AccountScope,
    runner: RunnerId,
    epoch: RunnerKeyEpoch,
}
impl JournalCatalogBinding {
    pub fn new(scope: AccountScope, runner: RunnerId, epoch: RunnerKeyEpoch) -> Self {
        Self {
            scope,
            runner,
            epoch,
        }
    }
    pub fn scope(&self) -> &AccountScope {
        &self.scope
    }
    pub fn runner(&self) -> RunnerId {
        self.runner
    }
    pub fn epoch(&self) -> RunnerKeyEpoch {
        self.epoch
    }
    pub fn stream(&self, chat: crate::identity::CloudChatId) -> StreamBinding {
        StreamBinding::new(self.scope.clone(), self.runner, self.epoch, chat)
    }
    pub fn contains(&self, stream: &StreamBinding) -> bool {
        stream.scope() == &self.scope
            && stream.runner() == self.runner
            && stream.runner_epoch() == self.epoch
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCatalogContext {
    binding: JournalCatalogBinding,
    revision: u64,
}
impl JournalCatalogContext {
    pub fn new(binding: JournalCatalogBinding, revision: u64) -> Result<Self, JournalError> {
        if revision == 0 {
            return Err(JournalError::Corrupt);
        }
        Ok(Self { binding, revision })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Purpose-specific account/runner catalog crypto. No chat key, plaintext
/// production default, remote I/O, or implicit account/key discovery is allowed.
/// Implementations bind the entire context and enforce the hard input/output
/// limits before large allocations. Before mapping to contract coordinates,
/// implementations compare the full native binding, including the account public
/// key fingerprint, with their configured verified account/runner pins. A native
/// field absent from the public coordinate must not lose this expected-pin check.
/// Constructor values prove neither account/key ownership, crypto nor fsync.
pub trait JournalCatalogCipher: Send + Sync {
    fn seal(
        &self,
        context: &JournalCatalogContext,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, JournalError>;
    fn open(&self, context: &JournalCatalogContext, sealed: &[u8])
    -> Result<Vec<u8>, JournalError>;
}

pub const MAX_JOURNAL_CATALOG_PAGE_CHATS: usize = 256;

/// A cursor is tied to one verified account/worker catalog revision. It is a
/// descriptive position, not a proof of authentication or a source capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCatalogCursor {
    binding: JournalCatalogBinding,
    revision: u64,
    after: crate::identity::CloudChatId,
}
impl JournalCatalogCursor {
    pub fn new(
        binding: JournalCatalogBinding,
        revision: u64,
        after: crate::identity::CloudChatId,
    ) -> Result<Self, JournalError> {
        JournalCatalogContext::new(binding.clone(), revision)?;
        Ok(Self {
            binding,
            revision,
            after,
        })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn after(&self) -> crate::identity::CloudChatId {
        self.after
    }
}
#[derive(Clone, Debug)]
pub struct JournalCatalogPage {
    binding: JournalCatalogBinding,
    revision: u64,
    chats: Vec<crate::identity::CloudChatId>,
    more: bool,
}
impl JournalCatalogPage {
    pub fn new(
        binding: JournalCatalogBinding,
        revision: u64,
        chats: Vec<crate::identity::CloudChatId>,
        more: bool,
    ) -> Result<Self, JournalError> {
        JournalCatalogContext::new(binding.clone(), revision)?;
        if chats.len() > MAX_JOURNAL_CATALOG_PAGE_CHATS
            || (more && chats.is_empty())
            || chats.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            binding,
            revision,
            chats,
            more,
        })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn chats(&self) -> &[crate::identity::CloudChatId] {
        &self.chats
    }
    pub fn next_cursor(&self) -> Option<JournalCatalogCursor> {
        self.more.then(|| JournalCatalogCursor {
            binding: self.binding.clone(),
            revision: self.revision,
            after: self.chats[self.chats.len() - 1],
        })
    }
}
pub trait JournalCatalogReader: Send + Sync {
    /// A changed catalog revision is Conflict, requiring an explicit pagination
    /// restart. Never silently skip a newly inserted chat before an old cursor.
    fn catalog(
        &self,
        cursor: Option<JournalCatalogCursor>,
        limit: u16,
    ) -> super::JournalFuture<JournalCatalogPage>;
}
