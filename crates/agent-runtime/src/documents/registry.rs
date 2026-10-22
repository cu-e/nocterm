//! Installation-wide document ownership and revision delivery, keyed by context.
use super::{CandidateStamp, Client};
use crate::ChatOpenError;
use gpui::EntityId;
use nocterm_ai::execution_origin::ChatKey;
use nocterm_store::KeyedStamp;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
};

#[derive(Clone, Copy)]
struct DocumentOwner {
    entity: Option<EntityId>,
    window: EntityId,
    incarnation: u64,
    reserved: bool,
}

/// Noncloneable ownership reservation. Drop releases only its original incarnation.
pub struct DocumentClaim {
    registry: Weak<RefCell<DocumentRegistry>>,
    key: ChatKey,
    owner: DocumentOwner,
    previous: Option<DocumentOwner>,
}
impl DocumentClaim {
    pub fn key(&self) -> &ChatKey {
        &self.key
    }
    pub(super) fn belongs_to(&self, registry: &Rc<RefCell<DocumentRegistry>>) -> bool {
        self.registry
            .upgrade()
            .is_some_and(|original| Rc::ptr_eq(&original, registry))
    }
}
impl Drop for DocumentClaim {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.borrow_mut().rollback(self);
        }
    }
}

pub enum DocumentClaimOutcome {
    Owned(DocumentClaim),
    AlreadyOpen { window: EntityId },
    Deleted,
}

