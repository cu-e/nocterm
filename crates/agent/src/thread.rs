use crate::runtime::Runtime;
use futures::channel::oneshot;
use gpui_kit::{App, Context, EntityId, Subscription, WeakEntity};
use nocterm_ai::{
    AgentCommands, AgentInfo, BridgeCall, BridgeRegistration, TerminalCall, acp,
    approval::ApprovalGrants,
    context::{ConnectionDescriptor, OpaqueIds, TerminalDescriptor},
    thread::ThreadState,
};
use nocterm_ui::{ActiveAi as _, ActiveSettings as _};
use nocterm_workspace::{TerminalEntry, TerminalStatus, TextRequest, Workspace};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Attachment {
    Terminal(EntityId),
    Connection(String),
    Group(String),
}
pub(crate) struct PendingPermission {
    pub request: acp::RequestPermissionRequest,
    pub respond: oneshot::Sender<acp::RequestPermissionOutcome>,
}
pub(crate) struct AgentThread {
    pub agent_id: String,
    pub state: ThreadState,
    pub status: String,
    pub generating: bool,
    pub accept_updates: bool,
    pub session: Option<acp::SessionId>,
    pub commands: Option<Arc<dyn AgentCommands>>,
    pub info: Option<AgentInfo>,
    pub connection_key: Option<u64>,
    pub registration: Option<BridgeRegistration>,
    pub workspace: WeakEntity<Workspace>,
    pub attachments: Vec<Attachment>,
    pub images: Vec<nocterm_ai::images::PromptImage>,
    pub permissions: Vec<PendingPermission>,
    pub tools: Vec<BridgeCall>,
    pub context_bytes: usize,
    pub tool_bytes: usize,
    pub epoch: u64,
    ids: OpaqueIds,
    grants: ApprovalGrants,
    _release: Subscription,
}
impl AgentThread {
    pub(crate) fn new(
        agent_id: String,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = cx.entity_id();
        let release = cx.on_release(move |this, cx| {
            this.cancel_pending();
            if let (Some(commands), Some(session)) = (&this.commands, &this.session) {
                commands.cancel(session.clone());
                commands.close_session(session.clone());
            }
            let registration = this.registration.take().map(|registration| registration.id);
            Runtime::global(cx).update(cx, |runtime, _| {
                runtime.release(id, this.connection_key, registration)
            });
        });
        Self {
            agent_id,
            state: Default::default(),
            status: "Connecting…".into(),
            generating: false,
            accept_updates: true,
            session: None,
            commands: None,
            info: None,
            connection_key: None,
            registration: None,
            workspace,
            attachments: Vec::new(),
            images: Vec::new(),
            permissions: Vec::new(),
            tools: Vec::new(),
            context_bytes: 0,
            tool_bytes: 0,
            epoch: 0,
            ids: Default::default(),
            grants: Default::default(),
            _release: release,
        }
    }
    pub(crate) fn create_session(
        &mut self,
        commands: Arc<dyn AgentCommands>,
        info: AgentInfo,
        workdir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() {
            return;
        }
        let Some(registration) = &self.registration else {
            self.fail("Terminal bridge is unavailable.", cx);
            return;
        };
        let binary = match std::env::current_exe() {
            Ok(binary) => binary,
            Err(error) => {
                self.fail(&error.to_string(), cx);
                return;
            }
        };
        let server = acp::McpServerStdio::new("nocterm", binary)
            .args(vec!["agent-bridge".into()])
            .env(vec![
                acp::EnvVariable::new("NOCTERM_BRIDGE_ENDPOINT", registration.endpoint.clone()),
                acp::EnvVariable::new("NOCTERM_BRIDGE_TOKEN", registration.token.clone()),
            ]);
        self.commands = Some(commands.clone());
        self.info = Some(info);
        self.status = "Starting chat…".into();
        let epoch = self.epoch;
        let future = cx.background_executor().spawn(commands.new_session(
            acp::NewSessionRequest::new(workdir).mcp_servers(vec![acp::McpServer::Stdio(server)]),
        ));
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || !cx.ai_enabled() {
                    if let Ok(response) = result {
                        commands.close_session(response.session_id);
                    }
                    return;
                }
                match result {
                    Ok(response) => {
                        this.state.modes = response.modes;
                        this.state.config_options = response.config_options.unwrap_or_default();
                        this.session = Some(response.session_id);
                        this.status = "Ready".into();
                        cx.notify();
                    }
                    Err(error) => this.fail(&error.to_string(), cx),
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.accept_updates = false;
        self.generating = false;
        self.status = nocterm_ai::redact::redact(message);
        self.cancel_pending();
        self.registration.take();
        cx.notify();
    }
    fn cancel_pending(&mut self) {
        for permission in self.permissions.drain(..) {
            let _ = permission
                .respond
                .send(acp::RequestPermissionOutcome::Cancelled);
        }
        for call in self.tools.drain(..) {
            let _ = call.respond.send(Err("Request cancelled.".into()));
        }
        self.grants.clear();
    }
    pub(crate) fn permission(
        &mut self,
        request: acp::RequestPermissionRequest,
        respond: oneshot::Sender<acp::RequestPermissionOutcome>,
        cx: &mut Context<Self>,
    ) {
        if !self.accept_updates || !cx.ai_enabled() {
            let _ = respond.send(acp::RequestPermissionOutcome::Cancelled);
            return;
        }
        self.permissions
            .push(PendingPermission { request, respond });
        cx.notify();
    }
    pub(crate) fn choose_permission(
        &mut self,
        index: usize,
        option: Option<acp::PermissionOptionId>,
        cx: &mut Context<Self>,
    ) {
        if index >= self.permissions.len() {
            return;
        }
        let permission = self.permissions.remove(index);
        let outcome = option
            .filter(|option| {
                cx.ai_enabled()
                    && self.accept_updates
                    && permission
                        .request
                        .options
                        .iter()
                        .any(|candidate| candidate.option_id == *option)
            })
            .map(|option| {
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(option))
            })
            .unwrap_or(acp::RequestPermissionOutcome::Cancelled);
        let _ = permission.respond.send(outcome);
        cx.notify();
    }
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
                let attached = self.attachments.iter().any(|attachment| match attachment {
                    Attachment::Terminal(id) => *id == entry.item,
                    Attachment::Connection(id) => {
                        summary.is_some_and(|summary| summary.id.as_ref() == id)
                    }
                    Attachment::Group(group) => summary.is_some_and(|summary| {
                        summary
                            .group
                            .as_ref()
                            .is_some_and(|name| name.as_ref() == group)
                    }),
                });
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
    pub(crate) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if !cx.ai_enabled() || self.generating || !self.accept_updates {
            return;
        }
        let (Some(commands), Some(session)) = (self.commands.clone(), self.session.clone()) else {
            return;
        };
        if text.trim().is_empty() && self.images.is_empty() {
            return;
        }
        let descriptors = self
            .resolved(cx)
            .into_iter()
            .map(|(_, _, descriptor)| descriptor)
            .collect::<Vec<_>>();
        let context = nocterm_ai::context::context_block(&descriptors);
        self.context_bytes += context.len();
        let mut visible = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
        visible.extend(self.images.drain(..).map(|image| image.content()));
        self.state.push_user(visible.clone());
        let mut content = vec![acp::ContentBlock::Text(acp::TextContent::new(context))];
        content.extend(visible);
        self.generating = true;
        self.status = "Working…".into();
        let epoch = self.epoch;
        let future = cx
            .background_executor()
            .spawn(commands.prompt(acp::PromptRequest::new(session, content)));
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || !cx.ai_enabled() {
                    return;
                }
                this.generating = false;
                this.cancel_pending();
                match result {
                    Ok(response) => {
                        this.status = if this.accept_updates {
                            format!("{:?}", response.stop_reason)
                        } else {
                            "Stopped. Restart this chat to continue with a fresh agent session."
                                .into()
                        };
                        cx.notify();
                    }
                    Err(error) => this.fail(&error.to_string(), cx),
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn set_config(
        &mut self,
        id: acp::SessionConfigId,
        value: acp::SessionConfigOptionValue,
        cx: &mut Context<Self>,
    ) {
        if self.generating || !cx.ai_enabled() {
            return;
        }
        let (Some(commands), Some(session)) = (self.commands.clone(), self.session.clone()) else {
            return;
        };
        let epoch = self.epoch;
        let future = cx.background_executor().spawn(
            commands.set_config_option(acp::SetSessionConfigOptionRequest::new(session, id, value)),
        );
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || !cx.ai_enabled() {
                    return;
                }
                match result {
                    Ok(options) => this.state.config_options = options,
                    Err(error) => this.status = nocterm_ai::redact::redact(&error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn set_mode(&mut self, id: acp::SessionModeId, cx: &mut Context<Self>) {
        if self.generating || !cx.ai_enabled() {
            return;
        }
        let (Some(commands), Some(session)) = (self.commands.clone(), self.session.clone()) else {
            return;
        };
        let epoch = self.epoch;
        let future = cx
            .background_executor()
            .spawn(commands.set_mode(acp::SetSessionModeRequest::new(session, id.clone())));
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || !cx.ai_enabled() {
                    return;
                }
                match result {
                    Ok(()) => this.state.current_mode = Some(id),
                    Err(error) => this.status = nocterm_ai::redact::redact(&error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        if let (Some(commands), Some(session)) = (&self.commands, &self.session) {
            commands.cancel(session.clone());
        }
        self.accept_updates = false;
        self.cancel_pending();
        self.status = "Stopping…".into();
        let epoch = self.epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generating && this.epoch == epoch {
                    this.fail(
                        "Cancellation timed out. Restart this chat before sending another prompt.",
                        cx,
                    );
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn attach(&mut self, attachment: Attachment, cx: &mut Context<Self>) {
        if let Some(index) = self
            .attachments
            .iter()
            .position(|value| value == &attachment)
        {
            self.attachments.remove(index);
            self.grants.clear();
            self.cancel_pending();
        } else {
            self.attachments.push(attachment);
        }
        cx.notify();
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
        } else {
            let attached = call.call.terminal_id().is_none_or(|id| {
                self.resolved(cx)
                    .iter()
                    .any(|(candidate, _, _)| candidate == id)
            });
            if !cx.ai_enabled() || !self.accept_updates || !attached {
                let _ = call
                    .respond
                    .send(Err("Terminal was detached or chat is unavailable.".into()));
                cx.notify();
                return;
            }
            if grant && let Some(id) = call.call.terminal_id() {
                self.grants.grant(id, call.call.writes());
            }
            self.execute_tool(call, cx);
        }
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
        let resolved = self.resolved(cx);
        if let TerminalCall::ListTerminals = call.call {
            let descriptors: Vec<_> = resolved
                .into_iter()
                .map(|(_, _, descriptor)| descriptor)
                .collect();
            let payload = nocterm_ai::context::context_block(&descriptors);
            let _ = call
                .respond
                .send(Ok(serde_json::json!({"context":payload})));
            return;
        }
        let Some((_, entry, _)) = resolved
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.call.terminal_id())
        else {
            let _ = call
                .respond
                .send(Err("Terminal is not attached or was closed.".into()));
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
                                .ok_or("Terminal was detached or closed.")?;
                            let info = entry.access.info(cx).ok_or("Terminal was closed.")?;
                            if info.status != TerminalStatus::Connected {
                                return Err("Terminal is no longer connected.".into());
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
                            if let Some(reason) = reason {
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
                            } else {
                                Ok(None)
                            }
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
            TerminalCall::ListTerminals => {}
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
