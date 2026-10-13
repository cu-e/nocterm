//! The terminal tools a chat offers its agent: which terminals and servers it
//! may use, approvals, and carrying calls out.
//!
//! Terminals the user has open (tabs and the bottom shell) are used as they
//! are. An attached saved server without a session is "offline": the agent
//! opens it with `open_terminal`, which connects in the background without a
//! tab. A background session that needs the user (a password or a host key)
//! is moved into a tab so they can answer. A server terminal whose
//! connection closed reconnects in place when a call needs it.
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, AsyncApp, Context, EntityId, Task, WeakEntity};
use nocterm_ai::{BridgeCall, TerminalCall, ToolFailure};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use nocterm_workspace::{
    ExpectedSession, TerminalCapability, TerminalEntry, TerminalError, TerminalStatus, TextRequest,
};

use super::{
    AgentThread, SignInWait,
    approval_inbox::{ApprovalToken, ToolDecision, ToolGate, ToolRejection},
    background_open::BackgroundOpen,
    bound_call::BoundToolCall,
    connection_wait::ConnectionWaitError,
};

/// How long an agent waits for a background session to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the user has to answer a sign-in prompt for it.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);

impl AgentThread {
    pub(crate) fn handle_tool(&mut self, mut call: BridgeCall, cx: &mut Context<Self>) {
        if !cx.ai_enabled()
            || !self.lifecycle.accepts_updates()
            || self.registration().as_ref().map(|value| value.id) != Some(call.registration_id)
        {
            self.finish(call, Err("Chat is unavailable.".into()));
            return;
        }
        self.record_tool_display(&mut call, None, cx);
        if let Err(error) = call.call.validate() {
            self.finish(call, Err(error.into()));
            return;
        }
        let gate = self.tool_gate(&call.call, cx);
        let expected = self.tool_expectation(&call.call, cx);
        let decision = self.approvals.receive_tool(
            BoundToolCall::new(call, expected),
            &gate,
            &cx.setting::<nocterm_ai::AiSettings>().approval,
        );
        self.apply_tool_decision(decision, cx);
    }

    pub(crate) fn unsafe_live_input(&mut self, call: &TerminalCall, cx: &App) -> bool {
        if !matches!(call, TerminalCall::SendInput(_)) {
            return false;
        }
        self.resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .and_then(|(_, entry, _)| entry.capabilities.state().info(cx))
            .is_none_or(|info| info.at_prompt != Some(true) || info.dirty_input || info.alt_screen)
    }

    fn tool_expectation(&mut self, call: &TerminalCall, cx: &App) -> Option<ExpectedSession> {
        self.resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .and_then(|(_, entry, _)| entry.capabilities.state().info(cx))
            .map(|info| info.expected)
    }

    fn tool_gate(&mut self, call: &TerminalCall, cx: &App) -> ToolGate {
        let attached = match (call.terminal_id(), call.server_id()) {
            (Some(id), _) => self
                .resolved(cx)
                .iter()
                .any(|(candidate, _, _)| candidate == id),
            (None, Some(id)) => self
                .offline_servers(cx)
                .iter()
                .any(|(server, _)| server.server_id == id),
            (None, None) => true,
        };
        let unsafe_input = self.unsafe_live_input(call, cx);
        let expected = self.tool_expectation(call, cx);
        ToolGate::new(
            cx.ai_enabled() && self.lifecycle.accepts_updates(),
            self.registration().as_ref().map(|value| value.id),
            attached,
            unsafe_input,
            expected,
        )
    }

    pub(crate) fn approve_tool(
        &mut self,
        token: ApprovalToken,
        allow: bool,
        grant: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(call) = self.approvals.tool(token).cloned() else {
            return;
        };
        let gate = self.tool_gate(&call, cx);
        let decision = self.approvals.choose_tool(token, allow, grant, &gate);
        self.apply_tool_decision(decision, cx);
    }

