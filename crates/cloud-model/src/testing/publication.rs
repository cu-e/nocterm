//! Non-cryptographic signed-record and relay fixtures, with real finite permits.
use crate::{coordinator::*, identity::*, journal::*, payload::OpaqueBytes};
use nocterm_session::testing::Gate;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};

pub fn fixture_snapshot_budget() -> PublicationBudget {
    PublicationBudget::maintenance(IntakePoolLimits::new(512 * 1024 * 1024, 4096, 4096).unwrap())
}
#[derive(Default)]
pub struct ScriptedSnapshotEncoder;
impl CoordinatorSnapshotEncoder for ScriptedSnapshotEncoder {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        source: &SnapshotProjectionSource,
        limits: SnapshotLimits,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError> {
        let length = source
            .records()
            .len()
            .checked_mul(16)
            .and_then(|n| n.checked_add(32))
            .ok_or(JournalError::Capacity)?;
        if length > limits.output() {
            return Err(JournalError::Capacity);
        }
        let allocation = work.try_reserve(PublicationAllocation::new(
            ResidentPublication::required_bytes(length)?,
            1,
        )?)?;
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(&source.covered().records().get().to_be_bytes());
        bytes.extend_from_slice(&source.covered().events().get().to_be_bytes());
        for entry in source.records() {
            bytes.extend_from_slice(entry.id().as_bytes());
        }
        let digest = Fingerprint::from_bytes(Sha256::digest(&bytes).into());
        let mut id = [0x51; 16];
        id[8..].copy_from_slice(&next.events().get().to_be_bytes());
        let mut delivery = [0x91; 16];
        delivery[8..].copy_from_slice(&next.events().get().to_be_bytes());
        ResidentPublication::new(
            lease.stream().clone(),
            OutboxEntry::new(
                HeadAnchor::new(
                    HeadAnchorKind::Snapshot,
                    HeadRecordPosition::new(
                        lease.epoch(),
                        next.events(),
                        next.records(),
                        next.revision(),
                    )?,
                    EventId::from_bytes(id),
                    None,
                    digest,
                ),
                source.material(),
                OutboxDeliveryId::from_bytes(delivery),
                OpaqueBytes::new(bytes)?,
            ),
            allocation,
        )
    }
}
#[derive(Default)]
struct Relay {
    stored: BTreeMap<OutboxDeliveryId, (ResidentPublication, OutboxStored)>,
    attempts: Vec<ResidentPublication>,
    purge: Vec<u64>,
    lose_reply: bool,
    gate: Option<Gate>,
}
#[derive(Default, Clone)]
pub struct ScriptedPublicationTransport {
    relay: Arc<Mutex<Relay>>,
}
impl ScriptedPublicationTransport {
    pub fn lose_next_reply(&self) {
        self.relay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lose_reply = true;
    }
    pub fn pause(&self) -> Gate {
        let gate = Gate::pending();
        self.relay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .gate = Some(gate.clone());
        gate
    }
    pub fn attempts(&self) -> Vec<ResidentPublication> {
        self.relay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts
            .clone()
    }
    pub fn purged(&self) -> Vec<u64> {
        self.relay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .purge
            .clone()
    }
}
impl OutboxPublicationTransport for ScriptedPublicationTransport {
    fn publish(&self, publication: ResidentPublication) -> JournalFuture<OutboxStored> {
        let owner = self.clone();
        Box::pin(async move {
            let gate = owner
                .relay
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .gate
                .clone();
            if let Some(gate) = gate {
                gate.wait().await;
            }
            let mut relay = owner.relay.lock().unwrap_or_else(PoisonError::into_inner);
            if relay.attempts.len() >= 256 {
                return Err(JournalError::Capacity);
            }
            relay.attempts.push(publication.clone());
            let receipt = if let Some((old, receipt)) = relay.stored.get(&publication.delivery_id())
            {
                if old.stream() != publication.stream()
                    || old.anchor() != publication.anchor()
                    || old.sealed() != publication.sealed()
                {
                    return Err(JournalError::Conflict);
                }
                receipt.clone()
            } else {
                let cursor = DeliveryCursor::new(97 + relay.stored.len() as u64)?;
                let receipt = OutboxStored::new(
                    publication.stream().clone(),
                    publication.delivery_id(),
                    publication.anchor().ciphertext(),
                    cursor,
                );
                relay.stored.insert(
                    publication.delivery_id(),
                    (publication.clone(), receipt.clone()),
                );
                receipt
            };
            if std::mem::take(&mut relay.lose_reply) {
                return Err(JournalError::Conflict);
            }
            Ok(receipt)
        })
    }
    fn purge(&self, candidate: SnapshotCandidate) -> JournalFuture<SnapshotPurged> {
        let owner = self.clone();
        Box::pin(async move {
            let stored = candidate.stored().ok_or(JournalError::Conflict)?;
            let through = stored.cursor().get() - 1;
            let mut relay = owner.relay.lock().unwrap_or_else(PoisonError::into_inner);
            let (publication, receipt) = relay
                .stored
                .get(&candidate.delivery_id())
                .ok_or(JournalError::NotFound)?;
            if receipt != stored
                || publication.anchor() != candidate.anchor()
                || publication.anchor().kind() != HeadAnchorKind::Snapshot
            {
                return Err(JournalError::Conflict);
            }
            if relay.purge.len() >= 256 {
                return Err(JournalError::Capacity);
            }
            relay.purge.push(through);
            SnapshotPurged::new(stored.clone(), through)
        })
    }
}
