use super::JournalError;
use crate::identity::{ChatControlRevision, EventSequence, JournalSequence, StreamRevision};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JournalPosition {
    revision: StreamRevision,
    control: ChatControlRevision,
    records: JournalSequence,
    events: EventSequence,
}
impl JournalPosition {
    pub fn new(
        revision: StreamRevision,
        control: ChatControlRevision,
        records: JournalSequence,
        events: EventSequence,
    ) -> Self {
        Self {
            revision,
            control,
            records,
            events,
        }
    }
    pub fn revision(self) -> StreamRevision {
        self.revision
    }
    pub fn control(self) -> ChatControlRevision {
        self.control
    }
    pub fn records(self) -> JournalSequence {
        self.records
    }
    pub fn events(self) -> EventSequence {
        self.events
    }
    pub fn next(
        self,
        records: usize,
        events: usize,
        control_changed: bool,
    ) -> Result<Self, JournalError> {
        Ok(Self {
            revision: self.revision.next()?,
            records: self.records.advance(records)?,
            events: self.events.advance(events)?,
            control: if control_changed {
                self.control.next()?
            } else {
                self.control
            },
        })
    }
}
