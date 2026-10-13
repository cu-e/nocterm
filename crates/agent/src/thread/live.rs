//! Observation of commands typed into the live shell; no guessed exit status.
use super::{AgentThread, tools::unreachable};
use gpui_kit::Context;
use nocterm_ai::BridgeCall;
use nocterm_ui::ActiveAi as _;
use nocterm_workspace::{TerminalEntry, TerminalStatus, TextRequest};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Debug, thiserror::Error)]
enum TerminalOperationError {
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
    fn into_failure(self) -> nocterm_ai::ToolFailure {
        match self {
            Self::Detached | Self::Disconnected => unreachable(&self.to_string()),
            _ => self.to_string().into(),
        }
    }
}

impl AgentThread {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn run_command(
        &mut self,
        entry: TerminalEntry,
        expected: nocterm_workspace::ExpectedSession,
        request: nocterm_ai::RunCommand,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let before = entry
            .capabilities
            .text()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::TextRead,
            ))
            .and_then(|port| {
                port.read(
                    expected,
                    TextRequest {
                        max_lines: 2000,
                        max_bytes: 64 * 1024,
                        since_line: None,
                    },
                    cx,
                )
            });
        let since = match before {
            Ok(before) => before.next_line,
            Err(error) => {
                self.finish(call, Err(error.to_string().into()));
                return;
            }
        };
        let lease = match entry
            .capabilities
            .live()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::LiveCommands,
            ))
            .and_then(|port| port.begin(expected, &request.command, cx))
        {
            Ok(lease) => Arc::new(lease),
            Err(error) => {
                self.finish(call, Err(error.to_string().into()));
                return;
            }
        };
        self.tools
            .install_live(request.terminal_id.clone(), lease.clone());
        let id = request.terminal_id.clone();
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(30_000));
        let idle = Duration::from_millis(request.idle_ms.unwrap_or(1000));
        let guard = self.hold_operation();
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            let started = Instant::now();
            let mut changed = started;
            let mut generation = None;
            let result = loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let step = this.update(cx, |this, cx| {
                    if !lease.is_active() {
                        return Err(TerminalOperationError::Revoked);
                    }
                    if !this.session.accepts_updates() || !cx.ai_enabled() {
                        return Err(TerminalOperationError::Cancelled);
                    }
                    let (_, entry, _) = this
                        .resolved(cx)
                        .into_iter()
                        .find(|(value, _, _)| *value == id)
                        .ok_or(TerminalOperationError::Detached)?;
                    let info = entry
                        .capabilities
                        .state()
                        .info(cx)
                        .ok_or(nocterm_workspace::TerminalError::Gone)?;
                    if info.expected != expected {
                        return Err(TerminalOperationError::Terminal(
                            nocterm_workspace::TerminalError::StaleSession,
                        ));
                    }
                    if info.status != TerminalStatus::Connected {
                        return Err(TerminalOperationError::Disconnected);
                    }
                    if generation != Some(info.generation) {
                        generation = Some(info.generation);
                        changed = Instant::now();
                    }
                    let reason = if info.at_prompt == Some(true) {
                        Some("prompt_returned")
                    } else if started.elapsed() >= timeout {
                        Some("timeout")
                    } else if changed.elapsed() >= idle {
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
                            expected,
                            TextRequest {
                                max_lines: 2000,
                                max_bytes: 64 * 1024,
                                since_line: Some(since),
                            },
                            cx,
                        )?;
                    let mut payload = this.text_payload(tail, cx);
                    payload["completion"] = serde_json::json!(reason);
                    payload["exit_status"] = serde_json::Value::Null;
                    payload["completed"] = serde_json::json!(reason == "prompt_returned");
                    Ok(Some(payload))
                });
                match step {
                    Ok(Ok(Some(result))) => break Ok(result),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => break Err(error),
                    Err(_) => break Err(TerminalOperationError::ChatClosed),
                }
            };
            let _ = this.update(cx, |this, _| {
                this.tools.finish_live_if_same(&id, &lease);
            });
            drop(lease);
            let _ = this.update(cx, |this, cx| {
                this.finish(call, result.map_err(TerminalOperationError::into_failure));
                cx.notify();
            });
        })
        .detach();
    }
}
