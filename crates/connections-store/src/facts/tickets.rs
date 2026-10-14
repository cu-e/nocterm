use super::policy::CountryPolicySnapshot;
use super::source::ProbeSourceLease;
use gpui::App;
use nocterm_connections_model::{profile_authority::ProfilePublication, profiles::ProfileId};
use nocterm_session::RemoteFs;
use std::sync::{Arc, Weak};
#[derive(Debug)]
pub(super) struct JobIdentity;
pub(super) type JobKey = Arc<JobIdentity>;
pub(super) fn job_key() -> JobKey {
    Arc::new(JobIdentity)
}
#[derive(Clone)]
pub(super) struct ProbeTicket {
    profile: ProfileId,
    incarnation: u64,
    endpoint: u64,
    icon: u64,
    session: Weak<dyn RemoteFs>,
    source: ProbeSourceLease,
    job: JobKey,
}
impl ProbeTicket {
    pub(super) fn new(
        profile: ProfileId,
        publication: &ProfilePublication,
        session: &Arc<dyn RemoteFs>,
        source: ProbeSourceLease,
        job: JobKey,
    ) -> Option<Self> {
        let stamp = publication.get(profile)?.stamp;
        Some(Self {
            profile,
            incarnation: stamp.incarnation,
            endpoint: stamp.endpoint,
            icon: stamp.icon_override,
            session: Arc::downgrade(session),
            source,
            job,
        })
    }
    pub(super) fn valid(&self, publication: &ProfilePublication, cx: &App) -> bool {
        self.source.is_current(cx)
            && publication.get(self.profile).is_some_and(|row| {
                self.session.upgrade().is_some()
                    && row.icon.is_none()
                    && row.stamp.incarnation == self.incarnation
                    && row.stamp.endpoint == self.endpoint
                    && row.stamp.icon_override == self.icon
            })
    }
    pub(super) fn profile(&self) -> ProfileId {
        self.profile
    }
    pub(super) fn job(&self) -> &JobKey {
        &self.job
    }
    pub(super) fn session(&self) -> &Weak<dyn RemoteFs> {
        &self.session
    }
}
#[derive(Clone)]
pub(super) struct CountryTicket {
    profile: ProfileId,
    incarnation: u64,
    endpoint: u64,
    country: u64,
    host: String,
    session: Weak<dyn RemoteFs>,
    source: ProbeSourceLease,
    policy: CountryPolicySnapshot,
    job: JobKey,
}
impl CountryTicket {
    pub(super) fn new(
        profile: ProfileId,
        publication: &ProfilePublication,
        session: &Arc<dyn RemoteFs>,
        source: ProbeSourceLease,
        policy: CountryPolicySnapshot,
        job: JobKey,
    ) -> Option<Self> {
        let row = publication.get(profile)?;
        Some(Self {
            profile,
            incarnation: row.stamp.incarnation,
            endpoint: row.stamp.endpoint,
            country: row.stamp.country_override,
            host: row.target.host.clone(),
            session: Arc::downgrade(session),
            source,
            policy,
            job,
        })
    }
    pub(super) fn valid(
        &self,
        publication: &ProfilePublication,
        policy: CountryPolicySnapshot,
        cx: &App,
    ) -> bool {
        self.source.is_current(cx)
            && policy.enabled()
            && policy == self.policy
            && self.session.upgrade().is_some()
            && publication.get(self.profile).is_some_and(|row| {
                row.country.is_none()
                    && row.target.host == self.host
                    && row.stamp.incarnation == self.incarnation
                    && row.stamp.endpoint == self.endpoint
                    && row.stamp.country_override == self.country
            })
    }
    pub(super) fn profile(&self) -> ProfileId {
        self.profile
    }
    pub(super) fn job(&self) -> &JobKey {
        &self.job
    }
    pub(super) fn source(&self) -> &ProbeSourceLease {
        &self.source
    }
    pub(super) fn session(&self) -> &Weak<dyn RemoteFs> {
        &self.session
    }
}
