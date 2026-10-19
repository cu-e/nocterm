//! One lazy native cleanup attempt shared by normal close and final capture.
use super::startup::{CleanupError, Completion};
use futures::FutureExt as _;
use nocterm_ai::AgentCommands;
use std::sync::{Arc, Mutex};

pub(super) struct ProcessCleanup {
    commands: Arc<dyn AgentCommands>,
    completion: Mutex<Completion>,
}
impl ProcessCleanup {
    pub(super) fn new(commands: Arc<dyn AgentCommands>) -> Self {
        Self {
            completion: Mutex::new(Self::attempt(commands.clone())),
            commands,
        }
    }
    pub(super) fn owns(&self, commands: &Arc<dyn AgentCommands>) -> bool {
        Arc::ptr_eq(&self.commands, commands)
    }
    pub(super) fn commands(&self) -> &Arc<dyn AgentCommands> {
        &self.commands
    }
    pub(super) fn join(&self) -> Completion {
        self.completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    pub(super) fn capture(&self) -> Completion {
        let mut completion = self
            .completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Failed ordinary cleanup retains its process owner for one final retry.
        // Pending or successful attempts are joined, never invoked a second time.
        if completion.peek().is_some_and(Result::is_err) {
            *completion = Self::attempt(self.commands.clone());
        }
        completion.clone()
    }
    fn attempt(commands: Arc<dyn AgentCommands>) -> Completion {
        async move {
            // Some ports perform side effects while constructing their future.
            // The shared future must own invocation as well as its acknowledgement.
            commands
                .shutdown_gracefully()
                .await
                .map_err(CleanupError::process)
        }
        .boxed()
        .shared()
    }
}
