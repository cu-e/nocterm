//! One bounded source of authoritative native records for recovery and history.
use super::services::CoordinatorServices;
use crate::codec::archive::NativeRunArchive;
use nocterm_cloud_model::{
    authority::StreamBinding, coordinator::CoordinatorError, identity::*, journal::*,
};

pub(super) struct JournalReplay {
    records: Vec<StoredRecord>,
    head: JournalPosition,
    bytes: usize,
}
impl JournalReplay {
    pub(super) fn records(&self) -> &[StoredRecord] {
        &self.records
    }
    pub(super) async fn load(
        services: &CoordinatorServices,
        stream: StreamBinding,
    ) -> Result<Self, CoordinatorError> {
        let mut replay = Self {
            records: Vec::new(),
            head: JournalPosition::default(),
            bytes: 0,
        };
        let mut after = JournalSequence::default();
        loop {
            let lookup = stream.clone();
            let limits = JournalReadLimits::new(64, 64 * 1024 * 1024)?;
            let result = services
                .io()
                .run(move |journal| journal.read(lookup, after, limits))
                .await;
            let page = match result {
                Ok(page) => page,
                Err(CoordinatorError::Journal(JournalError::Gap { actual, .. }))
                    if after.get() == 0 && actual.get() > 0 =>
                {
                    let lookup = stream.clone();
                    let snapshot = services
                        .io()
                        .run(move |journal| journal.snapshot(lookup))
                        .await?
                        .ok_or(JournalError::Corrupt)?;
                    if snapshot.stream() != &stream || snapshot.at().records() != actual {
                        return Err(JournalError::Conflict.into());
                    }
                    let archive = NativeRunArchive::decode(&snapshot)?;
                    replay.head = snapshot.at();
                    for record in archive.into_records() {
                        replay.push(record)?;
                    }
                    after = actual;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if page.stream() != &stream
                || page.head().revision() < replay.head.revision()
                || page.head().control() < replay.head.control()
                || page.head().records() < replay.head.records()
                || page.head().events() < replay.head.events()
            {
                return Err(JournalError::Corrupt.into());
            }
            replay.head = page.head();
            after = page.next();
            for record in page.records() {
                replay.push(record.clone())?;
            }
            if after == replay.head.records() {
                return Ok(replay);
            }
        }
    }
    fn push(&mut self, record: StoredRecord) -> Result<(), CoordinatorError> {
        let previous = self
            .records
            .last()
            .map_or(JournalSequence::default(), StoredRecord::sequence);
        if record.sequence() != previous.advance(1)?
            || self
                .records
                .last()
                .is_some_and(|prior| record.revision() < prior.revision())
        {
            return Err(JournalError::Corrupt.into());
        }
        self.bytes = self.bytes.saturating_add(record.record().payload().len());
        if self.bytes > 64 * 1024 * 1024 || self.records.len() >= 16384 {
            return Err(JournalError::Capacity.into());
        }
        self.records.push(record);
        Ok(())
    }
}
