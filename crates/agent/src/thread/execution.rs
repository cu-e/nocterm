//! Structured execution uses the executor of the exact attached session.
use super::AgentThread;
use gpui_kit::Context;
use nocterm_ai::{BridgeCall, TerminalCall, ToolFailure};
use nocterm_session::ExecRequest;
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use nocterm_workspace::TerminalEntry;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
pub(in crate::thread) mod jobs;

impl AgentThread {
    pub(super) fn execute_command(
        &mut self,
        entry: TerminalEntry,
        expected: nocterm_workspace::ExpectedSession,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let TerminalCall::ExecCommand(request) = &call.call else {
            return;
        };
        let executor = match entry
            .capabilities
            .execution()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::StructuredExecution,
            ))
            .and_then(|port| port.executor(expected, cx))
        {
            Ok(executor) => executor,
            Err(error) => {
                self.finish(call, Err(error.to_string().into()));
                return;
            }
        };
        let program = ExecRequest {
            program: request.program.clone(),
            args: request.args.clone(),
            stdin: request
                .stdin
                .as_ref()
                .map(|stdin| stdin.as_bytes().to_vec()),
        };
        let timer = cx
            .background_executor()
            .timer(jobs::timeout(request.timeout_ms));
        let started =
            match self
                .tools
                .start_command(request.terminal_id.clone(), executor, program, timer)
            {
                Ok(started) => started,
                Err(error) => {
                    self.finish(call, Err(error));
                    return;
                }
            };
        let (id, run) = started.into_run();
        cx.background_executor().spawn(run).detach();
        self.start_execution_watch(cx);
        let terminal = request.terminal_id.clone();
        let yield_ms = request.yield_ms.unwrap_or(1000);
        self.wait_command(terminal, id, yield_ms, call, cx);
    }

    pub(super) fn read_command(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        let TerminalCall::ReadCommand(request) = &call.call else {
            return;
        };
        let terminal = request.terminal_id.clone();
        let id = request.command_id.clone();
        self.wait_command(terminal, id, request.yield_ms.unwrap_or(0), call, cx);
    }

    pub(super) fn cancel_command(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        let TerminalCall::CancelCommand(request) = &call.call else {
            return;
        };
        let result = self
            .tools
            .cancel_command(&request.command_id, &request.terminal_id)
            .map(|state| self.command_payload(&request.command_id, state, cx));
        self.finish(call, result);
    }

    fn wait_command(
        &mut self,
        terminal: String,
        id: String,
        yield_ms: u64,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let ticket = self.session.ticket();
        let guard = self.hold_operation();
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            let until = Instant::now() + Duration::from_millis(yield_ms);
            loop {
                let result = this.update(cx, |this, cx| {
                    if !this.session.session_current(ticket)
                        || !this.session.accepts_updates()
                        || !cx.ai_enabled()
                    {
                        return Err(ToolFailure::from("Chat is unavailable."));
                    }
                    this.revoke_executions(cx);
                    let state = this.tools.command_snapshot(&id, &terminal)?;
                    if state.active() && Instant::now() < until {
                        return Ok(None);
                    }
                    Ok(Some(this.command_payload(&id, state, cx)))
                });
                match result {
                    Ok(Ok(None)) => {
                        cx.background_executor()
                            .timer(Duration::from_millis(25))
                            .await
                    }
                    Ok(Ok(Some(payload))) => {
                        let _ = this.update(cx, |this, cx| {
                            this.finish(call, Ok(payload));
                            cx.notify();
                        });
                        break;
                    }
                    Ok(Err(error)) => {
                        let _ = this.update(cx, |this, cx| {
                            this.finish(call, Err(error));
                            cx.notify();
                        });
                        break;
                    }
                    Err(_) => {
                        let _ = call.respond.send(Err("Chat was closed.".into()));
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn command_payload(
        &self,
        id: &str,
        state: super::tool_runtime::CommandSnapshot,
        cx: &gpui_kit::App,
    ) -> serde_json::Value {
        let redact = |text: String| {
            if cx
                .setting::<nocterm_ai::AiSettings>()
                .approval
                .redact_secrets
            {
                nocterm_ai::redact::redact(&text)
            } else {
                text
            }
        };
        serde_json::json!({
            "command_id": id, "state": state.state.as_str(),
            "stdout": redact(String::from_utf8_lossy(&state.stdout).into_owned()),
            "stderr": redact(state.exit.as_ref().map(|exit| exit.stderr.clone()).unwrap_or_default()),
            "exit_status": state.exit.as_ref().and_then(|exit| exit.status),
            "total_bytes": state.total_bytes, "truncated": state.total_bytes > state.stdout.len() as u64,
            "error": state.error.map(|error| nocterm_ai::redact::redact(&error)),
        })
    }

    pub(crate) fn revoke_executions(&mut self, cx: &gpui_kit::App) {
        if self.tools.active_commands().next().is_none() && self.tools.live_empty() {
            return;
        }
        let resolved = self.resolved(cx);
        let revoked = self
            .tools
            .active_commands()
            .filter_map(|(id, executor)| {
                let current = resolved
                    .iter()
                    .find(|(candidate, _, _)| candidate == id)
                    .and_then(|(_, entry, _)| {
                        entry.capabilities.state().info(cx).and_then(|info| {
                            entry
                                .capabilities
                                .execution()?
                                .executor(info.expected, cx)
                                .ok()
                        })
                    });
                (!current.is_some_and(|current| Arc::ptr_eq(&current, executor)))
                    .then_some(id.to_owned())
            })
            .collect::<Vec<_>>();
        let attached = resolved
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect::<Vec<_>>();
        self.tools.revoke_detached_live(&attached);
        for id in revoked {
            // All commands on a removed or replaced session lose their authority.
            self.tools.revoke_command_terminal(&id);
        }
    }

    fn start_execution_watch(&mut self, cx: &mut Context<Self>) {
        if self.tools.has_watch() {
            return;
        }
        self.tools.install_watch(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let active = this
                    .update(cx, |this, cx| this.on_execution_watch(cx))
                    .unwrap_or(false);
                if !active {
                    break;
                }
            }
        }));
    }

    fn on_execution_watch(&mut self, cx: &gpui_kit::App) -> bool {
        if !cx.ai_enabled() {
            self.cancel_execution();
        } else {
            self.revoke_executions(cx);
        }
        if self.tools.active_commands().next().is_none() {
            self.tools.finish_watch();
            false
        } else {
            true
        }
    }
    pub(super) fn cancel_execution(&mut self) {
        self.tools.cancel_execution();
    }
}