    fn apply_tool_decision(&mut self, decision: ToolDecision, cx: &mut Context<Self>) {
        match decision {
            ToolDecision::Respond(call, reason) => {
                let error = match reason {
                    ToolRejection::Unavailable => ToolFailure::from("Chat is unavailable."),
                    ToolRejection::Denied => ToolFailure::from("User denied this request."),
                    ToolRejection::Detached => {
                        unreachable("Terminal was detached or chat is unavailable.")
                    }
                };
                self.finish(call, Err(error));
            }
            ToolDecision::Execute(call) => self.execute_tool(call, cx),
            ToolDecision::Queue => {}
            ToolDecision::Unchanged => return,
        }
        cx.notify();
    }

    fn execute_tool(&mut self, call: BoundToolCall, cx: &mut Context<Self>) {
        self.execute_tool_with(call, true, cx);
    }

    /// Carries out `call`; with `reconnect`, a closed server terminal it
    /// needs is connected again first.
    fn execute_tool_with(&mut self, bound: BoundToolCall, reconnect: bool, cx: &mut Context<Self>) {
        let captured = bound.expected();
        let call = bound.call();
        if !cx.ai_enabled()
            || !self.lifecycle.accepts_updates()
            || self.registration().as_ref().map(|value| value.id) != Some(call.registration_id)
        {
            self.finish(bound.into_call(), Err("Chat is unavailable.".into()));
            return;
        }
        match &call.call {
            TerminalCall::ListTerminals => {
                let payload = self.context(cx);
                self.finish(
                    bound.into_call(),
                    Ok(serde_json::json!({"context":payload})),
                );
                return;
            }
            TerminalCall::OpenTerminal(request) => {
                let server = request.server_id.clone();

                self.open_terminal(server, bound.into_call(), cx);
                return;
            }
            _ => {}
        }
        let Some((_, entry, _)) = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.call.terminal_id())
        else {
            self.finish_unreachable(
                bound.into_call(),
                "Terminal is not attached or was closed. Call list_terminals for the current ones.",
            );
            return;
        };
        let Some(expected) = captured else {
            self.finish(
                bound.into_call(),
                Err(TerminalError::StaleSession.to_string().into()),
            );
            return;
        };
        if entry
            .capabilities
            .state()
            .info(cx)
            .is_none_or(|info| info.expected != expected)
        {
            self.finish(
                bound.into_call(),
                Err(TerminalError::StaleSession.to_string().into()),
            );
            return;
        }
        if reconnect && self.needs_reconnect(&call.call, &entry, cx) {
            self.reconnect_then_execute(bound, &entry, expected, cx);
            return;
        }

