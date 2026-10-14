//! Raw facts and physically bounded jobs; views and native implementations are supplied.
use crate::observation::{LocationRequest, ObservationDeps};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter};
use nocterm_connections_model::{
    facts::Known,
    observation::{CountryCode, ObservationError},
    os::OsId,
    os_observation::OsObservationError,
    profile_authority::ProfilePublication,
    profiles::ProfileId,
};
use nocterm_session::{RemoteFs, Target};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};
use std::{collections::HashSet, sync::Arc};
mod admission;
mod authority;
mod flags;
mod jobs;
mod location;
mod policy;
mod prepared;
mod source;
mod tickets;
pub use flags::FlagState;
pub use jobs::MAX_FACT_JOBS;
pub use policy::{CountryPolicySnapshot, CountryPolicySource};
pub use prepared::FactsPrepared;
pub use source::{ProbeSourceAuthority, ProbeSourceIdentity, ProbeSourceLease};
use tickets::{CountryTicket, JobKey, ProbeTicket};
#[derive(Debug, thiserror::Error)]
pub enum FactsAdmissionError {
    #[error("original session source is no longer current")]
    StaleSource,
    #[error("saved profile is no longer present")]
    MissingProfile,
    #[error("session target differs from the saved profile")]
    EndpointMismatch,
    #[error("native observation capacity exhausted: need {needed}, available {available}")]
    Capacity { needed: usize, available: usize },
    #[error("fact publication is closing")]
    Closing,
}
pub struct ProbeRequest {
    profile: ProfileId,
    target: Target,
    fs: Arc<dyn RemoteFs>,
    source: ProbeSourceLease,
}
impl ProbeRequest {
    pub fn new(
        profile: ProfileId,
        target: Target,
        fs: Arc<dyn RemoteFs>,
        source: ProbeSourceLease,
    ) -> Self {
        Self {
            profile,
            target,
            fs,
            source,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    Admitted { os: bool, country: bool },
    Settled,
}
#[derive(Clone, Copy, Debug)]
pub enum FactsEvent {
    CapacityAvailable,
}
/// Immutable operational load: admitted native work and retained original-session decisions.
#[derive(Clone, Copy, Debug)]
pub struct FactWorkload {
    physical_jobs: usize,
    retained_sources: usize,
}
impl FactWorkload {
    pub fn physical_jobs(self) -> usize {
        self.physical_jobs
    }
    pub fn retained_sources(self) -> usize {
        self.retained_sources
    }
}
pub struct FactsService {
    known: Entity<Persisted<Known>>,
    observations: ObservationDeps,
    authority: authority::ProfileAuthority,
    probes: admission::ProbeLedger,
    locations: location::LocationAdmission,
    flags: flags::RawFlags,
    jobs: jobs::ObservationJobs,
    policy: policy::DetectionPolicy,
}
impl EventEmitter<FactsEvent> for FactsService {}
impl FactsService {
    pub fn os<'a>(&'a self, id: ProfileId, cx: &'a App) -> Option<&'a str> {
        self.known.read(cx).visible().os(id)
    }
    pub fn country<'a>(&'a self, id: ProfileId, cx: &'a App) -> Option<&'a str> {
        self.known.read(cx).visible().country(id)
    }
    pub fn flag(&self, code: CountryCode) -> Option<&FlagState> {
        self.flags.state(code)
    }
    pub fn available_jobs(&self) -> usize {
        self.jobs.available()
    }
    pub fn workload(&self) -> FactWorkload {
        FactWorkload {
            physical_jobs: MAX_FACT_JOBS - self.jobs.available(),
            retained_sources: self.probes.retained_sources(),
        }
    }
    pub fn persistence_error<'a>(&'a self, cx: &'a App) -> Option<&'a StoreError> {
        self.known.read(cx).error()
    }
    fn closing(&self, cx: &App) -> bool {
        self.known.read(cx).is_closing()
    }
    /// Observers are a prompt; every request/completion also invokes this actual carrier read.
    pub fn synchronize(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (ProfilePublication, CountryPolicySnapshot) {
        let (publication, forgotten, changed) = self.authority.synchronize(cx);
        let policy = self.policy.read(cx);
        self.probes.prune(&publication, cx);
        self.locations
            .prune(&publication, policy, &mut self.probes, cx);
        if changed && !self.closing(cx) {
            let ids: HashSet<_> = publication.iter().map(|(id, _)| id).collect();
            let _ = self.known.update(cx, |known, cx| {
                known.update_latest(
                    move |known| {
                        known.retain_profiles(&ids);
                        for id in forgotten {
                            known.forget_profile(id);
                        }
                    },
                    cx,
                )
            });
        }
        for (_, row) in publication.iter() {
            if let Some(code) = row
                .country
                .as_deref()
                .and_then(|code| CountryCode::new(code).ok())
            {
                self.flags
                    .request(code, self.observations.network_enabled());
            }
        }
        (publication, policy)
    }
    pub fn request_probe(
        &mut self,
        request: ProbeRequest,
        cx: &mut Context<Self>,
    ) -> Result<ProbeOutcome, FactsAdmissionError> {
        let (publication, policy) = self.synchronize(cx);
        if self.closing(cx) {
            return Err(FactsAdmissionError::Closing);
        }
        if !request.source.is_current(cx) {
            return Err(FactsAdmissionError::StaleSource);
        }
        let row = publication
            .get(request.profile)
            .ok_or(FactsAdmissionError::MissingProfile)?;
        if row.target != request.target {
            return Err(FactsAdmissionError::EndpointMismatch);
        }
        let (fresh_os, fresh_country) =
            self.probes
                .needs(request.profile, &request.fs, &request.source);
        let os = fresh_os && row.icon.is_none();
        let country = fresh_country
            && self.observations.network_enabled()
            && policy.enabled()
            && row.country.is_none()
            && self.known.read(cx).visible().located_host(request.profile)
                != Some(request.target.host.as_str());
        let needed = usize::from(os) + usize::from(country);
        let keys = match self
            .jobs
            .reserve_probe(needed, self.flags.pending().is_some())
        {
            Ok(keys) => keys,
            Err(error) => {
                self.drain_flags(cx);
                return Err(error);
            }
        };
        // Capacity refusal above changes no capability decision/lease. All reservations are atomic.
        self.probes
            .admitted(request.profile, &request.fs, &request.source, country);
        let mut keys = keys.into_iter();
        if os
            && let Some(ticket) = keys.next().and_then(|job| {
                ProbeTicket::new(
                    request.profile,
                    &publication,
                    &request.fs,
                    request.source.clone(),
                    job,
                )
            })
        {
            self.probes.set_slot(ticket.clone());
            self.start_os(ticket, request.fs.clone(), cx);
        }
        if country
            && let Some(ticket) = keys.next().and_then(|job| {
                CountryTicket::new(
                    request.profile,
                    &publication,
                    &request.fs,
                    request.source.clone(),
                    policy,
                    job,
                )
            })
        {
            self.locations.set_slot(ticket.clone());
            self.locations.refresh_cache(self.known.read(cx).visible());
            let request = LocationRequest::new(
                &request.target.host,
                self.locations.cache(),
                self.observations.clone(),
            );
            self.start_country(ticket, request, cx);
        }
        self.drain_flags(cx);
        Ok(if needed == 0 {
            ProbeOutcome::Settled
        } else {
            ProbeOutcome::Admitted { os, country }
        })
    }
    /// A global country-key route: it needs neither a session nor detect-country policy.
    pub fn request_flag(
        &mut self,
        code: CountryCode,
        cx: &mut Context<Self>,
    ) -> Result<(), FactsAdmissionError> {
        self.synchronize(cx);
        if self.closing(cx) {
            return Err(FactsAdmissionError::Closing);
        }
        self.flags
            .request(code, self.observations.network_enabled());
        self.drain_flags(cx);
        cx.notify();
        Ok(())
    }
    /// Weak source reprojection found no eligible external head; yield its lane to flags.
    pub fn yield_probe_turn(&mut self, cx: &mut Context<Self>) {
        self.synchronize(cx);
        self.jobs.yield_probe_turn();
        self.drain_flags(cx);
    }
    pub fn policy_changed(&mut self, cx: &mut Context<Self>) {
        self.synchronize(cx);
        self.progress(cx);
    }
    fn start_os(&self, ticket: ProbeTicket, fs: Arc<dyn RemoteFs>, cx: &mut Context<Self>) {
        let future = self.observations.os().detect(fs);
        let work = self.jobs.executor().spawn(future);
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| this.finish_os(ticket, result, cx));
        })
        .detach();
    }
    fn start_country(
        &self,
        ticket: CountryTicket,
        request: LocationRequest,
        cx: &mut Context<Self>,
    ) {
        let work = self.jobs.executor().spawn(async move { request.locate() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| this.finish_country(ticket, result, cx));
        })
        .detach();
    }
    fn finish_os(
        &mut self,
        ticket: ProbeTicket,
        result: Result<Option<OsId>, OsObservationError>,
        cx: &mut Context<Self>,
    ) {
        let released = self.jobs.release(ticket.job());
        let (publication, _) = self.synchronize(cx);
        if !released || self.closing(cx) {
            return;
        }
        if self.probes.finish(&ticket) && ticket.valid(&publication, cx) {
            match result {
                Ok(Some(os)) => {
                    let id = ticket.profile();
                    let _ = self.known.update(cx, |known, cx| {
                        known.update_latest(move |known| known.set_os(id, os), cx)
                    });
                }
                Ok(None) => {}
                Err(error) => tracing::debug!(%error,"remote OS observation failed"),
            }
        }
        self.progress(cx);
    }
    fn finish_country(
        &mut self,
        ticket: CountryTicket,
        result: Result<(String, Option<(String, CountryCode)>), ObservationError>,
        cx: &mut Context<Self>,
    ) {
        let released = self.jobs.release(ticket.job());
        let (publication, policy) = self.synchronize(cx);
        if !released || self.closing(cx) {
            return;
        }
        if self.locations.finish(&ticket) && ticket.valid(&publication, policy, cx) {
            match result {
                Ok((host, found)) => {
                    let code = found.as_ref().map(|(_, code)| *code);
                    let id = ticket.profile();
                    let _ = self.known.update(cx, |known, cx| {
                        known.update_latest(move |known| known.accept_country(id, host, found), cx)
                    });
                    if let Some(code) = code {
                        self.flags
                            .request(code, self.observations.network_enabled());
                    }
                }
                Err(error) => tracing::debug!(%error,"country observation failed"),
            }
        }
        self.progress(cx);
    }
    fn drain_flags(&mut self, cx: &mut Context<Self>) {
        if self.closing(cx) {
            return;
        }
        while let Some(code) = self.flags.pending() {
            let Some(job) = self.jobs.reserve_flag() else {
                break;
            };
            self.flags.loading(code, job.clone());
            let observations = self.observations.clone();
            let work = self
                .jobs
                .executor()
                .spawn(async move { observations.flags().load(code) });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| this.finish_flag(code, job, result, cx));
            })
            .detach();
        }
    }
    fn finish_flag(
        &mut self,
        code: CountryCode,
        job: JobKey,
        result: Result<Option<Vec<u8>>, ObservationError>,
        cx: &mut Context<Self>,
    ) {
        let released = self.jobs.release(&job);
        self.synchronize(cx);
        if !released || self.closing(cx) {
            return;
        }
        self.flags.finish(code, &job, result);
        self.progress(cx);
    }
    fn progress(&mut self, cx: &mut Context<Self>) {
        if self.closing(cx) {
            return;
        }
        cx.notify();
        cx.emit(FactsEvent::CapacityAvailable);
        self.drain_flags(cx);
    }
}
#[cfg(test)]
mod tests;
