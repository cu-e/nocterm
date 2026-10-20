//! Structured execution uses the executor of the exact attached session.
use super::AgentThread;
use nocterm_ai::{BridgeCall, ToolFailure};
use std::time::{Duration, Instant};
pub(in crate::thread) mod jobs;

/// Read waits retain the original ticket and never take a replacement executor.
pub(super) struct CommandWaitOperation {
    terminal: String,
    id: String,
    ticket: nocterm_ai::session::Ticket,
    yield_time: Duration,
    until: Instant,
    _guard: super::session_driver::OperationGuard,
}
impl CommandWaitOperation {
    pub(super) fn new(
        terminal: String,
        id: String,
        yield_ms: u64,
        ticket: nocterm_ai::session::Ticket,
        guard: super::session_driver::OperationGuard,
    ) -> Self {
        Self {
            terminal,
            id,
            ticket,
            yield_time: Duration::from_millis(yield_ms),
            until: Instant::now(),
            _guard: guard,
        }
    }
    pub(super) fn ticket(&self) -> nocterm_ai::session::Ticket {
        self.ticket
    }
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn terminal(&self) -> &str {
        &self.terminal
    }
    pub(super) fn frame(
        &self,
        state: super::tool_runtime::CommandSnapshot,
        redact: bool,
    ) -> Result<Option<serde_json::Value>, ToolFailure> {
        if state.active() && Instant::now() < self.until {
            return Ok(None);
        }
        Ok(Some(super::tool_runtime::presentation::command_payload(
            &self.id, state, redact,
        )))
    }
    pub(super) async fn run(
        mut self,
        call: BridgeCall,
        thread: &gpui::WeakEntity<AgentThread>,
        cx: &mut gpui::AsyncApp,
    ) {
        self.until = Instant::now() + self.yield_time;
        loop {
            match thread.update(cx, |thread, cx| thread.poll_command_wait(&self, cx)) {
                Ok(Ok(None)) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(25))
                        .await
                }
                Ok(Ok(Some(payload))) => {
                    let _ = thread.update(cx, |thread, cx| {
                        thread.on_tool_completion(call, Ok(payload), cx)
                    });
                    break;
                }
                Ok(Err(error)) => {
                    let _ = thread.update(cx, |thread, cx| {
                        thread.on_tool_completion(call, Err(error), cx)
                    });
                    break;
                }
                Err(_) => {
                    let _ = call.respond.send(Err("Chat was closed.".into()));
                    break;
                }
            }
        }
    }
}
/// Its retained Task is owned by CommandJobs, and cancellation drops that task.
pub(super) struct ExecutionWatchOperation;
impl ExecutionWatchOperation {
    pub(super) async fn run(self, thread: &gpui::WeakEntity<AgentThread>, cx: &mut gpui::AsyncApp) {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            if !thread
                .update(cx, |thread, cx| thread.on_execution_watch(cx))
                .unwrap_or(false)
            {
                break;
            }
        }
    }
}