        self.dispatch_tool(bound.into_call(), entry, expected, cx);
    }

    fn dispatch_tool(
        &mut self,
        call: BridgeCall,
        entry: TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) {
        match &call.call {
            TerminalCall::ReadTerminal(request) => {
                let result = entry
                    .capabilities
                    .text()
                    .ok_or(TerminalError::Unsupported(TerminalCapability::TextRead))
                    .and_then(|port| {
                        port.read(
                            expected,
                            TextRequest {
                                max_lines: request.lines.unwrap_or(200),
                                max_bytes: 64 * 1024,
                                since_line: request.since,
                            },
                            cx,
                        )
                    })
                    .map(|tail| self.text_payload(tail, cx));
                self.finish(call, result.map_err(|error| error.to_string().into()));
            }
            TerminalCall::SendInput(request) => {
                let text = format!(
                    "{}{}",
                    request.text,
                    if request.press_enter { "\r" } else { "" }
                );
                let result = entry
                    .capabilities
                    .input()
                    .ok_or(TerminalError::Unsupported(TerminalCapability::Input))
                    .and_then(|port| port.send_text(expected, &text, cx))
                    .map(|()| serde_json::json!({"accepted":true}));
                self.finish(call, result.map_err(|error| error.to_string().into()));
            }
            TerminalCall::RunCommand(request) => {
                let request = request.clone();
                self.run_command(entry, expected, request, call, cx);
            }
            TerminalCall::ExecCommand(_) => self.execute_command(entry, expected, call, cx),
            TerminalCall::ReadCommand(_) => self.read_command(call, cx),
            TerminalCall::CancelCommand(_) => self.cancel_command(call, cx),
            TerminalCall::ListTerminals | TerminalCall::OpenTerminal(_) => {}
        }
    }

    /// Connects to an attached offline server without a tab and answers with
    /// its terminal once it is ready.
    fn open_terminal(&mut self, server: String, call: BridgeCall, cx: &mut Context<Self>) {
        let summary = self
            .offline_servers(cx)
            .into_iter()
            .find(|(descriptor, _)| descriptor.server_id == server)
            .map(|(_, summary)| summary);
        let Some(summary) = summary else {
            // Already connected: answer with the open session instead.
            let open = self.resolved(cx).into_iter().find(|(_, _, descriptor)| {
                descriptor.connection.as_ref().is_some_and(|connection| {
                    self.server_ids.resolve(&server) == Some(connection.id.as_str())
                })
            });
            match open {
                Some((_, _, descriptor)) => {
                    self.finish(call, Ok(serde_json::json!({"terminal": descriptor})));
                }
                None => self.finish_unreachable(
                    call,
                    "Unknown server. Call list_terminals for the attached ones.",
                ),
            }
            return;
        };
        // A session the chat opened before and lost is connected again
        // rather than left behind for another.
        if let Some(entry) = self.closed_background(summary.id.as_ref(), cx) {
            let Some(expected) = entry
                .capabilities
                .state()
                .info(cx)
                .map(|info| info.expected)
            else {
                self.finish_unreachable(call, "The terminal was closed.");
                return;
            };
            let connected = self.reconnect(&entry, expected, cx);
            cx.spawn(async move |this, cx| {
                let result = connected.await;
                let _ = this.update(cx, |this, cx| {
                    this.finish(
                        call,
                        result
                            .map(|(_, payload)| payload)
                            .map_err(ConnectionWaitError::into_failure),
                    );
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let (Some(window), Some(directory)) = (
            self.window,
            self.workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).connection_directory()),
        ) else {
            self.finish_unreachable(call, "The workspace is unavailable.");
            return;
        };
        let opening = BackgroundOpen::new(
            self.workspace.clone(),
            window,
            directory,
            summary.id.to_string(),
            self.lifecycle.ticket(),
            self.registration().as_ref().map(|value| value.id),
        );
        let guard = self.hold_operation();
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            let result = opening.open(&this, cx).await;
            let _ = this.update(cx, |this, cx| {
                this.finish(call, result.map_err(ConnectionWaitError::into_failure));
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_unreachable(&mut self, call: BridgeCall, reason: &str) {
        self.finish(call, Err(unreachable(reason)));
    }

    /// Whether `call` acts on a server terminal that closed, or that is
    /// already reconnecting for another call.
    fn needs_reconnect(&self, call: &TerminalCall, entry: &TerminalEntry, cx: &App) -> bool {
        let uses_session = matches!(
            call,
            TerminalCall::ReadTerminal(_)
                | TerminalCall::SendInput(_)
                | TerminalCall::RunCommand(_)
                | TerminalCall::ExecCommand(_)
        );
        uses_session
            && (self.reconnecting.contains(&entry.item)
                || entry
                    .capabilities
                    .state()
                    .info(cx)
                    .is_some_and(|info| !info.local && info.status == TerminalStatus::Closed))
    }

    /// Reconnects `entry` in place, then carries out `call` once more. The
    /// user's tab stays open if connecting fails.
    fn reconnect_then_execute(
        &mut self,
        call: BoundToolCall,
        entry: &TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) {
        let connected = self.reconnect(entry, expected, cx);
        cx.spawn(async move |this, cx| {
            let result = connected.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((receipt, _)) => {
                        this.execute_tool_with(call.reconnected(receipt), false, cx)
                    }
                    Err(error) => this.finish(call.into_call(), Err(error.into_failure())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Connects the closed session `entry` again, or joins a reconnect
    /// already under way; resolves with its descriptor once connected.
    fn reconnect(
        &mut self,
        entry: &TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) -> Task<Result<(ExpectedSession, serde_json::Value), ConnectionWaitError>> {
        let Some(window) = self.window else {
            return Task::ready(Err(ConnectionWaitError::WorkspaceClosed));
        };
        let receipt = match self.reconnecting.receipt(expected) {
            Some(receipt) => receipt,
            None => {
                let result = entry
                    .capabilities
                    .reconnect()
                    .ok_or(TerminalError::Unsupported(TerminalCapability::Reconnect))
                    .and_then(|port| port.reconnect(expected, cx));
                match result {
                    Ok(receipt) => {
                        self.reconnecting.insert(receipt);
                        receipt
                    }
                    Err(error) => return Task::ready(Err(ConnectionWaitError::Terminal(error))),
                }
            }
        };
        let workspace = self.workspace.clone();
        let ticket = self.lifecycle.ticket();
        let registration = self.registration().as_ref().map(|value| value.id);
        let guard = self.hold_operation();
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            let result = wait_until_connected(
                &this,
                &WaitTarget {
                    workspace,
                    window,
                    expected: receipt,
                    ticket,
                    registration,
                },
                cx,
            )
            .await;
            let _ = this.update(cx, |this, _| this.reconnecting.remove(receipt));
            result.map(|payload| (receipt, payload))
        })
    }

    /// A session this chat opened for `profile` whose connection closed.
    fn closed_background(&mut self, profile: &str, cx: &App) -> Option<TerminalEntry> {
        let background = self.background.clone();
        self.resolved(cx)
            .into_iter()
            .find(|(_, entry, descriptor)| {
                background.contains(&entry.item)
                    && descriptor
                        .connection
                        .as_ref()
                        .is_some_and(|connection| connection.id == profile)
                    && entry
                        .capabilities
                        .state()
                        .info(cx)
                        .is_some_and(|info| info.status == TerminalStatus::Closed)
            })
            .map(|(_, entry, _)| entry)
    }

    /// Ends background sessions this chat opened that it can no longer use.
    pub(crate) fn prune_background(&mut self, cx: &mut Context<Self>) {
        let resolved: Vec<EntityId> = self
            .resolved(cx)
            .into_iter()
            .map(|(_, entry, _)| entry.item)
            .collect();
        let (keep, close): (Vec<_>, Vec<_>) = self
            .background
            .drain(..)
            .partition(|id| resolved.contains(id));
        self.background = keep;
        if let Some(window) = self.window {
            let workspace = self.workspace.clone();
            cx.defer(move |cx| {
                for item in close {
                    let _ = cx.update_window(window, |_, window, cx| {
                        let _ = workspace.update(cx, |workspace, cx| {
                            workspace.close_background_session(item, window, cx)
                        });
                    });
                }
            });
        }
    }

    pub(super) fn text_payload(
        &mut self,
        tail: nocterm_workspace::TerminalText,
        cx: &App,
    ) -> serde_json::Value {
        let text = if cx
            .setting::<nocterm_ai::AiSettings>()
            .approval
            .redact_secrets
        {
            nocterm_ai::redact::redact(&tail.text)
        } else {
            tail.text
        };
        self.tool_bytes += text.len();
        serde_json::json!({"text":text,"first_line":tail.first_line,"next_line":tail.next_line,"cursor_semantics":"inclusive snapshot; replace overlapping lines","truncated":tail.truncated,"alt_screen":tail.alt_screen})
    }
}

/// Authority held by one waiter, never inferred from a replacement session.
pub(super) struct WaitTarget {
    workspace: WeakEntity<nocterm_workspace::Workspace>,
    window: gpui_kit::AnyWindowHandle,
    expected: ExpectedSession,
    ticket: nocterm_ai::session::Ticket,
    registration: Option<u64>,
}
impl WaitTarget {
    pub(super) fn new(
        workspace: WeakEntity<nocterm_workspace::Workspace>,
        window: gpui_kit::AnyWindowHandle,
        expected: ExpectedSession,
        ticket: nocterm_ai::session::Ticket,
        registration: Option<u64>,
    ) -> Self {
        Self {
            workspace,
            window,
            expected,
            ticket,
            registration,
        }
    }
}
/// Waits for the particular accepted session to connect.
#[expect(clippy::too_many_lines, reason = "predates the limit")]
pub(super) async fn wait_until_connected(
    this: &WeakEntity<AgentThread>,
    target: &WaitTarget,
    cx: &mut AsyncApp,
) -> Result<serde_json::Value, ConnectionWaitError> {
    let WaitTarget {
        workspace,
        window,
        expected,
        ticket,
        registration,
    } = target;
    let (window, expected, ticket) = (*window, *expected, *ticket);
    let registration = *registration;
    let item = expected.item();
    let started = Instant::now();
    let mut shown = false;
    let mut waiting = false;
    let result = loop {
        cx.background_executor()
            .timer(Duration::from_millis(150))
            .await;
        let step = this
            .update(cx, |this, cx| {
                if this.workspace.entity_id() != workspace.entity_id()
                    || this.registration().as_ref().map(|value| value.id) != registration
                    || !this.lifecycle.session_current(ticket)
                    || !this.lifecycle.accepts_updates()
                    || !cx.ai_enabled()
                {
                    return Err(ConnectionWaitError::Stopped);
                }
                let entry = this
                    .resolved(cx)
                    .into_iter()
                    .find(|(_, entry, _)| entry.item == item);
                let Some((_, entry, descriptor)) = entry else {
                    return Err(ConnectionWaitError::SessionClosed);
                };
                let info = entry.capabilities.state().info(cx);
                if info.as_ref().is_none_or(|info| info.expected != expected) {
                    return Err(ConnectionWaitError::Terminal(TerminalError::StaleSession));
                }
                let status = info.as_ref().map(|info| info.status);
                // A password is asked in the chat; the session stays hidden.
                let wait = info.and_then(|info| {
                    let vault = info.status == TerminalStatus::AwaitingVault;
                    let user = info.status == TerminalStatus::AwaitingUser;
                    let prompts = entry.capabilities.prompts()?.clone();
                    let prompt = prompts
                        .pending(cx)
                        .filter(|prompt| prompt.token.expected() == expected && (vault || user))?;
                    Some(SignInWait::new(
                        item,
                        entry.title.clone(),
                        prompts,
                        prompt,
                        vault,
                    ))
                });
                let asking = wait.is_some();
                let current = this.sign_ins.iter().position(|wait| wait.item == item);
                match (current, wait) {
                    (Some(index), Some(wait)) => {
                        let old = &this.sign_ins[index];
                        if old.prompt != wait.prompt || old.vault != wait.vault {
                            this.sign_ins[index] = wait;
                            cx.notify();
                        }
                    }
                    (None, Some(wait)) => {
                        this.sign_ins.push(wait);
                        cx.notify();
                    }
                    (Some(index), None) => {
                        this.sign_ins.remove(index);
                        cx.notify();
                    }
                    (None, None) => {}
                }
                Ok((status, asking, descriptor))
            })
            .map_err(|_| ConnectionWaitError::ChatClosed);
        let step = match step {
            Ok(Ok(step)) => step,
            Ok(Err(error)) | Err(error) => break Err(error),
        };
        match step {
            (Some(TerminalStatus::Connected), _, descriptor) => {
                break Ok(serde_json::json!({ "terminal": descriptor }));
            }
            (_, true, _) => waiting = true,
            // A host key question is answered in the session's own tab.
            (Some(TerminalStatus::AwaitingUser), false, _) if !shown => {
                shown = true;
                let workspace = workspace.clone();
                let _ = cx.update_window(window, |_, window, cx| {
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.show_background_session(item, window, cx)
                    });
                });
            }
            (Some(TerminalStatus::Closed) | None, _, _) => {
                break Err(ConnectionWaitError::ConnectFailed);
            }
            _ => {}
        }
        let limit = if shown || waiting {
            SIGN_IN_TIMEOUT
        } else {
            CONNECT_TIMEOUT
        };
        if started.elapsed() >= limit {
            break Err(if shown {
                ConnectionWaitError::VisiblePromptTimeout
            } else if waiting {
                ConnectionWaitError::HiddenPromptTimeout
            } else {
                ConnectionWaitError::Timeout
            });
        }
    };
    let _ = this.update(cx, |this, cx| {
        this.sign_ins
            .retain(|wait| wait.item != item || wait.prompt.token.expected() != expected);
        cx.notify();
    });
    result
}

/// A failure to reach a terminal or server, with what the agent should do
/// about it: tell the user rather than look for another way in.
pub(super) fn unreachable(reason: &str) -> ToolFailure {
    format!(
        "{reason} Tell the user right away, in one short sentence, which terminal or server you could not reach and why. Do not try to connect another way: nocterm holds the credentials."
    )
    .into()
}
