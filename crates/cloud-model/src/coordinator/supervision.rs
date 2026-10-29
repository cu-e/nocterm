//! Finite supervision limits and honest completion reports; no executor or clock.
use super::CoordinatorError;

#[derive(Clone, Copy, Debug)]
pub struct SupervisorLimits {
    commands: usize,
    urgent: usize,
    workers: usize,
    retirements: usize,
    native: usize,
}
impl SupervisorLimits {
    pub fn new(
        commands: usize,
        urgent: usize,
        workers: usize,
        retirements: usize,
        native: usize,
    ) -> Result<Self, CoordinatorError> {
        if [commands, urgent, workers, retirements, native]
            .into_iter()
            .any(|limit| limit == 0 || limit > 4096)
            || retirements < workers
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            commands,
            urgent,
            workers,
            retirements,
            native,
        })
    }
    pub fn commands(self) -> usize {
        self.commands
    }
    pub fn urgent(self) -> usize {
        self.urgent
    }
    pub fn workers(self) -> usize {
        self.workers
    }
    pub fn retirements(self) -> usize {
        self.retirements
    }
    pub fn native(self) -> usize {
        self.native
    }
}
impl Default for SupervisorLimits {
    fn default() -> Self {
        Self {
            commands: 64,
            urgent: 16,
            workers: 8,
            retirements: 16,
            native: 128,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupervisorPhase {
    Running,
    Draining,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub phase: SupervisorPhase,
    pub outstanding: usize,
    pub failed_native_tasks: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainReport {
    Drained,
    Failed {
        failed_native_tasks: usize,
    },
    Deadline {
        outstanding_tasks: usize,
        in_flight_writes: usize,
        failed_native_tasks: usize,
    },
}
