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
use nocterm_ai::{
    BridgeCall, TerminalCall,
    context::{ConnectionDescriptor, ServerDescriptor, TerminalDescriptor},
};
use nocterm_ui::{ActiveAi as _, ActiveSettings as _};
use nocterm_workspace::{ConnectionSummary, TerminalEntry, TerminalStatus, TextRequest};

use super::{AgentThread, Attachment, SignInWait};

/// How long an agent waits for a background session to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the user has to answer a sign-in prompt for it.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);

impl AgentThread {
    /// Saved servers this chat is attached to, directly or by folder.
    fn attached_servers(&self, cx: &App) -> Vec<ConnectionSummary> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        workspace
            .read(cx)
            .connection_directory()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default()
            .into_iter()
            .filter(|summary| self.attaches_server(summary))
            .collect()
    }

    fn attaches_server(&self, summary: &ConnectionSummary) -> bool {
        self.attachment_scope()
            .iter()
            .any(|attachment| match attachment {
                Attachment::Terminal(_) | Attachment::UnavailableLocal(_) => false,
                Attachment::Connection(id) => summary.id.as_ref() == id,
                Attachment::Group(group) => summary
                    .group
                    .as_ref()
                    .is_some_and(|name| name.as_ref() == group),
            })
    }

    /// The terminals this chat may use: attached tabs, and every open
    /// session (tab or background) of an attached server.
    pub(crate) fn resolved(
        &mut self,
        cx: &App,
    ) -> Vec<(String, TerminalEntry, TerminalDescriptor)> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        let summaries = workspace
            .connection_directory()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default();
        workspace
            .terminals(cx)
            .into_iter()
            .filter_map(|entry| {
                let info = entry.access.info(cx)?;
                let summary = info
                    .profile
                    .as_ref()
                    .and_then(|profile| summaries.iter().find(|summary| summary.id == *profile));
                let attached = self
                    .attachment_scope()
                    .contains(&Attachment::Terminal(entry.item))
                    || summary.is_some_and(|summary| self.attaches_server(summary));
                if !attached {
                    return None;
                }
                let id = self.ids.get(&format!("{:?}", entry.item));
                let connection = summary
                    .map(|summary| ConnectionDescriptor {
                        id: summary.id.to_string(),
                        name: summary.name.to_string(),
                        group: summary.group.as_ref().map(ToString::to_string),
                        description: summary.description.to_string(),
                        host: summary.target.host.clone(),
                        port: summary.target.port,
                        user: summary.target.user.clone(),
                    })
                    .or_else(|| {
                        info.target.map(|target| ConnectionDescriptor {
                            id: String::new(),
                            name: entry.title.to_string(),
                            group: None,
                            description: String::new(),
                            host: target.host,
                            port: target.port,
                            user: target.user,
                        })
                    });
                let descriptor = TerminalDescriptor {
                    id: id.clone(),
                    title: entry.title.to_string(),
                    local: info.local,
                    connection,
                    cwd: info.cwd.map(|path| path.to_string_lossy().into_owned()),
                    status: format!("{:?}", info.status),
                };
                Some((id, entry, descriptor))
            })
            .collect()
    }

    /// Attached servers without a live session, with the ids agents use for
    /// them.
    pub(crate) fn offline_servers(
        &mut self,
        cx: &App,
    ) -> Vec<(ServerDescriptor, ConnectionSummary)> {
        let live: Vec<String> = self
            .resolved(cx)
            .into_iter()
            .filter(|(_, entry, _)| {
                entry
                    .access
                    .info(cx)
                    .is_some_and(|info| info.status != TerminalStatus::Closed)
            })
            .filter_map(|(_, _, descriptor)| descriptor.connection.map(|connection| connection.id))
            .collect();
        self.attached_servers(cx)
            .into_iter()
            .filter(|summary| !live.iter().any(|id| id == summary.id.as_ref()))
            .map(|summary| {
                let descriptor = ServerDescriptor {
                    server_id: self.server_ids.get(summary.id.as_ref()),
                    name: summary.name.to_string(),
                    group: summary.group.as_ref().map(ToString::to_string),
                    description: summary.description.to_string(),
                    host: summary.target.host.clone(),
                    port: summary.target.port,
                    user: summary.target.user.clone(),
                };
                (descriptor, summary)
            })
            .collect()
    }

    /// What the agent is told about its terminals with each prompt.
    pub(crate) fn context(&mut self, cx: &App) -> String {
        let terminals: Vec<_> = self
            .resolved(cx)
            .into_iter()
            .map(|(_, _, descriptor)| descriptor)
            .collect();
        let servers: Vec<_> = self
            .offline_servers(cx)
            .into_iter()
            .map(|(server, _)| server)
            .collect();
        nocterm_ai::context::context_block(&terminals, &servers)
    }

    /// A human description of what `call` acts on, for its approval card.
    pub(crate) fn describe_target(&mut self, call: &TerminalCall, cx: &App) -> String {
        if let Some(server) = call.server_id() {
            return self
                .offline_servers(cx)
                .into_iter()
                .find(|(descriptor, _)| descriptor.server_id == server)
                .map(|(descriptor, _)| {
                    format!(
                        "{} · {}@{}:{}",
                        descriptor.name, descriptor.user, descriptor.host, descriptor.port
                    )
                })
                .unwrap_or_else(|| "Unavailable server".into());
        }
        self.resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .map(|(_, entry, descriptor)| {
                format!(
                    "{}{}",
                    entry.title,
                    descriptor
                        .connection
                        .as_ref()
                        .map(|connection| format!(
                            " · {}@{}:{}",
                            connection.user, connection.host, connection.port
                        ))
                        .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "Unavailable terminal".into())
    }

    pub(crate) fn handle_tool(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        if !cx.ai_enabled()
            || !self.accept_updates
            || self.registration.as_ref().map(|value| value.id) != Some(call.registration_id)
        {
            let _ = call.respond.send(Err("Chat is unavailable.".into()));
            return;
        }
        if let Err(error) = call.call.validate() {
            let _ = call.respond.send(Err(error));
            return;
        }
        if self
            .grants
            .requires_approval(&call.call, &cx.settings().ai.approval)
        {
            self.tools.push(call);
            cx.notify();
            return;
        }
        self.execute_tool(call, cx);
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
        if grant && let Some(id) = call.call.target() {
            self.grants.grant(id, call.call.writes());
        }
        self.execute_tool(call, cx);
        cx.notify();
    }

    fn execute_tool(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        // Always resolve again after approval: attachments, auth state and tab lifetime may have changed.
        if !cx.ai_enabled()
            || !self.accept_updates
            || self.registration.as_ref().map(|value| value.id) != Some(call.registration_id)
        {
            let _ = call.respond.send(Err("Chat is unavailable.".into()));
            return;
        }
        match &call.call {
            TerminalCall::ListTerminals => {
                let payload = self.context(cx);
                let _ = call
                    .respond
                    .send(Ok(serde_json::json!({"context":payload})));
                return;
            }
            TerminalCall::OpenTerminal(request) => {
                let server = request.server_id.clone();
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
            TerminalCall::ListTerminals | TerminalCall::OpenTerminal(_) => {}
        }
    }

    fn run_command(
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
        if let Err(error) = entry.access.run_command(&request.command, cx) {
            let _ = call.respond.send(Err(error));
            return;
        }
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
                    Ok(Some(payload))
                });
                match step {
                    Ok(Ok(Some(result))) => break Ok(result),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => break Err(error),
                    Err(_) => break Err("Chat was closed.".into()),
                }
            };
            let _ = call.respond.send(result);
        })
        .detach();
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
        cx.spawn(async move |this, cx| {
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

    fn text_payload(
        &mut self,
        tail: nocterm_workspace::TerminalText,
        cx: &App,
    ) -> serde_json::Value {
        let text = if cx.settings().ai.approval.redact_secrets {
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
pub(crate) fn unreachable(reason: &str) -> String {
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
