use super::{
    identity::{Run, Stream, fp, namespace},
    record::{command_kind, command_tag},
};
use crate::storage::StorageBudget;
use nocterm_cloud_model::{
    authority::{CommandIssuer, StreamBinding},
    engine::EngineCheckpoint,
    identity::*,
    journal::*,
    payload::OpaqueBytes,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Position {
    revision: u64,
    control: u64,
    records: u64,
    events: u64,
}
impl From<JournalPosition> for Position {
    fn from(value: JournalPosition) -> Self {
        Self {
            revision: value.revision().get(),
            control: value.control().get(),
            records: value.records().get(),
            events: value.events().get(),
        }
    }
}
impl Position {
    pub(crate) fn restore(self) -> Result<JournalPosition, JournalError> {
        if self.control > self.revision || self.revision > self.records {
            return Err(JournalError::Corrupt);
        }
        Ok(JournalPosition::new(
            StreamRevision::from_u64(self.revision),
            ChatControlRevision::from_u64(self.control),
            JournalSequence::from_u64(self.records),
            EventSequence::from_u64(self.events),
        ))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Header {
    stream: Stream,
    writer: u64,
    position: Position,
    retained: u64,
    deleted: bool,
    charge: StorageBudget,
    active: Vec<[u8; 16]>,
    acknowledged: u64,
    event_floor: u64,
    transfer: Option<[u8; 32]>,
}
impl Header {
    pub(crate) fn new(
        stream: &StreamBinding,
        writer: WriterEpoch,
        position: JournalPosition,
        retained: JournalSequence,
        deleted: bool,
        charge: StorageBudget,
        active: Vec<[u8; 16]>,
        acknowledged: EventSequence,
        event_floor: EventSequence,
        transfer: Option<[u8; 32]>,
    ) -> Self {
        Self {
            stream: stream.into(),
            writer: writer.get(),
            position: position.into(),
            retained: retained.get(),
            deleted,
            charge,
            active,
            acknowledged: acknowledged.get(),
            event_floor: event_floor.get(),
            transfer,
        }
    }
    pub(crate) fn restore(
        self,
    ) -> Result<
        (
            StreamBinding,
            WriterEpoch,
            JournalPosition,
            JournalSequence,
            bool,
            StorageBudget,
            Vec<[u8; 16]>,
            EventSequence,
            EventSequence,
            Option<[u8; 32]>,
        ),
        JournalError,
    > {
        let position = self.position.restore()?;
        self.charge.validate()?;
        if self.retained > position.records().get()
            || self.active.len() > 256
            || self.active.windows(2).any(|pair| pair[0] >= pair[1])
            || self.event_floor > self.acknowledged
            || self.acknowledged > position.events().get()
        {
            return Err(JournalError::Corrupt);
        }
        Ok((
            self.stream.restore()?,
            WriterEpoch::new(self.writer)?,
            position,
            JournalSequence::from_u64(self.retained),
            self.deleted,
            self.charge,
            self.active,
            EventSequence::from_u64(self.acknowledged),
            EventSequence::from_u64(self.event_floor),
            self.transfer,
        ))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Receipt {
    stream: Stream,
    writer: u64,
    position: Position,
    command: [u8; 16],
    kind: u8,
    digest: [u8; 32],
    device: String,
    key: [u8; 32],
    invocation: [u8; 32],
    expected: u64,
}
impl From<&CommandReceipt> for Receipt {
    fn from(value: &CommandReceipt) -> Self {
        let acceptance = value.acceptance();
        Self {
            stream: value.lease().stream().into(),
            writer: value.lease().epoch().get(),
            position: value.position().into(),
            command: *acceptance.id().as_bytes(),
            kind: command_tag(acceptance.kind()),
            digest: *acceptance.digest().as_bytes(),
            device: acceptance.issuer().device().as_str().into(),
            key: *acceptance.issuer().key().as_bytes(),
            invocation: *acceptance.issuer().invocation().as_bytes(),
            expected: acceptance.expected_control().get(),
        }
    }
}
impl Receipt {
    pub(crate) fn restore(self) -> Result<CommandReceipt, JournalError> {
        let acceptance = CommandAcceptance::new(
            CommandId::from_bytes(self.command),
            command_kind(self.kind)?,
            fp(self.digest),
            CommandIssuer::new(namespace(self.device)?, fp(self.key), fp(self.invocation)),
            ChatControlRevision::from_u64(self.expected),
        );
        CommandReceipt::from_durable(
            WriterLease::new(self.stream.restore()?, WriterEpoch::new(self.writer)?),
            self.position.restore()?,
            acceptance,
        )
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    run: Run,
    format: String,
    revision: u64,
    bytes: Vec<u8>,
}
impl From<&EngineCheckpoint> for Checkpoint {
    fn from(value: &EngineCheckpoint) -> Self {
        Self {
            run: value.run().into(),
            format: value.format().as_str().into(),
            revision: value.journal_revision().get(),
            bytes: value.bytes().as_bytes().to_vec(),
        }
    }
}
impl Checkpoint {
    pub(crate) fn restore(self) -> Result<EngineCheckpoint, JournalError> {
        EngineCheckpoint::new(
            self.run.restore()?,
            namespace(self.format)?,
            StreamRevision::from_u64(self.revision),
            OpaqueBytes::new(self.bytes)?,
        )
        .map_err(|_| JournalError::Corrupt)
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot {
    stream: Stream,
    position: Position,
    payload: Vec<u8>,
}
impl From<&JournalSnapshot> for Snapshot {
    fn from(value: &JournalSnapshot) -> Self {
        Self {
            stream: value.stream().into(),
            position: value.at().into(),
            payload: value.payload().as_bytes().to_vec(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Outbox {
    version: u8,
    delivery: [u8; 16],
    anchor: super::head::Anchor,
    material: u64,
    sealed: Vec<u8>,
}
impl From<&OutboxEntry> for Outbox {
    fn from(value: &OutboxEntry) -> Self {
        Self {
            version: 2,
            delivery: *value.delivery_id().as_bytes(),
            anchor: value.anchor().into(),
            material: value.key_epoch().get(),
            sealed: value.sealed().as_bytes().to_vec(),
        }
    }
}
impl Outbox {
    pub(crate) fn restore(self) -> Result<OutboxEntry, JournalError> {
        if self.version != 2 {
            return Err(JournalError::UnsupportedFormat);
        }
        if self.sealed.is_empty() {
            return Err(JournalError::Corrupt);
        }
        Ok(OutboxEntry::new(
            self.anchor.restore()?,
            ChatKeyEpoch::new(self.material)?,
            OutboxDeliveryId::from_bytes(self.delivery),
            OpaqueBytes::new(self.sealed)?,
        ))
    }
}
impl Snapshot {
    pub(crate) fn restore(self) -> Result<JournalSnapshot, JournalError> {
        JournalSnapshot::new(
            self.stream.restore()?,
            self.position.restore()?,
            OpaqueBytes::new(self.payload)?,
        )
    }
}
