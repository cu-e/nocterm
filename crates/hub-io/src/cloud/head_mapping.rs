//! Native head/anchor mapping after the sole SDK has authenticated its canonical DTO.
use nocterm_cloud_model::{
    authority::{RunBinding, StreamBinding},
    identity::*,
    journal::{
        CurrentChatHead, CurrentHeadFacts, HeadAnchor, HeadAnchorKind, HeadRecordPosition,
        JournalCatalogBinding,
    },
    projection::ProjectionError,
};
use nocterm_hub_model::cloud::{
    RecordKind,
    head::{ChatHead, HeadRecordAnchor},
};

pub(super) struct PinnedHeadMapper {
    binding: JournalCatalogBinding,
}
impl PinnedHeadMapper {
    pub(super) fn new(binding: JournalCatalogBinding) -> Self {
        Self { binding }
    }
    pub(super) fn for_stream(stream: &StreamBinding) -> Self {
        Self::new(JournalCatalogBinding::new(
            stream.scope().clone(),
            stream.runner(),
            stream.runner_epoch(),
        ))
    }
    pub(super) fn head(&self, value: &ChatHead) -> Result<CurrentChatHead, ProjectionError> {
        let stream = self.binding.stream(CloudChatId::from_bytes(value.chat_id));
        CurrentChatHead::new(
            stream.clone(),
            ChatKeyEpoch::new(value.chat_key_epoch).map_err(|_| ProjectionError::InvalidRecord)?,
            CurrentHeadFacts {
                writer: value
                    .current_writer
                    .map(WriterEpoch::new)
                    .transpose()
                    .map_err(|_| ProjectionError::InvalidRecord)?,
                control: ChatControlRevision::from_u64(value.control_revision),
                deleted: value.deleted,
                head: value
                    .head
                    .as_ref()
                    .map(|value| self.anchor(&stream, value))
                    .transpose()?,
                event_floor: EventSequence::from_u64(value.retained_from),
                snapshot: value
                    .snapshot
                    .as_ref()
                    .map(|value| self.anchor(&stream, value))
                    .transpose()?,
            },
        )
        .map_err(|_| ProjectionError::InvalidRecord)
    }
    pub(super) fn anchor(
        &self,
        stream: &StreamBinding,
        value: &HeadRecordAnchor,
    ) -> Result<HeadAnchor, ProjectionError> {
        if !self.binding.contains(stream) {
            return Err(ProjectionError::WrongSource);
        }
        let run = value
            .run
            .map(|run| {
                Ok::<_, ProjectionError>(RunBinding::new(
                    stream.scope().clone(),
                    stream.runner(),
                    stream.runner_epoch(),
                    stream.chat(),
                    RunId::from_bytes(run.id),
                    RunEpoch::new(run.expected_epoch)
                        .map_err(|_| ProjectionError::InvalidRecord)?,
                ))
            })
            .transpose()?;
        let position = value.position;
        let position = HeadRecordPosition::new(
            WriterEpoch::new(position.writer_epoch).map_err(|_| ProjectionError::InvalidRecord)?,
            EventSequence::from_u64(position.sequence),
            JournalSequence::from_u64(position.journal_sequence),
            StreamRevision::from_u64(position.revision),
        )
        .map_err(|_| ProjectionError::InvalidRecord)?;
        Ok(HeadAnchor::new(
            match value.kind {
                RecordKind::Event => HeadAnchorKind::Event,
                RecordKind::Snapshot => HeadAnchorKind::Snapshot,
            },
            position,
            EventId::from_bytes(value.id),
            run,
            Fingerprint::from_bytes(value.ciphertext_digest),
        ))
    }
}
