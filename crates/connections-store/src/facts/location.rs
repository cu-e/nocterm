use super::{admission::ProbeLedger, policy::CountryPolicySnapshot, tickets::CountryTicket};
use nocterm_connections_model::{
    facts::Known, profile_authority::ProfilePublication, profiles::ProfileId,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Weak},
};
pub(super) struct LocationAdmission {
    slots: HashMap<ProfileId, CountryTicket>,
    cache: Arc<BTreeMap<String, String>>,
}
impl LocationAdmission {
    pub(super) fn new(known: &Known) -> Self {
        Self {
            slots: HashMap::new(),
            cache: Arc::new(known.geoip().clone()),
        }
    }
    pub(super) fn cache(&self) -> Arc<BTreeMap<String, String>> {
        self.cache.clone()
    }
    pub(super) fn refresh_cache(&mut self, known: &Known) {
        if self.cache.as_ref() != known.geoip() {
            self.cache = Arc::new(known.geoip().clone());
        }
    }
    pub(super) fn set_slot(&mut self, ticket: CountryTicket) {
        self.slots.insert(ticket.profile(), ticket);
    }
    pub(super) fn finish(&mut self, ticket: &CountryTicket) -> bool {
        if !self.slots.get(&ticket.profile()).is_some_and(|current| {
            Arc::ptr_eq(current.job(), ticket.job())
                && Weak::ptr_eq(current.session(), ticket.session())
        }) {
            return false;
        }
        self.slots.remove(&ticket.profile());
        true
    }
    pub(super) fn prune(
        &mut self,
        publication: &ProfilePublication,
        policy: CountryPolicySnapshot,
        probes: &mut ProbeLedger,
        cx: &gpui::App,
    ) {
        self.slots.retain(|_, ticket| {
            let valid = ticket.valid(publication, policy, cx);
            if !valid {
                probes.country_revoked(ticket.profile(), ticket.session(), ticket.source());
            }
            valid
        });
    }
}
