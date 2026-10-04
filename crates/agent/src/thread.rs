use crate::runtime::Runtime;
use futures::{FutureExt as _, channel::oneshot};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, EntityId, Subscription, WeakEntity, Window,
};
use nocterm_ai::{
    AgentCommands, AgentInfo, BridgeCall, BridgeRegistration, acp, approval::ApprovalGrants,
    context::OpaqueIds, thread::ThreadState,
};
use nocterm_ui::ActiveAi as _;
use nocterm_workspace::Workspace;
use std::{path::PathBuf, sync::Arc, time::Duration};

mod saved;
mod tools;
#[cfg(test)]
pub(crate) use saved::chat_title;

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
#[derive(Clone, Debug)]
pub(crate) struct PromptMetadata {
    pub model: Option<String>,
}
/// An agent session from a saved chat, to reopen on the next connection.
#[derive(Clone, Debug)]
pub(crate) struct Restore {
    pub session: acp::SessionId,
    pub workdir: PathBuf,
    /// Open a copy of the session rather than the session itself.
    pub fork: bool,
}
/// What a forked chat starts from.
pub(crate) struct Fork {
    chat: nocterm_ai::history::SavedChat,
    restore: Option<Restore>,
    attachments: Vec<Attachment>,
}
pub(crate) struct AgentThread {
    pub agent_id: String,
    /// Names the chat's file in the saved history.
    pub chat_id: String,
    /// When the chat last changed, in seconds since the Unix epoch.
    pub updated: u64,
    /// Restored from history and not yet connected.
    pub dormant: bool,
    pub restore: Option<Restore>,
    /// The name the user gave the chat.
    pub name: Option<String>,
    pub pinned: bool,
    pub state: ThreadState,
    pub last_prompt: Option<PromptMetadata>,
    /// The model of a saved chat's last prompt.
    pub last_model: Option<String>,
    pub status: String,
    pub status_error: bool,
    pub generating: bool,
    /// The user stopped the last turn. Its late updates are dropped, and the
    /// next prompt goes on in the same session.
    pub stopped: bool,
    pub auth_required: bool,
    pub authenticating: bool,
    session_workdir: Option<PathBuf>,
    pub accept_updates: bool,
    pub session: Option<acp::SessionId>,
    pub commands: Option<Arc<dyn AgentCommands>>,
    pub info: Option<AgentInfo>,
    pub connection_key: Option<u64>,
    pub registration: Option<BridgeRegistration>,
    pub workspace: WeakEntity<Workspace>,
    /// The window of the panel showing this chat; background sessions open
    /// there.
    pub window: Option<AnyWindowHandle>,
    pub attachments: Vec<Attachment>,
    /// Sessions this chat opened without a tab; closed with the chat.
    pub background: Vec<EntityId>,
    pub images: Vec<nocterm_ai::images::PromptImage>,
    pub permissions: Vec<PendingPermission>,
    pub tools: Vec<BridgeCall>,
    pub context_bytes: usize,
    pub tool_bytes: usize,
    pub epoch: u64,
    ids: OpaqueIds,
    /// Ids of attached servers without a session, as agents see them.
    server_ids: OpaqueIds,
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
            if let Some(window) = this.window {
                let workspace = this.workspace.clone();
                let background = std::mem::take(&mut this.background);
                cx.defer(move |cx| {
                    for item in background {
                        let _ = cx.update_window(window, |_, window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.close_background_session(item, window, cx)
                            });
                        });
                    }
                });
            }
        });
        let chat = nocterm_ai::history::SavedChat::new(agent_id.clone());
        Self {
            agent_id,
            chat_id: chat.id,
            updated: chat.updated,
            dormant: false,
            restore: None,
            name: None,
            pinned: false,
            state: Default::default(),
            last_prompt: None,
            last_model: None,
            status: "Connecting…".into(),
            status_error: false,
            generating: false,
            stopped: false,
            auth_required: false,
            authenticating: false,
            session_workdir: None,
            accept_updates: true,
            session: None,
            commands: None,
            info: None,
            connection_key: None,
            registration: None,
            workspace,
            window: None,
            attachments: Vec::new(),
            background: Vec::new(),
            images: Vec::new(),
            permissions: Vec::new(),
            tools: Vec::new(),
            context_bytes: 0,
            tool_bytes: 0,
            epoch: 0,
            ids: Default::default(),
            server_ids: OpaqueIds::with_prefix("s"),
            grants: Default::default(),
            _release: release,
        }
    }
    /// The session is gone: the chat must be restarted to go on.
    pub(crate) fn ended(&self) -> bool {
        !self.accept_updates && !self.stopped
    }
    /// An empty chat nobody has named: dropped when the user moves on.
    pub(crate) fn is_draft(&self) -> bool {
        self.state.entries.is_empty()
            && self.name.is_none()
            && !self.generating
            && self.permissions.is_empty()
            && self.tools.is_empty()
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
        let server = acp::McpServerStdio::new(bridge_server_name(registration.id), binary)
            .args(vec!["agent-bridge".into()])
            .env(vec![
                acp::EnvVariable::new("NOCTERM_BRIDGE_ENDPOINT", registration.endpoint.clone()),
                acp::EnvVariable::new("NOCTERM_BRIDGE_TOKEN", registration.token.clone()),
            ]);
        self.commands = Some(commands.clone());
        self.info = Some(info);
        self.status = "Starting chat…".into();
        self.status_error = false;
        self.auth_required = false;
        let restore = self
            .restore
            .take()
            .filter(|restore| restore.workdir.is_absolute());
        self.session_workdir = Some(
            restore
                .as_ref()
                .map_or_else(|| workdir.clone(), |restore| restore.workdir.clone()),
        );
        let epoch = self.epoch;
        let servers = vec![acp::McpServer::Stdio(server)];
        let session_commands = commands.clone();
        let pending_restore = restore.clone();
        let future = cx.background_executor().spawn(async move {
            // A saved chat reopens its session so the agent remembers it;
            // failing that, the chat goes on in a new session.
            if let Some(restore) = &restore {
                match session_commands
                    .restore_session(nocterm_ai::RestoreSessionRequest {
                        session_id: restore.session.clone(),
                        cwd: restore.workdir.clone(),
                        mcp_servers: servers.clone(),
                        fork: restore.fork,
                    })
                    .await
                {
                    Ok(response) => return (Ok(response), Some(true), workdir),
                    Err(error @ nocterm_ai::AgentError::AuthRequired(_)) => {
                        return (Err(error), None, workdir);
                    }
                    Err(error) => {
                        tracing::debug!(%error, "could not reopen agent session");
                    }
                }
            }
            let result = session_commands
                .new_session(acp::NewSessionRequest::new(workdir.clone()).mcp_servers(servers))
                .await;
            (result, restore.is_some().then_some(false), workdir)
        });
        cx.spawn(async move |this, cx| {
            let (result, restored, workdir) = future.await;
            let _ = this.update(cx, |this, cx| {
                if restored == Some(false) {
                    // The new session lives in the connection's directory.
                    this.session_workdir = Some(workdir);
                }
                if restored.is_none() && result.is_err() {
                    // Kept for after signing in.
                    this.restore = pending_restore;
                }
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
                        this.auth_required = false;
                        this.status = if restored == Some(false) {
                            "Started a new agent session: the agent does not remember the messages above.".into()
                        } else {
                            "Ready".into()
                        };
                        cx.notify();
                    }
                    Err(nocterm_ai::AgentError::AuthRequired(message)) => {
                        this.require_authentication(&message, cx);
                    }
                    Err(error) => this.fail(&error.to_string(), cx),
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.finish_pending_tools();
        if !self.dormant {
            self.persist(cx);
        }
        self.epoch += 1;
        self.accept_updates = false;
        self.stopped = false;
        self.generating = false;
        self.auth_required = false;
        self.authenticating = false;
        self.status = nocterm_ai::redact::redact(message);
        self.status_error = true;
        self.cancel_pending();
        // A grant belongs to this session; a restarted chat asks again.
        self.grants.clear();
        self.registration.take();
        cx.notify();
    }
    fn require_authentication(&mut self, message: &str, cx: &mut Context<Self>) {
        self.finish_pending_tools();
        self.auth_required = true;
        self.authenticating = false;
        self.generating = false;
        self.status = nocterm_ai::redact::redact(message);
        self.status_error = true;
        self.cancel_pending();
        cx.notify();
    }
    pub(crate) fn authenticate(
        &mut self,
        method: acp::AuthMethodId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() || !self.auth_required || self.authenticating {
            return;
        }
        let Some(commands) = self.commands.clone() else {
            return;
        };
        let Some(method) = self
            .info
            .as_ref()
            .and_then(|info| {
                info.auth_methods
                    .iter()
                    .find(|available| available.id() == &method)
            })
            .cloned()
        else {
            return;
        };
        let auth = match method {
            acp::AuthMethod::Agent(method) => commands.authenticate(method.id),
            acp::AuthMethod::Terminal(method) => {
                let runtime = Runtime::global(cx);
                let Some(opener) = runtime.read(cx).services.terminal_auth.clone() else {
                    return;
                };
                let Some(request) = self
                    .connection_key
                    .and_then(|key| runtime.read(cx).terminal_auth_request(key, &method))
                else {
                    return;
                };
                let completion = opener(self.workspace.clone(), request, window, cx);
                async move {
                    completion
                        .await
                        .map_err(|_| nocterm_ai::AgentError::Io("Sign-in cancelled".into()))?
                        .map_err(nocterm_ai::AgentError::Io)
                }
                .boxed()
            }
            _ => return,
        };
        self.authenticating = true;
        self.status = "Signing in…".into();
        self.status_error = false;
        let epoch = self.epoch;
        let future = cx.background_executor().spawn(auth);
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || !cx.ai_enabled() {
                    return;
                }
                this.authenticating = false;
                match result {
                    Ok(()) => {
                        this.auth_required = false;
                        this.status_error = false;
                        if this.session.is_some() {
                            this.status = "Ready".into();
                            cx.notify();
                        } else if let (Some(info), Some(workdir)) =
                            (this.info.clone(), this.session_workdir.clone())
                        {
                            this.create_session(commands, info, workdir, cx);
                        }
                    }
                    Err(error) => {
                        this.status = nocterm_ai::redact::redact(&error.to_string());
                        this.status_error = true;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn finish_pending_tools(&mut self) {
        for entry in &mut self.state.entries {
            if let nocterm_ai::thread::Entry::Tool(call) = entry
                && matches!(
                    call.status,
                    acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress
                )
            {
                call.status = acp::ToolCallStatus::Failed;
            }
        }
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
    pub(crate) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if !cx.ai_enabled() || self.generating || self.auth_required || self.ended() {
            return;
        }
        let (Some(commands), Some(session)) = (self.commands.clone(), self.session.clone()) else {
            return;
        };
        if text.trim().is_empty() && self.images.is_empty() {
            return;
        }
        // A new turn: updates count again.
        self.stopped = false;
        self.accept_updates = true;
        let context = format!(
            "{}\n{}",
            nocterm_ai::context::TERMINAL_RULES,
            self.context(cx)
        );
        self.context_bytes += context.len();
        let mut visible = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
        visible.extend(self.images.drain(..).map(|image| image.content()));
        self.state.push_user(visible.clone());
        self.persist(cx);
        let mut content = vec![acp::ContentBlock::Text(acp::TextContent::new(context))];
        content.extend(visible);
        self.last_prompt = Some(PromptMetadata {
            model: self
                .state
                .config_options
                .iter()
                .find(|option| option.category == Some(acp::SessionConfigOptionCategory::Model))
                .map(config_label)
                .filter(|model| !model.is_empty()),
        });
        self.generating = true;
        self.status = "Working…".into();
        self.status_error = false;
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
                this.persist(cx);
                match result {
                    Ok(response) => {
                        if response.usage.is_some() {
                            this.state.tokens = response.usage;
                        }
                        // Agents that keep limits in their own files have
                        // just written this turn's.
                        let agent = this.agent_id.clone();
                        cx.defer(move |cx| {
                            Runtime::global(cx)
                                .update(cx, |runtime, cx| runtime.refresh_limits(&agent, cx))
                        });
                        this.status = if this.stopped {
                            "Stopped".into()
                        } else {
                            format!("{:?}", response.stop_reason)
                        };
                        cx.notify();
                    }
                    Err(nocterm_ai::AgentError::AuthRequired(message)) => {
                        this.require_authentication(&message, cx);
                    }
                    // Some agents answer a cancelled prompt with an error; the
                    // session itself goes on.
                    Err(error) if this.stopped => {
                        tracing::debug!(%error, "stopped prompt ended with an error");
                        this.status = "Stopped".into();
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
                    Ok(options) => {
                        this.state.config_options = options;
                        this.status_error = false;
                    }
                    Err(error) => {
                        this.status = nocterm_ai::redact::redact(&error.to_string());
                        this.status_error = true;
                    }
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
                    Ok(()) => {
                        this.state.current_mode = Some(id);
                        this.status_error = false;
                    }
                    Err(error) => {
                        this.status = nocterm_ai::redact::redact(&error.to_string());
                        this.status_error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Cancels the turn in progress. The session stays open for the next
    /// prompt.
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.finish_pending_tools();
        self.cancel_pending();
        if !self.generating {
            cx.notify();
            return;
        }
        if let (Some(commands), Some(session)) = (&self.commands, &self.session) {
            commands.cancel(session.clone());
        }
        self.accept_updates = false;
        self.stopped = true;
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
            self.prune_background(cx);
        } else {
            self.attachments.push(attachment);
        }
        cx.notify();
    }
}

/// The name of a chat's terminal tools server. Each chat's is unique: agents
/// such as Hermes keep one server per name for the whole process, so a shared
/// name would send every chat's calls to the first chat's bridge.
pub(crate) fn bridge_server_name(registration: u64) -> String {
    format!("nocterm-{registration}")
}

pub(crate) fn config_label(option: &acp::SessionConfigOption) -> String {
    match &option.kind {
        acp::SessionConfigKind::Select(select) => {
            let options = match &select.options {
                acp::SessionConfigSelectOptions::Ungrouped(values) => {
                    values.iter().collect::<Vec<_>>()
                }
                acp::SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| group.options.iter())
                    .collect(),
                _ => Vec::new(),
            };
            options
                .iter()
                .find(|value| value.value == select.current_value)
                .map(|value| value.name.clone())
                .unwrap_or_else(|| select.current_value.to_string())
        }
        acp::SessionConfigKind::Boolean(value) => {
            if value.current_value {
                "On".into()
            } else {
                "Off".into()
            }
        }
        _ => String::new(),
    }
}
