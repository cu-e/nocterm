//! Live observation preserves prompt semantics and never invents an exit status.
use super::{physical::PhysicalTerminal, presentation, projection::LiveOwner};
use nocterm_cloud_model::{
    identity::Timestamp,
    tool::{ScopedToolRequest, ToolAuthority, ToolEffectStatus, ToolExecutionError, ToolTimer},
};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

pub(super) struct LiveObservation {
    entry: Arc<Mutex<PhysicalTerminal>>,
    since: u64,
    started: Timestamp,
    changed: Timestamp,
    generation: Option<u64>,
    timeout: u64,
    idle: u64,
    owner: LiveOwner,
}
impl LiveObservation {
    pub(super) fn new(
        entry: Arc<Mutex<PhysicalTerminal>>,
        since: u64,
        call: &nocterm_ai::RunCommand,
        now: Timestamp,
        owner: LiveOwner,
    ) -> Self {
        Self {
            entry,
            since,
            started: now,
            changed: now,
            generation: None,
            timeout: call.timeout_ms.unwrap_or(30_000),
            idle: call.idle_ms.unwrap_or(1000),
            owner,
        }
    }
    pub(super) async fn observe(
        mut self,
        request: &ScopedToolRequest,
        authority: &dyn ToolAuthority,
        timer: &dyn ToolTimer,
    ) -> Result<(serde_json::Value, ToolEffectStatus), ToolExecutionError> {
        loop {
            timer.delay(Duration::from_millis(100)).await;
            authority.check(request)?;
            let now = timer.now();
            if now < self.changed || now < self.started {
                return Err(ToolExecutionError::Revoked);
            }
            let terminal = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
            terminal.check(request.permit().intent().authority())?;
            if self.generation != Some(terminal.generation()) {
                self.generation = Some(terminal.generation());
                self.changed = now;
            }
            let reason = if terminal.prompt_returned(&self.owner) {
                Some("prompt_returned")
            } else if now.elapsed_since(self.started).unwrap_or_default() >= self.timeout {
                Some("timeout")
            } else if now.elapsed_since(self.changed).unwrap_or_default() >= self.idle {
                Some("output_idle")
            } else {
                None
            };
            if let Some(reason) = reason {
                let mut output = presentation::text(terminal.read(2000, Some(self.since)));
                output["completion"] = serde_json::json!(reason);
                output["completed"] = serde_json::json!(reason == "prompt_returned");
                output["exit_status"] = serde_json::Value::Null;
                let effect = if reason == "prompt_returned" {
                    ToolEffectStatus::Observed
                } else {
                    ToolEffectStatus::Unknown
                };
                return Ok((output, effect));
            }
        }
    }
}
impl Drop for LiveObservation {
    fn drop(&mut self) {
        self.entry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .end_live(&self.owner);
    }
}
