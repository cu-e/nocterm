//! Bounded original grant facts and generation-bound publication; no callbacks under its lock.
use super::*;
use nocterm_cloud_model::{
    authority::StreamBinding,
    identity::{GrantEpoch, GrantId},
    journal::CommandAcceptance,
};

pub(super) struct InstallTicket {
    provider: NamespaceId,
    reference: GrantReference,
    generation: u64,
}
struct GrantFact {
    stream: StreamBinding,
    acceptance: CommandAcceptance,
    provider: NamespaceId,
    reference: GrantReference,
    generation: u64,
    pending: bool,
}
struct ActiveProvider {
    reference: GrantReference,
    lease: ProviderLease,
    models: Vec<NamespaceId>,
    expires: Timestamp,
}
impl ActiveProvider {
    /// A renewed lease may outlive the offer it was installed from.
    fn until(&self) -> Timestamp {
        self.lease
            .horizon()
            .map_or(self.expires, |renewed| renewed.max(self.expires))
    }
}
struct ProviderSlot {
    generation: u64,
    active: Option<ActiveProvider>,
}
pub(super) struct CatalogState {
    maximum: usize,
    next: u64,
    observed: Option<Timestamp>,
    healthy: bool,
    grants: BTreeMap<GrantId, GrantFact>,
    revoked: BTreeMap<GrantId, GrantEpoch>,
    providers: BTreeMap<NamespaceId, ProviderSlot>,
}
impl CatalogState {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            next: 0,
            observed: None,
            healthy: true,
            grants: BTreeMap::new(),
            revoked: BTreeMap::new(),
            providers: BTreeMap::new(),
        }
    }
    pub(super) fn observe(&mut self, now: Timestamp) -> bool {
        if self.observed.is_some_and(|previous| now < previous) {
            self.healthy = false;
        }
        self.observed = Some(now);
        self.healthy
    }
    pub(super) fn begin(
        &mut self,
        command: &CommandAcknowledgement,
        offer: &ProviderOffer,
    ) -> Result<Option<InstallTicket>, CoordinatorError> {
        if !self.healthy {
            return Err(CoordinatorError::NeedsProviderAccess);
        }
        let reference = offer.reference();
        if self
            .revoked
            .get(&reference.id())
            .is_some_and(|epoch| *epoch >= reference.epoch())
        {
            return Err(CoordinatorError::NeedsProviderAccess);
        }
        if let Some(old) = self.grants.get(&reference.id()) {
            if old.stream != *command.stream()
                || old.provider != *offer.provider()
                || old.reference.epoch() > reference.epoch()
            {
                return Err(CoordinatorError::Conflict);
            }
            if old.reference.epoch() == reference.epoch() {
                if old.acceptance != *command.acceptance() || old.pending {
                    return Err(CoordinatorError::Conflict);
                }
                if self
                    .providers
                    .get(offer.provider())
                    .and_then(|slot| slot.active.as_ref())
                    .is_some_and(|active| active.reference == reference)
                {
                    return Ok(None);
                }
                return Err(CoordinatorError::NeedsProviderAccess);
            }
        } else if !self.revoked.contains_key(&reference.id()) && self.fact_count() >= self.maximum {
            return Err(CoordinatorError::Overloaded);
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or(CoordinatorError::Overloaded)?;
        let generation = self.next;
        self.grants.insert(
            reference.id(),
            GrantFact {
                stream: command.stream().clone(),
                acceptance: command.acceptance().clone(),
                provider: offer.provider().clone(),
                reference,
                generation,
                pending: true,
            },
        );
        self.providers
            .entry(offer.provider().clone())
            .and_modify(|slot| slot.generation = generation)
            .or_insert(ProviderSlot {
                generation,
                active: None,
            });
        Ok(Some(InstallTicket {
            provider: offer.provider().clone(),
            reference,
            generation,
        }))
    }
    pub(super) fn abort(&mut self, ticket: &InstallTicket) {
        if let Some(fact) = self.grants.get_mut(&ticket.reference.id())
            && fact.generation == ticket.generation
        {
            fact.pending = false;
        }
    }
    pub(super) fn commit(
        &mut self,
        ticket: &InstallTicket,
        offer: &ProviderOffer,
        lease: ProviderLease,
    ) -> Result<Option<ProviderLease>, CoordinatorError> {
        if !self.healthy
            || self
                .revoked
                .get(&ticket.reference.id())
                .is_some_and(|epoch| *epoch >= ticket.reference.epoch())
        {
            return Err(CoordinatorError::NeedsProviderAccess);
        }
        let fact = self
            .grants
            .get_mut(&ticket.reference.id())
            .ok_or(CoordinatorError::Conflict)?;
        let slot = self
            .providers
            .get_mut(&ticket.provider)
            .ok_or(CoordinatorError::Conflict)?;
        if !fact.pending
            || fact.generation != ticket.generation
            || slot.generation != ticket.generation
        {
            return Err(CoordinatorError::Conflict);
        }
        fact.pending = false;
        let old = slot.active.replace(ActiveProvider {
            reference: ticket.reference,
            lease,
            models: offer.models().to_vec(),
            expires: offer.expires(),
        });
        Ok(old.map(|active| active.lease))
    }
    pub(super) fn model(
        &self,
        selection: &ModelSelection,
        now: Timestamp,
    ) -> Result<ProviderLease, EngineError> {
        if !self.healthy {
            return Err(EngineError::NeedsProviderAccess);
        }
        let active = self
            .providers
            .get(selection.provider())
            .and_then(|slot| slot.active.as_ref())
            .ok_or(EngineError::NeedsProviderAccess)?;
        if now >= active.until() || !active.models.contains(selection.model()) {
            return Err(EngineError::NeedsProviderAccess);
        }
        Ok(active.lease.clone())
    }
    pub(super) fn revoke(&mut self, reference: GrantReference) -> Vec<ProviderLease> {
        if !self.grants.contains_key(&reference.id())
            && !self.revoked.contains_key(&reference.id())
            && self.fact_count() >= self.maximum
        {
            return self.retire();
        }
        self.revoked
            .entry(reference.id())
            .and_modify(|epoch| *epoch = (*epoch).max(reference.epoch()))
            .or_insert(reference.epoch());
        if let Some(fact) = self.grants.get_mut(&reference.id())
            && fact.reference == reference
        {
            fact.pending = false;
        }
        let mut retired = Vec::new();
        for slot in self.providers.values_mut() {
            if slot
                .active
                .as_ref()
                .is_some_and(|active| active.reference == reference)
                && let Some(active) = slot.active.take()
            {
                retired.push(active.lease);
            }
        }
        retired
    }
    pub(super) fn retire(&mut self) -> Vec<ProviderLease> {
        self.healthy = false;
        self.providers
            .values_mut()
            .filter_map(|slot| slot.active.take().map(|active| active.lease))
            .collect()
    }
    fn fact_count(&self) -> usize {
        self.grants.len()
            + self
                .revoked
                .keys()
                .filter(|id| !self.grants.contains_key(*id))
                .count()
    }
}
