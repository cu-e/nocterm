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
impl AgentThread {
    pub(super) fn run_command(
        &mut self,
        entry: TerminalEntry,
        request: nocterm_ai::RunCommand,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let before = entry.access.read(
            TextRequest {
                max_lines: 2000,
                max_bytes: 64 * 1024,
                since_line: None,
            },
            cx,
        );
        let since = match before {
            Ok(before) => before.next_line,
            Err(error) => {
                let _ = call.respond.send(Err(error));
                return;
            }
        };
        let lease = match entry.access.begin_live_command(&request.command, cx) {
            Ok(lease) => Arc::new(lease),
            Err(error) => {
                let _ = call.respond.send(Err(error));
                return;
            }
        };
        self.live_commands
            .insert(request.terminal_id.clone(), lease.clone());
        let epoch = self.epoch;
        let id = request.terminal_id.clone();
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(30_000));
        let idle = Duration::from_millis(request.idle_ms.unwrap_or(1000));
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let mut changed = started;
            let mut generation = None;
            let result = loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let step = this.update(cx, |this, cx| {
                    if !lease.is_active() {
                        return Err(
                            "Live command ownership was revoked by user input or reconnection."
                                .into(),
                        );
                    }
                    if this.epoch != epoch || !this.accept_updates || !cx.ai_enabled() {
                        return Err("Command observation cancelled.".into());
                    }
                    let (_, entry, _) = this
                        .resolved(cx)
                        .into_iter()
                        .find(|(value, _, _)| *value == id)
                        .ok_or_else(|| unreachable("Terminal was detached or closed."))?;
                    let info = entry.access.info(cx).ok_or("Terminal was closed.")?;
                    if info.status != TerminalStatus::Connected {
                        return Err(unreachable("Terminal is no longer connected."));
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
                    let tail = entry.access.read(
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
                    Err(_) => break Err("Chat was closed.".into()),
                }
            };
            let _ = this.update(cx, |this, _| {
                if this
                    .live_commands
                    .get(&id)
                    .is_some_and(|current| Arc::ptr_eq(current, &lease))
                {
                    this.live_commands.remove(&id);
                }
            });
            drop(lease);
            let _ = call.respond.send(result);
        })
        .detach();
    }
}