#[derive(Default)]
pub(super) struct DocumentRegistry {
    clients: HashMap<EntityId, Client>,
    owners: HashMap<ChatKey, DocumentOwner>,
    deleted: HashSet<ChatKey>,
    stamps: HashMap<ChatKey, (EntityId, u64)>,
    closing: bool,
    sequence: u64,
}
impl DocumentRegistry {
    pub(super) fn claim(
        registry: &Rc<RefCell<Self>>,
        key: ChatKey,
        window: EntityId,
        replacing: Option<EntityId>,
    ) -> Result<DocumentClaimOutcome, ChatOpenError> {
        let mut this = registry.borrow_mut();
        if this.closing {
            return Err(ChatOpenError::Closed);
        }
        if this.deleted.contains(&key) {
            return Ok(DocumentClaimOutcome::Deleted);
        }
        if let Some(entity) = this
            .owners
            .get(&key)
            .filter(|owner| !owner.reserved)
            .and_then(|owner| owner.entity)
            && this
                .clients
                .get(&entity)
                .is_none_or(|client| !client.alive())
        {
            this.retire(entity);
        }
        let previous = this.owners.get(&key).copied();
        if let Some(expected) = replacing {
            if previous.is_none_or(|owner| owner.reserved || owner.entity != Some(expected)) {
                return Err(ChatOpenError::StaleClaim(key));
            }
        } else if let Some(owner) = previous {
            return Ok(DocumentClaimOutcome::AlreadyOpen {
                window: owner.window,
            });
        }
        let incarnation = this
            .sequence
            .checked_add(1)
            .ok_or(ChatOpenError::IncarnationExhausted)?;
        this.sequence = incarnation;
        let owner = DocumentOwner {
            entity: previous.and_then(|owner| owner.entity),
            window,
            incarnation,
            reserved: true,
        };
        this.owners.insert(key.clone(), owner);
        Ok(DocumentClaimOutcome::Owned(DocumentClaim {
            registry: Rc::downgrade(registry),
            key,
            owner,
            previous,
        }))
    }
    fn reservation_current(&self, claim: &DocumentClaim) -> bool {
        self.owners
            .get(&claim.key)
            .is_some_and(|owner| owner.incarnation == claim.owner.incarnation && owner.reserved)
    }
    fn rollback(&mut self, claim: &DocumentClaim) {
        if !self.reservation_current(claim) {
            return;
        }
        self.owners.remove(&claim.key);
        if let Some(previous) = claim.previous
            && previous
                .entity
                .is_some_and(|id| self.clients.get(&id).is_some_and(|client| client.alive()))
        {
            self.owners.insert(claim.key.clone(), previous);
        }
    }
    pub(super) fn require_reservation(&self, claim: &DocumentClaim) -> Result<(), ChatOpenError> {
        if self.closing {
            return Err(ChatOpenError::Closed);
        }
        if self.deleted.contains(&claim.key) {
            return Err(ChatOpenError::Deleted(claim.key.clone()));
        }
        if !self.reservation_current(claim) {
            return Err(ChatOpenError::StaleClaim(claim.key.clone()));
        }
        Ok(())
    }
    pub(super) fn commit(
        &mut self,
        claim: &DocumentClaim,
        client: Client,
    ) -> Result<(), ChatOpenError> {
        self.require_reservation(claim)?;
        if self
            .owners
            .iter()
            .any(|(key, owner)| key != claim.key() && owner.entity == Some(client.id()))
        {
            return Err(ChatOpenError::StaleClaim(claim.key.clone()));
        }
        if let Some(previous) = claim.previous.and_then(|owner| owner.entity) {
            self.clients.remove(&previous);
            self.stamps.remove(&claim.key);
        }
        let owner = client.id();
        self.owners.insert(
            claim.key.clone(),
            DocumentOwner {
                entity: Some(owner),
                reserved: false,
                ..claim.owner
            },
        );
        self.clients.insert(owner, client);
        Ok(())
    }
    pub(super) fn retire(&mut self, id: EntityId) -> Vec<ChatKey> {
        let ids = self
            .owners
            .iter()
            .filter(|(_, owner)| owner.entity == Some(id))
            .map(|(id, _)| id.clone())
            .collect();
        self.clients.remove(&id);
        self.owners.retain(|_, owner| owner.entity != Some(id));
        self.stamps.retain(|_, (owner, _)| *owner != id);
        ids
    }
    pub(super) fn current(&self, id: &ChatKey, owner: EntityId) -> bool {
        !self.closing
            && self.owners.get(id).is_some_and(|current| !current.reserved)
            && self.capture_current(id, owner)
    }
    pub(super) fn capture_current(&self, id: &ChatKey, owner: EntityId) -> bool {
        !self.deleted.contains(id)
            && self
                .owners
                .get(id)
                .is_some_and(|current| current.entity == Some(owner))
    }
    pub(super) fn accepted(&mut self, id: &ChatKey, owner: EntityId, revision: u64) {
        self.stamps.insert(id.to_owned(), (owner, revision));
    }
    pub(super) fn accepts_revision(&self, id: &ChatKey, owner: EntityId, revision: u64) -> bool {
        self.capture_current(id, owner)
            && self
                .stamps
                .get(id)
                .is_none_or(|(accepted_owner, accepted_revision)| {
                    *accepted_owner != owner || revision >= *accepted_revision
                })
    }
    pub(super) fn seal(&mut self, id: ChatKey) {
        self.deleted.insert(id.clone());
        self.stamps.remove(&id);
        self.owners.remove(&id);
    }
    pub(super) fn target(
        &self,
        stamp: &KeyedStamp<ChatKey, CandidateStamp>,
        succeeded: bool,
    ) -> Option<Client> {
        let owner = stamp.tag().owner?;
        if stamp.tag().deleted || !self.current(stamp.key(), owner) {
            return None;
        }
        // A stale failure must not pause a newer accepted document revision.
        // Successful old publications still acknowledge their actual own revision.
        if !succeeded && self.stamps.get(stamp.key()) != Some(&(owner, stamp.tag().revision)) {
            return None;
        }
        self.clients.get(&owner).cloned()
    }
    pub(super) fn scope_of(&self, owner: EntityId) -> Option<nocterm_ai::history::ContextId> {
        self.owners
            .iter()
            .find_map(|(key, current)| (current.entity == Some(owner)).then_some(key.scope()))
    }
    pub(super) fn client(&self, id: EntityId) -> Option<Client> {
        self.clients.get(&id).cloned()
    }
    pub(super) fn clients(&self) -> Vec<Client> {
        self.clients.values().cloned().collect()
    }
    pub(super) fn contains_deleted(&self, id: &ChatKey) -> bool {
        self.deleted.contains(id)
    }
    pub(super) fn closing(&self) -> bool {
        self.closing
    }
    pub(super) fn stop(&mut self) {
        self.closing = true;
    }
}
