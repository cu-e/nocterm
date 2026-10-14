use super::{
    source::{ProbeSourceIdentity, ProbeSourceLease},
    tickets::ProbeTicket,
};
use nocterm_connections_model::{profile_authority::ProfilePublication, profiles::ProfileId};
use nocterm_session::RemoteFs;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
struct SessionDecision {
    session: Weak<dyn RemoteFs>,
    source: ProbeSourceIdentity,
    lifetime: ProbeSourceLease,
    binding: ProfileId,
    os: bool,
    country: CountryDecision,
}
#[derive(Clone, Copy)]
enum CountryDecision {
    Fresh,
    Attempted,
    Revoked,
}
pub(super) struct ProbeLedger {
    decisions: Vec<SessionDecision>,
    slots: HashMap<ProfileId, ProbeTicket>,
}
impl ProbeLedger {
    pub(super) fn new() -> Self {
        Self {
            decisions: Vec::new(),
            slots: HashMap::new(),
        }
    }
    fn index(&mut self, fs: &Arc<dyn RemoteFs>, source: &ProbeSourceLease) -> Option<usize> {
        self.decisions
            .retain(|entry| entry.session.strong_count() > 0);
        self.decisions.iter().position(|entry| {
            entry.source == source.identity()
                && entry
                    .session
                    .upgrade()
                    .is_some_and(|original| Arc::ptr_eq(&original, fs))
        })
    }
    pub(super) fn needs(
        &mut self,
        profile: ProfileId,
        fs: &Arc<dyn RemoteFs>,
        source: &ProbeSourceLease,
    ) -> (bool, bool) {
        self.index(fs, source).map_or((true, true), |index| {
            let entry = &self.decisions[index];
            (
                !entry.os,
                entry.binding != profile || !matches!(entry.country, CountryDecision::Attempted),
            )
        })
    }
    pub(super) fn admitted(
        &mut self,
        profile: ProfileId,
        fs: &Arc<dyn RemoteFs>,
        source: &ProbeSourceLease,
        country: bool,
    ) {
        let index = match self.index(fs, source) {
            Some(index) => index,
            None => {
                // A genuine newer epoch replaces its logical decision, never its physical jobs.
                self.decisions
                    .retain(|entry| entry.source.item() != source.identity().item());
                self.decisions.push(SessionDecision {
                    session: Arc::downgrade(fs),
                    source: source.identity(),
                    lifetime: source.clone(),
                    binding: profile,
                    os: false,
                    country: CountryDecision::Fresh,
                });
                self.decisions.len() - 1
            }
        };
        self.decisions[index].os = true;
        if country {
            self.decisions[index].binding = profile;
            self.decisions[index].country = CountryDecision::Attempted;
        }
    }
    pub(super) fn country_revoked(
        &mut self,
        profile: ProfileId,
        fs: &Weak<dyn RemoteFs>,
        source: &ProbeSourceLease,
    ) {
        if let Some(entry) = self.decisions.iter_mut().find(|entry| {
            entry.binding == profile
                && Weak::ptr_eq(&entry.session, fs)
                && entry.source == source.identity()
        }) {
            entry.country = CountryDecision::Revoked;
        }
    }
    pub(super) fn retained_sources(&self) -> usize {
        self.decisions.len()
    }
    pub(super) fn set_slot(&mut self, ticket: ProbeTicket) {
        self.slots.insert(ticket.profile(), ticket);
    }
    pub(super) fn finish(&mut self, ticket: &ProbeTicket) -> bool {
        if !self.slots.get(&ticket.profile()).is_some_and(|current| {
            Arc::ptr_eq(current.job(), ticket.job())
                && Weak::ptr_eq(current.session(), ticket.session())
        }) {
            return false;
        }
        self.slots.remove(&ticket.profile());
        true
    }
    pub(super) fn prune(&mut self, publication: &ProfilePublication, cx: &gpui::App) {
        self.decisions.retain(|decision| {
            decision.session.strong_count() > 0 && decision.lifetime.same_session_alive(cx)
        });
        self.slots.retain(|_, ticket| ticket.valid(publication, cx));
    }
}
