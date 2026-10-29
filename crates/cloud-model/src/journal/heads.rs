//! Current native observations; neither DTO construction nor a cursor grants authority.
use super::{JournalCatalogBinding, JournalError, JournalFuture, MAX_JOURNAL_CATALOG_CHATS};
use crate::{
    authority::{RunBinding, StreamBinding},
    identity::{
        ChatControlRevision, ChatKeyEpoch, CloudChatId, CommandId, EventId, EventSequence,
        Fingerprint, JournalSequence, StreamRevision, WriterEpoch,
    },
};

pub const MAX_CURRENT_HEADS_PAGE: usize = 64;
pub const MAX_JOURNAL_CHAT_HEAD_PLAINTEXT: usize = 8 * 1024;
pub const MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT: usize = MAX_JOURNAL_CHAT_HEAD_PLAINTEXT + 64;

/// The actual chat key generation is carried inside protected metadata. It is
/// independent of the runner identity generation in the stream coordinate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalChatHeadContext {
    stream: StreamBinding,
    writer: Option<WriterEpoch>,
    revision: u64,
}
impl JournalChatHeadContext {
    pub fn new(
        stream: StreamBinding,
        writer: Option<WriterEpoch>,
        revision: u64,
    ) -> Result<Self, JournalError> {
        if revision == 0 {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            stream,
            writer,
            revision,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn writer(&self) -> Option<WriterEpoch> {
        self.writer
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Purpose-specific runner-root protection, usable after content-key retirement.
/// Implementations compare full configured account/R pins before mapping to the
/// public crypto coordinate and bound both input and output before allocation.
/// This cannot encrypt transcript content or derive a key from descriptive IDs.
pub trait JournalChatHeadCipher: Send + Sync {
    fn seal(
        &self,
        context: &JournalChatHeadContext,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, JournalError>;
    fn open(
        &self,
        context: &JournalChatHeadContext,
        sealed: &[u8],
    ) -> Result<Vec<u8>, JournalError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadAnchorKind {
    Event,
    Snapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadRecordPosition {
    writer: WriterEpoch,
    event: EventSequence,
    journal: JournalSequence,
    revision: StreamRevision,
}
impl HeadRecordPosition {
    pub fn new(
        writer: WriterEpoch,
        event: EventSequence,
        journal: JournalSequence,
        revision: StreamRevision,
    ) -> Result<Self, JournalError> {
        if event.get() == 0 || journal.get() == 0 || revision.get() == 0 {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            writer,
            event,
            journal,
            revision,
        })
    }
    pub fn writer(self) -> WriterEpoch {
        self.writer
    }
    pub fn event(self) -> EventSequence {
        self.event
    }
    pub fn journal(self) -> JournalSequence {
        self.journal
    }
    pub fn revision(self) -> StreamRevision {
        self.revision
    }
}

/// Original signed public record coordinates retained before native ACK/purge.
/// The fingerprint names its ciphertext, never an SDK/native checkpoint body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadAnchor {
    kind: HeadAnchorKind,
    position: HeadRecordPosition,
    id: EventId,
    run: Option<RunBinding>,
    ciphertext: Fingerprint,
}
impl HeadAnchor {
    pub fn new(
        kind: HeadAnchorKind,
        position: HeadRecordPosition,
        id: EventId,
        run: Option<RunBinding>,
        ciphertext: Fingerprint,
    ) -> Self {
        Self {
            kind,
            position,
            id,
            run,
            ciphertext,
        }
    }
    pub fn kind(&self) -> HeadAnchorKind {
        self.kind
    }
    pub fn position(&self) -> HeadRecordPosition {
        self.position
    }
    pub fn id(&self) -> EventId {
        self.id
    }
    pub fn run(&self) -> Option<&RunBinding> {
        self.run.as_ref()
    }
    pub fn ciphertext(&self) -> Fingerprint {
        self.ciphertext
    }
}

/// Descriptive inputs validated together against the original stream.
#[derive(Clone, Debug)]
pub struct CurrentHeadFacts {
    pub writer: Option<WriterEpoch>,
    pub control: ChatControlRevision,
    pub deleted: bool,
    pub head: Option<HeadAnchor>,
    /// Inclusive first retained public signed event/snapshot, or zero for an
    /// actual no-event head. This is not the native outbox's exclusive cursor
    /// or the hub delivery cursor.
    pub event_floor: EventSequence,
    pub snapshot: Option<HeadAnchor>,
}

#[derive(Clone, Debug)]
pub struct CurrentChatHead {
    stream: StreamBinding,
    key_epoch: ChatKeyEpoch,
    facts: CurrentHeadFacts,
}
impl CurrentChatHead {
    pub fn new(
        stream: StreamBinding,
        key_epoch: ChatKeyEpoch,
        facts: CurrentHeadFacts,
    ) -> Result<Self, JournalError> {
        let result = Self {
            stream,
            key_epoch,
            facts,
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), JournalError> {
        let facts = &self.facts;
        let Some(head) = &facts.head else {
            return if facts.control.get() == 0
                && !facts.deleted
                && facts.event_floor.get() == 0
                && facts.snapshot.is_none()
            {
                Ok(())
            } else {
                Err(JournalError::Corrupt)
            };
        };
        let after_deleted_tail = facts.deleted
            && facts.snapshot.is_none()
            && head.position.event.get().checked_add(1) == Some(facts.event_floor.get());
        if facts
            .writer
            .is_none_or(|writer| writer < head.position.writer)
            || facts.event_floor.get() == 0
            || (facts.event_floor > head.position.event && !after_deleted_tail)
        {
            return Err(JournalError::Corrupt);
        }
        for anchor in [facts.head.as_ref(), facts.snapshot.as_ref()]
            .into_iter()
            .flatten()
        {
            if anchor
                .run
                .as_ref()
                .is_some_and(|run| run.stream() != self.stream)
            {
                return Err(JournalError::Corrupt);
            }
        }
        if let Some(snapshot) = &facts.snapshot {
            if snapshot.kind != HeadAnchorKind::Snapshot
                || snapshot.position.event != facts.event_floor
                || snapshot.position.event > head.position.event
                || facts
                    .writer
                    .is_none_or(|writer| writer < snapshot.position.writer)
                || (snapshot.position.event == head.position.event && snapshot != head)
            {
                return Err(JournalError::Corrupt);
            }
        } else if !facts.deleted && facts.event_floor.get() > 1 {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn key_epoch(&self) -> ChatKeyEpoch {
        self.key_epoch
    }
    pub fn facts(&self) -> &CurrentHeadFacts {
        &self.facts
    }
}

/// Paging binds the authenticated head commitment, including per-chat changes.
/// A membership-only catalog revision/hash cannot satisfy this coordinate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentHeadsCursor {
    binding: JournalCatalogBinding,
    revision: u64,
    root: Fingerprint,
    after: CloudChatId,
    seen: u32,
}
impl CurrentHeadsCursor {
    pub fn new(
        binding: JournalCatalogBinding,
        revision: u64,
        root: Fingerprint,
        after: CloudChatId,
        seen: u32,
    ) -> Result<Self, JournalError> {
        if revision == 0 || seen == 0 || seen as usize > MAX_JOURNAL_CATALOG_CHATS {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            binding,
            revision,
            root,
            after,
            seen,
        })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn root(&self) -> Fingerprint {
        self.root
    }
    pub fn after(&self) -> CloudChatId {
        self.after
    }
    pub fn seen(&self) -> u32 {
        self.seen
    }
}

#[derive(Clone, Debug)]
pub struct CurrentHeadsPage {
    binding: JournalCatalogBinding,
    revision: u64,
    root: Fingerprint,
    total: u32,
    after: Option<CurrentHeadsCursor>,
    heads: Vec<CurrentChatHead>,
    more: bool,
}
impl CurrentHeadsPage {
    pub fn new(
        binding: JournalCatalogBinding,
        revision: u64,
        root: Fingerprint,
        total: u32,
        after: Option<CurrentHeadsCursor>,
        heads: Vec<CurrentChatHead>,
        more: bool,
    ) -> Result<Self, JournalError> {
        let seen = after.as_ref().map_or(0, |cursor| cursor.seen);
        let end = seen
            .checked_add(u32::try_from(heads.len()).map_err(|_| JournalError::Capacity)?)
            .ok_or(JournalError::Capacity)?;
        if revision == 0
            || total as usize > MAX_JOURNAL_CATALOG_CHATS
            || heads.len() > MAX_CURRENT_HEADS_PAGE
            || end > total
            || more != (end < total)
            || (more && heads.is_empty())
            || after.as_ref().is_some_and(|cursor| {
                cursor.binding != binding
                    || cursor.revision != revision
                    || cursor.root != root
                    || heads
                        .first()
                        .is_some_and(|head| head.stream.chat() <= cursor.after)
            })
            || heads.iter().any(|head| !binding.contains(head.stream()))
            || heads
                .windows(2)
                .any(|pair| pair[0].stream.chat() >= pair[1].stream.chat())
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            binding,
            revision,
            root,
            total,
            after,
            heads,
            more,
        })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn root(&self) -> Fingerprint {
        self.root
    }
    pub fn total(&self) -> u32 {
        self.total
    }
    pub fn after(&self) -> Option<&CurrentHeadsCursor> {
        self.after.as_ref()
    }
    pub fn heads(&self) -> &[CurrentChatHead] {
        &self.heads
    }
    pub fn next_cursor(&self) -> Option<CurrentHeadsCursor> {
        self.more.then(|| CurrentHeadsCursor {
            binding: self.binding.clone(),
            revision: self.revision,
            root: self.root,
            after: self.heads[self.heads.len() - 1].stream.chat(),
            seen: self.after.as_ref().map_or(0, |cursor| cursor.seen) + self.heads.len() as u32,
        })
    }
}

#[derive(Clone, Debug)]
pub struct OriginalCommandReceiptQuery {
    pub stream: StreamBinding,
    pub key_epoch: ChatKeyEpoch,
    pub command: CommandId,
    pub digest: Fingerprint,
    pub expected_control: ChatControlRevision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginalCommandReceiptObservation {
    NotFound,
    Accepted(ChatControlRevision),
    Conflict,
    /// Uncertain storage never admits a new command or establishes a rejection.
    Unknown,
}
/// Each read revalidates its captured native account/R source. Busy/overload,
/// missing required metadata or corrupt rows are typed refusals, not stale heads.
/// No method acquires a writer, admits a command, or opens model/SSH access.
pub trait CurrentCloudHeads: Send + Sync {
    fn catalog(
        &self,
        after: Option<CurrentHeadsCursor>,
        limit: u16,
    ) -> JournalFuture<CurrentHeadsPage>;
    fn chat(&self, stream: StreamBinding) -> JournalFuture<Option<CurrentChatHead>>;
    fn receipt(
        &self,
        query: OriginalCommandReceiptQuery,
    ) -> JournalFuture<OriginalCommandReceiptObservation>;
}
