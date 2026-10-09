//! The terminal tools a chat offers its agent: which terminals and servers it
//! may use, approvals, and carrying calls out.
//!
//! Terminals the user has open (tabs and the bottom shell) are used as they
//! are. An attached saved server without a session is "offline": the agent
//! opens it with `open_terminal`, which connects in the background without a
//! tab. A background session that needs the user (a password or a host key)
//! is moved into a tab so they can answer.
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, AsyncApp, Context, EntityId, WeakEntity};
use nocterm_ai::{BridgeCall, TerminalCall};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use nocterm_workspace::{TerminalStatus, TextRequest};

use super::{AgentThread, SignInWait};

/// How long an agent waits for a background session to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the user has to answer a sign-in prompt for it.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);

impl AgentThread {
    pub(crate) fn handle_tool(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        if !cx.ai_enabled()
            || !self.accept_updates
            || self.registration().as_ref().map(|value| value.id) != Some(call.registration_id)
        {
            let _ = call.respond.send(Err("Chat is unavailable.".into()));
            return;
        }
        if let Err(error) = call.call.validate() {
            let _ = call.respond.send(Err(error));
            return;
        }
        if self.grants.requires_approval(
            &call.call,
            &cx.setting::<nocterm_settings::AiSettings>().approval,
        ) || self.unsafe_live_input(&call.call, cx)
        {
            self.approval_generation = self.approval_generation.wrapping_add(1);
            self.tools.push(call);
            cx.notify();
            return;
        }
        self.execute_tool(call, cx);
    }

    pub(crate) fn unsafe_live_input(&mut self, call: &TerminalCall, cx: &App) -> bool {
        if !matches!(call, TerminalCall::SendInput(_)) {
            return false;
        }
        self.resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .and_then(|(_, entry, _)| entry.access.info(cx))
            .is_none_or(|info| info.at_prompt != Some(true) || info.dirty_input || info.alt_screen)
    }

    pub(crate) fn approve_tool(
        &mut self,
        index: usize,
        allow: bool,
        grant: bool,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tools.len() {
            return;
        }
        let call = self.tools.remove(index);
        if !allow {
            let _ = call.respond.send(Err("User denied this request.".into()));
            cx.notify();
            return;
        }
        let attached = match (call.call.terminal_id(), call.call.server_id()) {
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
        if !cx.ai_enabled() || !self.accept_updates || !attached {
            let _ = call.respond.send(Err(unreachable(
                "Terminal was detached or chat is unavailable.",
            )));
            cx.notify();
            return;
        }
        if grant
            && !self.unsafe_live_input(&call.call, cx)
            && let Some(id) = call.call.target()
        {
            self.grants.grant(id, call.call.capability());
        }
        self.execute_tool(call, cx);
        cx.notify();
    }

    fn execute_tool(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        // Always resolve again after approval: attachments, auth state and tab lifetime may have changed.
        if !cx.ai_enabled()
            || !self.accept_updates
            || self.registration().as_ref().map(|value| value.id) != Some(call.registration_id)
        {
            let _ = call.respond.send(Err("Chat is unavailable.".into()));
            return;
        }
        match &call.call {
            TerminalCall::ListTerminals => {
                self.record_tool_display(&call.call, None, cx);
                let payload = self.context(cx);
                let _ = call
                    .respond
                    .send(Ok(serde_json::json!({"context":payload})));
                return;
            }
            TerminalCall::OpenTerminal(request) => {
                let server = request.server_id.clone();
                self.record_tool_display(&call.call, None, cx);
                self.open_terminal(server, call, cx);
                return;
            }
            _ => {}
        }
        let Some((_, entry, _)) = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.call.terminal_id())
        else {
            let _ = call.respond.send(Err(unreachable(
                "Terminal is not attached or was closed. Call list_terminals for the current ones.",
            )));
            return;
        };
        self.record_tool_display(&call.call, Some(&entry), cx);
        match &call.call {
            TerminalCall::ReadTerminal(request) => {
                let result = entry
                    .access
                    .read(
                        TextRequest {
                            max_lines: request.lines.unwrap_or(200),
                            max_bytes: 64 * 1024,
                            since_line: request.since,
                        },
                        cx,
                    )
                    .map(|tail| self.text_payload(tail, cx));
                let _ = call.respond.send(result);
            }
            TerminalCall::SendInput(request) => {
                let text = format!(
                    "{}{}",
                    request.text,
                    if request.press_enter { "\r" } else { "" }
                );
                let result = entry
                    .access
                    .send_text(&text, cx)
                    .map(|()| serde_json::json!({"accepted":true}));
                let _ = call.respond.send(result);
            }
            TerminalCall::RunCommand(request) => {
                let request = request.clone();
                self.run_command(entry, request, call, cx);
            }
            TerminalCall::ExecCommand(_) => self.execute_command(entry, call, cx),
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
            let _ = call.respond.send(match open {
                Some((_, _, descriptor)) => Ok(serde_json::json!({"terminal": descriptor})),
                None => Err(unreachable(
                    "Unknown server. Call list_terminals for the attached ones.",
                )),
            });
            return;
        };
        let (Some(window), Some(directory)) = (
            self.window,
            self.workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).connection_directory()),
        ) else {
            let _ = call
                .respond
                .send(Err(unreachable("The workspace is unavailable.")));
            return;
        };
        let workspace = self.workspace.clone();
        let profile = summary.id.to_string();
        let epoch = self.epoch;
        let guard = self.hold_operation();
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            let opened = cx
                .update_window(window, |_, window, cx| {
                    directory.open_background(&profile, &workspace, window, cx)
                })
                .ok()
                .flatten();
            let Some(item) = opened else {
                let _ = call.respond.send(Err(unreachable(
                    "Could not open a session for this server.",
                )));
                return;
            };
            let _ = this.update(cx, |this, _| this.background.push(item));
            let result = wait_until_connected(&this, &workspace, window, item, epoch, cx).await;
            if result.is_err() {
                close_background(&workspace, window, item, cx);
                let _ = this.update(cx, |this, _| this.background.retain(|id| *id != item));
            }
            let _ = call.respond.send(result);
        })
        .detach();
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
            .setting::<nocterm_settings::AiSettings>()
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

