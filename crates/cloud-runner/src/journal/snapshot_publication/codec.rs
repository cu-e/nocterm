//! Small authenticated transfer phases; body hashes bind separately bounded blobs.
use crate::codec::{Position, head::Anchor, identity::Stream};
use nocterm_cloud_model::{identity::*, journal::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(in crate::journal) struct Candidate {
    stream: Stream,
    material: u64,
    anchor: Anchor,
    delivery: [u8; 16],
    length: u64,
    covered: Position,
    archive: [u8; 32],
    archive_length: u64,
    stored: Option<(u64, [u8; 32])>,
}
impl From<&SnapshotCandidate> for Candidate {
    fn from(value: &SnapshotCandidate) -> Self {
        Self {
            stream: value.stream().into(),
            material: value.material().get(),
            anchor: value.anchor().into(),
            delivery: *value.delivery_id().as_bytes(),
            length: value.length() as u64,
            covered: value.covered().into(),
            archive: *value.archive().as_bytes(),
            archive_length: value.archive_length() as u64,
            stored: value
                .stored()
                .map(|s| (s.cursor().get(), *s.ciphertext().as_bytes())),
        }
    }
}
impl Candidate {
    pub(in crate::journal) fn restore(self) -> Result<SnapshotCandidate, JournalError> {
        let stream = self.stream.restore()?;
        let delivery = OutboxDeliveryId::from_bytes(self.delivery);
        let stored = self
            .stored
            .map(|(cursor, digest)| {
                Ok::<_, JournalError>(OutboxStored::new(
                    stream.clone(),
                    delivery,
                    Fingerprint::from_bytes(digest),
                    DeliveryCursor::new(cursor)?,
                ))
            })
            .transpose()?;
        SnapshotCandidate::from_durable(
            SnapshotDescriptor {
                stream,
                material: ChatKeyEpoch::new(self.material)?,
                anchor: self.anchor.restore()?,
                delivery,
                length: usize::try_from(self.length).map_err(|_| JournalError::Corrupt)?,
                covered: self.covered.restore()?,
                archive: Fingerprint::from_bytes(self.archive),
                archive_length: usize::try_from(self.archive_length)
                    .map_err(|_| JournalError::Corrupt)?,
            },
            stored,
        )
    }
}

#[derive(Serialize, Deserialize)]
pub(in crate::journal) struct Transfer {
    pub(in crate::journal) pending: Option<Candidate>,
    pub(in crate::journal) active: Option<Candidate>,
}
