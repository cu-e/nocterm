//! Bounds one operation's sealed snapshots before retaining another object or page.
use std::{collections::BTreeMap, sync::Arc};

use nocterm_hub_proto::v1;

use crate::{
    cache::{ObjectCopy, VaultCopy},
    classify::remote,
    store::OpError,
};

pub(super) fn merge_vault(
    mut vault: Arc<VaultCopy>,
    budget: Option<SnapshotBudget>,
    envelope: Vec<u8>,
    batch: v1::ChangeBatch,
) -> Result<(Arc<VaultCopy>, SnapshotBudget, bool), OpError> {
    let mut budget = match budget {
        Some(budget) => budget,
        None => {
            let mut budget = SnapshotBudget::default();
            budget.add(&vault.objects)?;
            budget
        }
    };
    let next = Arc::make_mut(&mut vault);
    next.key_envelope = envelope;
    let more = budget.apply(&mut next.objects, &mut next.cursor, batch)?;
    Ok((vault, budget, more))
}

const MAX_BYTES: usize = 24 * 1024 * 1024;
const MAX_OBJECTS: usize = 32_768;
pub(super) const MAX_GROUPS: usize = 256;
const MAX_PAGES: usize = 1_024;

#[derive(Debug, thiserror::Error)]
enum SnapshotError {
    #[error("the vault snapshot exceeds its memory or page limit")]
    Capacity,
    #[error("the vault change page does not advance its cursor")]
    Progress,
}

#[derive(Default)]
pub(super) struct SnapshotBudget {
    bytes: usize,
    objects: usize,
    pages: usize,
}

impl SnapshotBudget {
    pub(super) fn add(&mut self, objects: &BTreeMap<String, ObjectCopy>) -> Result<(), OpError> {
        for (id, object) in objects {
            self.admit(0, charge(id, object), true)?;
        }
        Ok(())
    }

    fn admit(&mut self, old: usize, next: usize, fresh: bool) -> Result<(), OpError> {
        let bytes = self.bytes.saturating_sub(old).saturating_add(next);
        let objects = self.objects.saturating_add(usize::from(fresh));
        if bytes > MAX_BYTES || objects > MAX_OBJECTS {
            return Err(OpError::Hub(remote(SnapshotError::Capacity)));
        }
        self.bytes = bytes;
        self.objects = objects;
        Ok(())
    }

    pub(super) fn apply(
        &mut self,
        objects: &mut BTreeMap<String, ObjectCopy>,
        cursor: &mut u64,
        batch: v1::ChangeBatch,
    ) -> Result<bool, OpError> {
        self.pages += 1;
        if self.pages > MAX_PAGES {
            return Err(OpError::Hub(remote(SnapshotError::Capacity)));
        }
        let before = *cursor;
        for object in batch.changes {
            if object.stream_version <= before || object.stream_version > batch.head {
                return Err(OpError::Hub(remote(SnapshotError::Progress)));
            }
            *cursor = (*cursor).max(object.stream_version);
            if objects
                .get(&object.object_id)
                .is_some_and(|held| held.version > object.version)
            {
                continue;
            }
            let next = ObjectCopy {
                version: object.version,
                kind: object.kind,
                ciphertext: object.ciphertext,
                deleted: object.deleted,
            };
            let old = objects
                .get_key_value(&object.object_id)
                .map(|(id, old)| charge(id, old));
            self.admit(
                old.unwrap_or(0),
                charge(&object.object_id, &next),
                old.is_none(),
            )?;
            objects.insert(object.object_id, next);
        }
        if batch.more && *cursor == before {
            return Err(OpError::Hub(remote(SnapshotError::Progress)));
        }
        if !batch.more {
            *cursor = (*cursor).max(batch.head);
        }
        Ok(batch.more)
    }
}

fn charge(id: &String, object: &ObjectCopy) -> usize {
    // Includes tree/node metadata, JSON escaping and base64 expansion, as well
    // as retained capacities. This leaves file framing/keyset headroom below 32 MiB.
    256usize
        .saturating_add(id.capacity().saturating_mul(6))
        .saturating_add(object.kind.capacity().saturating_mul(6))
        .saturating_add(object.ciphertext.capacity().saturating_mul(4).div_ceil(3))
}