/// Waits for background session `item` to connect. A sign-in prompt moves it
/// into a tab, so the user can answer it.
#[expect(clippy::too_many_lines, reason = "predates the limit")]
async fn wait_until_connected(
    this: &WeakEntity<AgentThread>,
    workspace: &WeakEntity<nocterm_workspace::Workspace>,
    window: gpui_kit::AnyWindowHandle,
    item: EntityId,
    epoch: u64,
    cx: &mut AsyncApp,
) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    let mut shown = false;
    let mut waiting = false;
    let result = loop {
        cx.background_executor()
            .timer(Duration::from_millis(150))
            .await;
        let step = this
            .update(cx, |this, cx| {
                if this.epoch != epoch || !this.accept_updates || !cx.ai_enabled() {
                    return Err("The chat stopped before the server connected.".to_owned());
                }
                let entry = this
                    .resolved(cx)
                    .into_iter()
                    .find(|(_, entry, _)| entry.item == item);
                let Some((_, entry, descriptor)) = entry else {
                    return Err("The session was closed before it connected.".into());
                };
                let info = entry.access.info(cx);
                let status = info.as_ref().map(|info| info.status);
                // A password is asked in the chat; the session stays hidden.
                let wait = info.and_then(|info| {
                    let vault = info.status == TerminalStatus::AwaitingVault;
                    let user = info.status == TerminalStatus::AwaitingUser;
                    Some(SignInWait {
                        item,
                        title: entry.title.clone(),
                        access: entry.access.clone(),
                        prompt: info.sign_in.filter(|_| vault || user)?,
                        vault,
                    })
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
            .map_err(|_| "The chat was closed.".to_owned());
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
                break Err(unreachable("Could not connect to the server."));
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
                "The server is waiting for the user to sign in; ask them to finish signing in, then call list_terminals.".into()
            } else if waiting {
                "The server is waiting for the user to sign in from the chat or unlock the vault; ask them to, then call open_terminal again.".into()
            } else {
                unreachable("Connecting to the server timed out.")
            });
        }
    };
    let _ = this.update(cx, |this, cx| {
        this.sign_ins.retain(|wait| wait.item != item);
        cx.notify();
    });
    result
}

/// A failure to reach a terminal or server, with what the agent should do
/// about it: tell the user rather than look for another way in.
pub(super) fn unreachable(reason: &str) -> String {
    format!(
        "{reason} Tell the user right away, in one short sentence, which terminal or server you could not reach and why. Do not try to connect another way: nocterm holds the credentials."
    )
}

fn close_background(
    workspace: &WeakEntity<nocterm_workspace::Workspace>,
    window: gpui_kit::AnyWindowHandle,
    item: EntityId,
    cx: &mut AsyncApp,
) {
    let workspace = workspace.clone();
    let _ = cx.update_window(window, |_, window, cx| {
        let _ = workspace.update(cx, |workspace, cx| {
            workspace.close_background_session(item, window, cx)
        });
    });
}
