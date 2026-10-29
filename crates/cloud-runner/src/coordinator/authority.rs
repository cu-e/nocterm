//! Original job liveness, independent of the foreground view's retirement.
use super::{run::RunRecord, services::CoordinatorStorage};
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::{AuthenticatedCommandSource, CoordinatorError},
    journal::JournalError,
    tool::*,
};
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

#[derive(Clone, Default)]
pub(super) struct RunAuthorities {
    runs: Arc<Mutex<BTreeMap<RunBinding, Weak<RunRecord>>>>,
}
impl RunAuthorities {
    pub(super) fn register(&self, run: &Arc<RunRecord>) -> Result<(), CoordinatorError> {
        let mut runs = self.runs.lock();
        runs.retain(|_, run| run.strong_count() > 0);
        if !runs.contains_key(run.binding()) && runs.len() >= 4096 {
            return Err(CoordinatorError::Overloaded);
        }
        runs.insert(run.binding().clone(), Arc::downgrade(run));
        Ok(())
    }
    pub(super) fn stop_all(&self) -> Vec<RunBinding> {
        let runs = self
            .runs
            .lock()
            .values()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        runs.into_iter()
            .map(|run| {
                run.control().stop();
                run.binding().clone()
            })
            .collect()
    }
    fn get(&self, run: &RunBinding) -> Result<Arc<RunRecord>, CoordinatorError> {
        self.runs
            .lock()
            .get(run)
            .and_then(Weak::upgrade)
            .ok_or(CoordinatorError::Stopped)
    }
}
pub(super) struct BackgroundAuthority {
    storage: CoordinatorStorage,
    source: Arc<dyn AuthenticatedCommandSource>,
    timer: Arc<dyn ToolTimer>,
    runs: RunAuthorities,
}
impl BackgroundAuthority {
    pub(super) fn new(
        storage: CoordinatorStorage,
        source: Arc<dyn AuthenticatedCommandSource>,
        timer: Arc<dyn ToolTimer>,
        runs: RunAuthorities,
    ) -> Result<Self, CoordinatorError> {
        if storage.binding() != source.binding() {
            return Err(CoordinatorError::StaleSource);
        }
        Ok(Self {
            storage,
            source,
            timer,
            runs,
        })
    }
    fn validate(&self, command: &BackgroundCommand) -> Result<(), CoordinatorError> {
        let original = command.dispatch().intent();
        let binding = original.authority().run();
        self.source.check_current()?;
        if !self.storage.binding().contains(&binding.stream()) {
            return Err(CoordinatorError::StaleSource);
        }
        let run = self.runs.get(binding)?;
        run.control().check_background(self.source.as_ref())?;
        let writer = self
            .storage
            .io()
            .owned_writer(&binding.stream())
            .ok_or(JournalError::Fenced)?;
        if writer.epoch() != command.dispatch().commit().writer() {
            return Err(JournalError::Fenced.into());
        }
        self.storage.io().journal().require_current(&writer)?;
        let grants = run.grants();
        let grant = grants
            .tool(original.authority().grant())
            .ok_or(CoordinatorError::NeedsGrant)?;
        grant
            .authority()
            .require_same(original.authority())
            .map_err(ToolExecutionError::from)?;
        if grant.revocation().is_some()
            || !grant
                .policy()
                .capabilities()
                .permits(nocterm_ai::ToolCapability::Execution)
        {
            return Err(CoordinatorError::NeedsGrant);
        }
        run.control()
            .check_window(self.timer.now(), grant.policy().window())
    }
}
impl BackgroundCommandAuthority for BackgroundAuthority {
    fn check(&self, command: &BackgroundCommand) -> Result<(), ToolExecutionError> {
        self.validate(command)
            .map_err(|cause| ToolExecutionError::Execution(nocterm_core::SharedCause::new(cause)))
    }
}
