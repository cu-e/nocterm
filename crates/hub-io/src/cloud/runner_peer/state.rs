//! One original live SDK runner and its real pending durable credential record.
use super::runtime::{ClockSample, RuntimeAuthority};
use super::{RunnerCredentialDeadlines, RunnerHubError, sdk_error};
use crate::cloud::runner_session_file::SessionRecord;
use nocterm_hub_client::cloud::CloudRunner;
use std::sync::Arc;

#[derive(Default)]
struct Rotation {
    phase: Phase,
}
#[derive(Default)]
enum Phase {
    #[default]
    Stable,
    Active(Arc<()>),
    Unknown,
}
impl Rotation {
    fn start(&mut self) -> Result<Arc<()>, RunnerHubError> {
        self.require_stable()?;
        let token = Arc::new(());
        self.phase = Phase::Active(token.clone());
        Ok(token)
    }
    fn returned(&mut self, token: &Arc<()>) -> Result<(), RunnerHubError> {
        self.require_active(token)?;
        self.phase = Phase::Stable;
        Ok(())
    }
    fn require_active(&self, token: &Arc<()>) -> Result<(), RunnerHubError> {
        if !matches!(&self.phase, Phase::Active(original) if Arc::ptr_eq(original, token)) {
            return Err(RunnerHubError::RefreshUnknown);
        }
        Ok(())
    }
    fn abandon(&mut self, token: &Arc<()>) -> bool {
        if matches!(&self.phase, Phase::Active(original) if Arc::ptr_eq(original, token)) {
            self.phase = Phase::Unknown;
            true
        } else {
            false
        }
    }
    fn require_known(&self) -> Result<(), RunnerHubError> {
        if matches!(self.phase, Phase::Unknown) {
            Err(RunnerHubError::RefreshUnknown)
        } else {
            Ok(())
        }
    }
    fn require_stable(&self) -> Result<(), RunnerHubError> {
        self.require_known()?;
        if matches!(self.phase, Phase::Active(_)) {
            Err(RunnerHubError::DurabilityPending)
        } else {
            Ok(())
        }
    }
}
#[derive(Default)]
pub(super) struct SessionOwner {
    runner: Option<CloudRunner>,
    pending: Option<Arc<SessionRecord>>,
    rotation: Rotation,
    generation: Arc<()>,
    authority: RuntimeAuthority,
}
impl SessionOwner {
    pub(super) fn has_runner(&self) -> bool {
        self.runner.is_some()
    }
    pub(super) fn install(
        &mut self,
        runner: CloudRunner,
        pending: Option<Arc<SessionRecord>>,
        sample: ClockSample,
    ) -> Result<(), RunnerHubError> {
        let (access, refresh) = runner.credential_deadlines().map_err(sdk_error)?;
        self.authority
            .accept(RunnerCredentialDeadlines { access, refresh }, sample)?;
        self.runner = Some(runner);
        self.pending = pending;
        self.generation = Arc::new(());
        Ok(())
    }
    pub(super) fn runtime(&mut self, sample: ClockSample) -> Result<CloudRunner, RunnerHubError> {
        self.rotation.require_known()?;
        let runner = self.runner.as_ref().ok_or(RunnerHubError::Unavailable)?;
        self.authority.check(sample)?;
        if let Err(error) = runner.check_source() {
            self.authority.fence_source();
            return Err(sdk_error(error));
        }
        Ok(runner.clone())
    }
    pub(super) fn ready(&mut self, sample: ClockSample) -> Result<CloudRunner, RunnerHubError> {
        let runner = self.runtime(sample)?;
        self.rotation.require_stable()?;
        if self.pending.is_some() {
            return Err(RunnerHubError::DurabilityPending);
        }
        Ok(runner)
    }
    pub(super) fn transport_ready(&mut self, sample: ClockSample) -> Result<bool, RunnerHubError> {
        self.runtime(sample)?;
        let now = self.authority.check(sample)?;
        Ok(self.rotation.require_stable().is_ok()
            && self.pending.is_none()
            && self.authority.access_live(now))
    }
    pub(super) fn capture(
        &mut self,
        sample: ClockSample,
    ) -> Result<(CloudRunner, Arc<()>), RunnerHubError> {
        let runner = self.ready(sample)?;
        let now = self.authority.check(sample)?;
        if !self.authority.access_live(now) {
            return Err(RunnerHubError::DurabilityPending);
        }
        Ok((runner, self.generation.clone()))
    }
    pub(super) fn check_generation(
        &mut self,
        generation: &Arc<()>,
        sample: ClockSample,
    ) -> Result<(), RunnerHubError> {
        self.runtime(sample)?;
        if !Arc::ptr_eq(generation, &self.generation) {
            return Err(RunnerHubError::TransportChanged);
        }
        self.ready(sample)?;
        Ok(())
    }
    pub(super) fn begin(
        &mut self,
        sample: ClockSample,
    ) -> Result<(CloudRunner, Arc<()>), RunnerHubError> {
        let runner = self.ready(sample)?;
        let token = self.rotation.start()?;
        self.generation = Arc::new(());
        Ok((runner, token))
    }
    pub(super) fn returned(
        &mut self,
        token: &Arc<()>,
        record: Arc<SessionRecord>,
        deadlines: RunnerCredentialDeadlines,
        sample: ClockSample,
    ) -> Result<(), RunnerHubError> {
        self.rotation.require_active(token)?;
        self.authority.accept(deadlines, sample)?;
        self.rotation.returned(token)?;
        self.pending = Some(record);
        Ok(())
    }
    pub(super) fn pending(&self) -> Option<Arc<SessionRecord>> {
        self.pending.clone()
    }
    pub(super) fn persisted(&mut self, record: &Arc<SessionRecord>) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, record))
        {
            self.pending = None;
            self.generation = Arc::new(());
            true
        } else {
            false
        }
    }
    pub(super) fn abandon(&mut self, token: &Arc<()>) -> bool {
        self.rotation.abandon(token)
    }
    pub(super) fn deadlines(
        &mut self,
        sample: ClockSample,
    ) -> Result<RunnerCredentialDeadlines, RunnerHubError> {
        self.runtime(sample)?;
        self.authority.deadlines()
    }
    pub(super) fn wait_millis(&mut self, sample: ClockSample) -> Result<u64, RunnerHubError> {
        self.runtime(sample)?;
        let now = self.authority.check(sample)?;
        let remaining = self.authority.deadlines()?.refresh - now;
        u64::try_from(remaining.min(100)).map_err(|_| RunnerHubError::HardExpired)
    }
    pub(super) fn retire(&mut self) {
        if let Some(runner) = self.runner.take() {
            runner.revoke_local();
        }
        self.pending.take();
        self.rotation.phase = Phase::Unknown;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runner_runtime_rotation_only_the_original_pending_owner_can_become_unknown() {
        let mut rotation = Rotation::default();
        let old = rotation.start().unwrap();
        assert!(rotation.require_known().is_ok());
        assert!(matches!(
            rotation.require_stable(),
            Err(RunnerHubError::DurabilityPending)
        ));
        rotation.returned(&old).unwrap();
        let current = rotation.start().unwrap();
        assert!(!rotation.abandon(&old));
        assert!(rotation.require_known().is_ok());
        assert!(rotation.abandon(&current));
        assert!(matches!(
            rotation.require_known(),
            Err(RunnerHubError::RefreshUnknown)
        ));
        assert!(rotation.start().is_err());
        assert!(
            SessionOwner::default()
                .runtime(super::super::runtime::now())
                .is_err()
        );
    }
}
