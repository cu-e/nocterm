//! Concrete protocol graph and native worker are admitted before scheduling.
use super::{
    budget::SourceBudget,
    channel::{self, Receiver},
    commands::Commands,
    diagnostics::Diagnostics,
    driver::DriverTask,
    events,
    handler::Incoming,
    initialize::Restore,
    models::Models,
    native_connect::Cause,
    native_driver::{NativeDriver, ProcessOwner},
    native_process::Spawned,
    native_session::{Ready, Session},
    redaction::DiagnosticRedactor,
    rpc::Rpc,
    state::State,
};
use agent_client_protocol::managed::{Driver, Shared};
use nocterm_ai::{
    connection::{ManagedCommandsHandle, ManagedStartupHandle},
    history::source::{ManagedSourceFuture, SourceLifetime},
};
use std::sync::{Arc, Mutex};
pub(super) struct Launch {
    budget: SourceBudget,
    rules: Arc<DiagnosticRedactor>,
}
impl Launch {
    pub(super) fn new(budget: SourceBudget, rules: Arc<DiagnosticRedactor>) -> Self {
        Self { budget, rules }
    }
    pub(super) fn start(
        self,
        spawned: Spawned,
        startup: ManagedStartupHandle,
        lifetime: SourceLifetime,
    ) -> Result<Receiver<Ready>, Cause> {
        let budget = self.budget;
        let (mut child, group, secrets) = spawned.into_parts();
        let stdin = child.stdin.take().ok_or(Cause::Pipe("stdin"))?;
        let stdout = child.stdout.take().ok_or(Cause::Pipe("stdout"))?;
        let stderr = child.stderr.take().ok_or(Cause::Pipe("stderr"))?;
        let (driver, connection) = Driver::new(budget.clone())?;
        let (events, event_receiver) = events::channel(&budget)?;
        let (stop, stopped) = channel::bounded(budget.clone(), 1)?;
        let (ready, ready_receiver) = channel::bounded(budget.clone(), 1)?;
        let diagnostics = Shared::new(
            Diagnostics::new(self.rules, secrets, budget.clone()),
            &budget,
        )?;
        let models = Shared::new(Mutex::new(Models::new(budget.clone())), &budget)?;
        let state = State::new(
            lifetime.clone(),
            Restore::Unsupported,
            false,
            false,
            &budget,
        )?;
        let rpc = Rpc::new(
            connection,
            budget.clone(),
            diagnostics.clone(),
            models.clone(),
            state.clone(),
        );
        let commands = ManagedCommandsHandle::new_with_lifetime(
            budget.context(),
            lifetime.clone(),
            Commands::new(
                rpc.clone(),
                events.clone(),
                stop,
                group.clone(),
                lifetime.clone(),
            ),
        )?;
        let session = Session::new(&rpc, startup, commands, event_receiver, stopped, ready)?;
        let driver = DriverTask::new(
            driver,
            stdout,
            stdin,
            Incoming::new(events.clone(), models, state, budget.clone()),
            budget.clone(),
        );
        let ticket = budget.context().prepare_error::<Cause>(0)?;
        let process = ProcessOwner::new(child, stderr, group);
        let worker = NativeDriver::new(
            process,
            driver,
            session,
            events,
            diagnostics,
            ticket,
            budget,
        )?;
        let work = ManagedSourceFuture::new(lifetime, worker.run())?;
        std::thread::Builder::new()
            .name("nocterm-acp".into())
            .spawn(move || futures::executor::block_on(work))
            .map_err(Cause::Worker)?;
        Ok(ready_receiver)
    }
}
