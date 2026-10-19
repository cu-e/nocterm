//! Observation of commands typed into the live shell; no guessed exit status.
use super::{AgentThread, session_driver::OperationGuard, tools::unreachable};
use gpui_kit::{App, AsyncApp, WeakEntity};
use nocterm_ai::BridgeCall;

use nocterm_workspace::{TerminalEntry, TerminalStatus, TextRequest};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Debug, thiserror::Error)]
pub(super) enum TerminalOperationError {
    #[error("{0}")]
    Terminal(#[from] nocterm_workspace::TerminalError),
    #[error("Live command ownership was revoked by user input or reconnection.")]
    Revoked,
    #[error("Command observation cancelled.")]
    Cancelled,
    #[error("Terminal was detached or closed.")]
    Detached,
    #[error("Terminal is no longer connected.")]
    Disconnected,
    #[error("Chat was closed.")]
    ChatClosed,
}
impl TerminalOperationError {
    pub(super) fn into_failure(self) -> nocterm_ai::ToolFailure {
        match self {
            Self::Detached | Self::Disconnected => unreachable(&self.to_string()),
            _ => self.to_string().into(),
        }
    }
}

/// A live observation owns its cursor, timing and original lease, not the chat.
pub(super) struct LiveCommandOperation {
    id: String,
    expected: nocterm_workspace::ExpectedSession,
    lease: Arc<nocterm_workspace::LiveCommandLease>,
    since: u64,
    timeout: Duration,
    idle: Duration,
    started: Instant,
    changed: Instant,
    generation: Option<u64>,
    guard: Option<OperationGuard>,
}
impl LiveCommandOperation {
    pub(super) fn acquire(
        entry: TerminalEntry,
        expected: nocterm_workspace::ExpectedSession,
        request: nocterm_ai::RunCommand,
        cx: &mut App,
    ) -> Result<Self, TerminalOperationError> {
        let since = entry
            .capabilities
            .text()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::TextRead,
            ))?
            .read(
                expected,
                TextRequest {
                    max_lines: 2000,
                    max_bytes: 64 * 1024,
                    since_line: None,
                },
                cx,
            )?
            .next_line;
        let lease = Arc::new(
            entry
                .capabilities
                .live()
                .ok_or(nocterm_workspace::TerminalError::Unsupported(
                    nocterm_workspace::TerminalCapability::LiveCommands,
                ))?
                .begin(expected, &request.command, cx)?,
        );
        let started = Instant::now();
        Ok(Self {
            id: request.terminal_id,
            expected,
            lease,
            since,
            timeout: Duration::from_millis(request.timeout_ms.unwrap_or(30_000)),
            idle: Duration::from_millis(request.idle_ms.unwrap_or(1000)),
            started,
            changed: started,
            generation: None,
            guard: None,
        })
    }
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn lease(&self) -> Arc<nocterm_workspace::LiveCommandLease> {
        self.lease.clone()
    }
    pub(super) fn hold(&mut self, guard: OperationGuard) {
        self.guard = Some(guard);
    }
    pub(super) fn check_active(&self, available: bool) -> Result<(), TerminalOperationError> {
        if !self.lease.is_active() {
            return Err(TerminalOperationError::Revoked);
        }
        if !available {
            return Err(TerminalOperationError::Cancelled);
        }
        Ok(())
    }
    pub(super) fn poll(
        &mut self,
        entry: Option<TerminalEntry>,
        redact: bool,
        cx: &App,
    ) -> Result<Option<(serde_json::Value, usize)>, TerminalOperationError> {
        let entry = entry.ok_or(TerminalOperationError::Detached)?;
        let info = entry
            .capabilities
            .state()
            .info(cx)
            .ok_or(nocterm_workspace::TerminalError::Gone)?;
        if info.expected != self.expected {
            return Err(nocterm_workspace::TerminalError::StaleSession.into());
        }
        if info.status != TerminalStatus::Connected {
            return Err(TerminalOperationError::Disconnected);
        }
        if self.generation != Some(info.generation) {
            self.generation = Some(info.generation);
            self.changed = Instant::now();
        }
        let reason = if info.at_prompt == Some(true) {
            Some("prompt_returned")
        } else if self.started.elapsed() >= self.timeout {
            Some("timeout")
        } else if self.changed.elapsed() >= self.idle {
            Some("output_idle")
        } else {
            None
        };
        let Some(reason) = reason else {
            return Ok(None);
        };
        let tail = entry
            .capabilities
            .text()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::TextRead,
            ))?
            .read(
                self.expected,
                TextRequest {
                    max_lines: 2000,
                    max_bytes: 64 * 1024,
                    since_line: Some(self.since),
                },
                cx,
            )?;
        let (mut payload, bytes) = super::tool_runtime::presentation::text_payload(tail, redact);
        payload["completion"] = serde_json::json!(reason);
        payload["exit_status"] = serde_json::Value::Null;
        payload["completed"] = serde_json::json!(reason == "prompt_returned");
        Ok(Some((payload, bytes)))
    }
    pub(super) async fn run(
        mut self,
        call: BridgeCall,
        thread: &WeakEntity<AgentThread>,
        cx: &mut AsyncApp,
    ) {
        self.started = Instant::now();
        self.changed = self.started;
        let result = loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            match thread.update(cx, |thread, cx| thread.poll_live_command(&mut self, cx)) {
                Ok(Ok(Some(payload))) => break Ok(payload),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => break Err(error),
                Err(_) => break Err(TerminalOperationError::ChatClosed),
            }
        };
        let Self {
            id,
            lease,
            guard: _guard,
            ..
        } = self;
        let _ = thread.update(cx, |thread, cx| {
            thread.on_live_command_complete(id, lease, call, result, cx)
        });
    }
}
