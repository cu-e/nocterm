use super::{
    JournalError, JournalFuture, JournalPosition, JournalRecord, MAX_JOURNAL_RECORDS,
    MAX_TRANSACTION_BYTES,
};
use crate::{
    authority::StreamBinding,
    identity::{JournalSequence, StreamRevision},
    payload::{MAX_LOGICAL_PAYLOAD_BYTES, OpaqueBytes},
};

#[derive(Clone, Copy, Debug)]
pub struct JournalReadLimits {
    records: usize,
    bytes: usize,
}
impl JournalReadLimits {
    pub fn new(records: usize, bytes: usize) -> Result<Self, JournalError> {
        if records == 0
            || records > MAX_JOURNAL_RECORDS
            || bytes == 0
            || bytes > MAX_TRANSACTION_BYTES
        {
            return Err(JournalError::Capacity);
        }
        Ok(Self { records, bytes })
    }
    pub fn records(self) -> usize {
        self.records
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
}

#[derive(Clone, Debug)]
pub struct StoredRecord {
    stream: StreamBinding,
    sequence: JournalSequence,
    revision: StreamRevision,
    record: JournalRecord,
}
impl StoredRecord {
    pub fn new(
        stream: StreamBinding,
        sequence: JournalSequence,
        revision: StreamRevision,
        record: JournalRecord,
    ) -> Result<Self, JournalError> {
        if record
            .context()
            .run()
            .is_some_and(|run| run.stream() != stream)
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            stream,
            sequence,
            revision,
            record,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn sequence(&self) -> JournalSequence {
        self.sequence
    }
    pub fn revision(&self) -> StreamRevision {
        self.revision
    }
    pub fn record(&self) -> &JournalRecord {
        &self.record
    }
}

#[derive(Clone, Debug)]
pub struct JournalPage {
    stream: StreamBinding,
    after: JournalSequence,
    head: JournalPosition,
    records: Vec<StoredRecord>,
}
impl JournalPage {
    pub fn new(
        stream: StreamBinding,
        after: JournalSequence,
        head: JournalPosition,
        records: Vec<StoredRecord>,
        limits: JournalReadLimits,
    ) -> Result<Self, JournalError> {
        if records.len() > limits.records
            || records.iter().fold(0usize, |n, record| {
                n.saturating_add(record.record.budget_bytes())
            }) > limits.bytes
        {
            return Err(JournalError::Capacity);
        }
        if head.records() < after {
            return Err(JournalError::Gap {
                expected: after,
                actual: head.records(),
            });
        }
        let mut previous = after;
        let mut revision = StreamRevision::default();
        for record in &records {
            let expected = previous.advance(1)?;
            if record.sequence != expected {
                return Err(JournalError::Gap {
                    expected,
                    actual: record.sequence,
                });
            }
            if record.stream != stream
                || record.revision < revision
                || record.revision > head.revision()
                || record.sequence > head.records()
            {
                return Err(JournalError::Corrupt);
            }
            previous = record.sequence;
            revision = record.revision;
        }
        if records.is_empty() && head.records() != after {
            return Err(JournalError::Gap {
                expected: after.advance(1)?,
                actual: head.records(),
            });
        }
        Ok(Self {
            stream,
            after,
            head,
            records,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn head(&self) -> JournalPosition {
        self.head
    }
    pub fn records(&self) -> &[StoredRecord] {
        &self.records
    }
    pub fn next(&self) -> JournalSequence {
        self.records
            .last()
            .map_or(self.after, StoredRecord::sequence)
    }
}

#[derive(Clone, Debug)]
pub struct JournalSnapshot {
    stream: StreamBinding,
    at: JournalPosition,
    payload: OpaqueBytes,
}
impl JournalSnapshot {
    pub fn new(
        stream: StreamBinding,
        at: JournalPosition,
        payload: OpaqueBytes,
    ) -> Result<Self, JournalError> {
        if payload.is_empty() || payload.len() > MAX_LOGICAL_PAYLOAD_BYTES {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            stream,
            at,
            payload,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn at(&self) -> JournalPosition {
        self.at
    }
    pub fn payload(&self) -> &OpaqueBytes {
        &self.payload
    }
}

pub trait JournalReader: Send + Sync {
    /// Check actual stored ownership and length headers before materializing body bytes.
    /// Missing retained prefixes are Gap, never silent truncation/skip.
    fn read(
        &self,
        stream: StreamBinding,
        after: JournalSequence,
        limits: JournalReadLimits,
    ) -> JournalFuture<JournalPage>;
    fn snapshot(&self, stream: StreamBinding) -> JournalFuture<Option<JournalSnapshot>>;
}

/// Publishes an authenticated native recovery snapshot and removes only its
/// covered prefix. It never fabricates remote delivery acknowledgement.
pub trait JournalSnapshotWriter: Send + Sync {
    fn compact(
        &self,
        lease: super::WriterLease,
        expected: JournalPosition,
        snapshot: JournalSnapshot,
    ) -> JournalFuture<()>;
}
