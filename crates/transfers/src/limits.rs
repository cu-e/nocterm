use crate::{TransferCause, TransferError};
#[derive(Clone, Copy, Debug)]
pub struct QueueLimits {
    queued: usize,
    backlog: usize,
    parallel: usize,
    history: usize,
}
impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            queued: 8,
            backlog: 32,
            parallel: 4,
            history: 64,
        }
    }
}
impl QueueLimits {
    pub fn new(
        queued: usize,
        backlog: usize,
        parallel: usize,
        history: usize,
    ) -> Result<Self, TransferError> {
        bounded(queued, 8, "queued batches")?;
        bounded(backlog, 32, "file backlog")?;
        bounded(parallel, 4, "parallel files")?;
        bounded(history, 64, "finished history")?;
        Ok(Self {
            queued,
            backlog,
            parallel,
            history,
        })
    }
    pub fn queued(self) -> usize {
        self.queued
    }
    pub fn backlog(self) -> usize {
        self.backlog
    }
    pub fn parallel(self) -> usize {
        self.parallel
    }
    pub fn history(self) -> usize {
        self.history
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryLimits {
    source_paths: usize,
    source_bytes: usize,
    source_path_bytes: usize,
    depth: usize,
    visited: usize,
    listing_entries: usize,
    listing_bytes: usize,
    rename_attempts: u32,
}
impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            source_paths: 4096,
            source_bytes: 1024 * 1024,
            source_path_bytes: 16 * 1024,
            depth: 256,
            visited: 1_000_000,
            listing_entries: 100_000,
            listing_bytes: 8 * 1024 * 1024,
            rename_attempts: 10_000,
        }
    }
}
impl DiscoveryLimits {
    pub fn with_walk_bounds(
        depth: usize,
        visited: usize,
        listing_entries: usize,
        listing_bytes: usize,
    ) -> Result<Self, TransferError> {
        bounded(depth, 256, "discovery depth")?;
        bounded(visited, 1_000_000, "visited entries")?;
        bounded(listing_entries, 100_000, "pending entries")?;
        bounded(listing_bytes, 8 * 1024 * 1024, "pending names")?;
        Ok(Self {
            depth,
            visited,
            listing_entries,
            listing_bytes,
            ..Self::default()
        })
    }
    pub fn source_path_bytes(self) -> usize {
        self.source_path_bytes
    }
    pub fn depth(self) -> usize {
        self.depth
    }
    pub fn visited(self) -> usize {
        self.visited
    }
    pub fn listing_entries(self) -> usize {
        self.listing_entries
    }
    pub fn listing_bytes(self) -> usize {
        self.listing_bytes
    }
    pub fn rename_attempts(self) -> u32 {
        self.rename_attempts
    }
    pub fn validate_sources(
        self,
        lengths: impl Iterator<Item = usize>,
    ) -> Result<(), TransferError> {
        let mut count = 0usize;
        let mut bytes = 0usize;
        for length in lengths {
            count = count.saturating_add(1);
            bytes = bytes.saturating_add(length);
            if count > self.source_paths
                || length > self.source_path_bytes
                || bytes > self.source_bytes
            {
                return Err(TransferError::RequestTooLarge);
            }
        }
        if count == 0 {
            Err(TransferError::Empty)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct StreamLimits {
    chunk: usize,
    acknowledgements: usize,
    details: usize,
}
impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            chunk: 32 * 1024,
            acknowledgements: 8,
            details: 100,
        }
    }
}
impl StreamLimits {
    pub fn new(
        chunk: usize,
        acknowledgements: usize,
        details: usize,
    ) -> Result<Self, TransferError> {
        bounded(chunk, 32 * 1024, "chunk bytes")?;
        bounded(acknowledgements, 8, "acknowledgement batch")?;
        bounded(details, 100, "diagnostic details")?;
        Ok(Self {
            chunk,
            acknowledgements,
            details,
        })
    }
    pub fn chunk(self) -> usize {
        self.chunk
    }
    pub fn acknowledgements(self) -> usize {
        self.acknowledgements
    }
    pub fn details(self) -> usize {
        self.details
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct TransferLimits {
    queue: QueueLimits,
    discovery: DiscoveryLimits,
    stream: StreamLimits,
}
impl TransferLimits {
    pub fn new(queue: QueueLimits, discovery: DiscoveryLimits, stream: StreamLimits) -> Self {
        Self {
            queue,
            discovery,
            stream,
        }
    }
    pub fn queue(self) -> QueueLimits {
        self.queue
    }
    pub fn discovery(self) -> DiscoveryLimits {
        self.discovery
    }
    pub fn stream(self) -> StreamLimits {
        self.stream
    }
}
fn bounded(value: usize, maximum: usize, label: &'static str) -> Result<(), TransferError> {
    if value == 0 || value > maximum {
        Err(TransferError::InvalidLimit(label))
    } else {
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LimitKind {
    #[error("directory depth")]
    Depth,
    #[error("visited entries")]
    Visited,
    #[error("pending directory entries")]
    PendingEntries,
    #[error("pending directory name bytes")]
    PendingNames,
    #[error("destination rename attempts")]
    RenameAttempts,
    #[error("byte accounting overflow")]
    ByteOverflow,
}
pub struct DiscoveryBudget {
    limits: DiscoveryLimits,
    visited: usize,
    pending_entries: usize,
    pending_bytes: usize,
}
impl DiscoveryBudget {
    pub fn new(limits: DiscoveryLimits) -> Self {
        Self {
            limits,
            visited: 0,
            pending_entries: 0,
            pending_bytes: 0,
        }
    }
    pub fn visit(&mut self, depth: usize) -> Result<(), TransferCause> {
        if depth >= self.limits.depth {
            return Err(TransferCause::Limit(LimitKind::Depth));
        }
        if self.visited >= self.limits.visited {
            return Err(TransferCause::Limit(LimitKind::Visited));
        }
        self.visited += 1;
        Ok(())
    }
    pub fn hold_listing(&mut self, entries: usize, bytes: usize) -> Result<(), TransferCause> {
        let entries = self.pending_entries.saturating_add(entries);
        let bytes = self.pending_bytes.saturating_add(bytes);
        if entries > self.limits.listing_entries {
            return Err(TransferCause::Limit(LimitKind::PendingEntries));
        }
        if bytes > self.limits.listing_bytes {
            return Err(TransferCause::Limit(LimitKind::PendingNames));
        }
        self.pending_entries = entries;
        self.pending_bytes = bytes;
        Ok(())
    }
    pub fn consumed_name(&mut self, bytes: usize) {
        self.pending_entries = self.pending_entries.saturating_sub(1);
        self.pending_bytes = self.pending_bytes.saturating_sub(bytes);
    }
}
pub struct StreamBudget {
    limits: StreamLimits,
    expected: Option<u64>,
    transferred: u64,
}
impl StreamBudget {
    pub fn new(limits: StreamLimits, expected: Option<u64>) -> Self {
        Self {
            limits,
            expected,
            transferred: 0,
        }
    }
    pub fn accept_chunk(&self, bytes: usize) -> Result<(), TransferCause> {
        self.accept_buffered(0, bytes)
    }
    pub fn accept_buffered(&self, buffered: usize, bytes: usize) -> Result<(), TransferCause> {
        if bytes > self.limits.chunk {
            return Err(TransferCause::OversizedChunk {
                requested: self.limits.chunk,
                received: bytes,
            });
        }
        let next = self
            .transferred
            .checked_add(buffered as u64)
            .and_then(|value| value.checked_add(bytes as u64))
            .ok_or(TransferCause::Limit(LimitKind::ByteOverflow))?;
        if self.expected.is_some_and(|expected| next > expected) {
            return Err(TransferCause::SizeChanged {
                expected: self.expected,
                observed: next,
            });
        }
        Ok(())
    }
    pub fn acknowledged(&mut self, bytes: usize) -> Result<(), TransferCause> {
        self.transferred = self
            .transferred
            .checked_add(bytes as u64)
            .ok_or(TransferCause::Limit(LimitKind::ByteOverflow))?;
        if self
            .expected
            .is_some_and(|expected| self.transferred > expected)
        {
            return Err(TransferCause::SizeChanged {
                expected: self.expected,
                observed: self.transferred,
            });
        }
        Ok(())
    }
    pub fn finish(&self) -> Result<(), TransferCause> {
        if self
            .expected
            .is_some_and(|expected| expected != self.transferred)
        {
            Err(TransferCause::SizeChanged {
                expected: self.expected,
                observed: self.transferred,
            })
        } else {
            Ok(())
        }
    }
    pub fn transferred(&self) -> u64 {
        self.transferred
    }
}
