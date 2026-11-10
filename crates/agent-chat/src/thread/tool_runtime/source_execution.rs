//! A native runner retains its original account lease through every scheduler poll.
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_ai::{ExecCommand, ToolFailure, local_environment::LocalEnvironmentAccess};
use nocterm_workspace_ports::{ExpectedSession, TerminalCapability, TerminalEntry, TerminalError};
use std::{future::poll_fn, task::Poll, time::Duration};

pub(in crate::thread) struct SourceExecution {
    source: LocalEnvironmentAccess,
}
impl SourceExecution {
    pub(in crate::thread) fn capture(
        runtime: &nocterm_agent_runtime::RuntimeHandle,
    ) -> Result<Self, ToolFailure> {
        Ok(Self {
            source: runtime
                .captured_source()
                .map_err(|error| ToolFailure::from(error.to_string()))?,
        })
    }
    pub(in crate::thread) fn start(
        self,
        entry: TerminalEntry,
        expected: ExpectedSession,
        request: &ExecCommand,
        tools: &mut super::ToolRuntime,
        cx: &mut gpui::App,
    ) -> Result<(String, BoxFuture<'static, ()>), ToolFailure> {
        self.source
            .check_current()
            .map_err(|error| ToolFailure::from(error.to_string()))?;
        let executor = entry
            .capabilities
            .execution()
            .ok_or(TerminalError::Unsupported(
                TerminalCapability::StructuredExecution,
            ))
            .and_then(|port| port.executor(expected, cx))
            .map_err(|error| ToolFailure::from(error.to_string()))?;
        let program = nocterm_session::ExecRequest {
            program: request.program.clone(),
            args: request.args.clone(),
            stdin: request
                .stdin
                .as_ref()
                .map(|stdin| stdin.as_bytes().to_vec()),
        };
        let timer = cx
            .background_executor()
            .timer(Duration::from_millis(request.timeout_ms.unwrap_or(30_000)));
        let (id, run) = tools
            .start_command(request.terminal_id.clone(), executor, program, timer)?
            .into_run();
        Ok((id, self.guard(run)))
    }
    /// Each native future poll is admitted against the original owner. Already
    /// admitted transport work is cancelled by dropping the runner; no new poll
    /// can consume credentials from a replacement account. The chat watchdog
    /// records cancellation of the retained command separately.
    fn guard(self, mut run: BoxFuture<'static, ()>) -> BoxFuture<'static, ()> {
        poll_fn(move |cx| {
            if self.source.check_current().is_err() {
                return Poll::Ready(());
            }
            run.as_mut().poll(cx)
        })
        .boxed()
    }
}
