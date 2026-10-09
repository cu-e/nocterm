use crate::runtime::{Runtime, SessionLease};
use futures::FutureExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, EntityId, Subscription, Task, WeakEntity, Window,
};
use nocterm_ai::{
    AgentInfo, BridgeCall, acp, approval::ApprovalGrants, context::OpaqueIds, thread::ThreadState,
};
use nocterm_ui::ActiveAi as _;
use nocterm_workspace::Workspace;
use std::{path::PathBuf, sync::Arc, time::Duration};

mod attachments;
mod client;
mod composer;
pub(crate) use client::client;
pub(crate) use composer::Composer;
mod execution;
mod permissions;
mod prompt;
mod queue;
mod tool_display;
pub(crate) use queue::QueuedPrompt;
mod lifecycle;
mod live;
mod restart;
mod saved;
mod session;
mod tool_context;
mod tools;
mod updates;
#[cfg(test)]
pub(crate) use saved::chat_title;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Attachment {
    Terminal(EntityId),
    Connection(String),
    Group(String),
    UnavailableLocal(String),
}
/// A background session asks for a secret. The user answers in the chat, or
/// unlocks the vault when a saved secret answers it; no tab opens.
#[derive(Clone)]
pub(crate) struct SignInWait {
    pub item: EntityId,
    pub title: gpui_kit::SharedString,
    pub access: std::rc::Rc<dyn nocterm_workspace::TerminalAccess>,
    pub prompt: nocterm_workspace::SignInPrompt,
    /// Unlocking the vault answers the prompt with a saved secret.
    pub vault: bool,
}
pub(crate) struct PendingPermission {
    pub request: acp::RequestPermissionRequest,
    pub respond: nocterm_ai::PermissionResponder,
    pub generation: u64,
    pub explanation: Option<&'static str>,
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
    pub lease: Option<SessionLease>,
    pub activation_pending: bool,
    pub closing_session: bool,
    pub document_revision: u64,
    pub persisted_revision: u64,
    pub persistence_error: Option<String>,
    pub operation_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
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
    /// The input state, kept apart from the conversation.
    pub composer: Composer,
    draft_changed: bool,
    /// A snapshot was queued for history, or this chat was restored.
    history_queued: bool,
    draft_save: Option<Task<()>>,
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
    pub accept_updates: bool,
    pub info: Option<AgentInfo>,
    pub workspace: WeakEntity<Workspace>,
    /// The window of the panel showing this chat; background sessions open
    /// there.
    pub window: Option<AnyWindowHandle>,
    /// Sessions this chat opened without a tab; closed with the chat.
    pub background: Vec<EntityId>,
    /// Background sessions waiting for the user to sign in from the chat.
    pub sign_ins: Vec<SignInWait>,
    pub prompt_attachments: Option<Vec<Attachment>>,
    pub dirty_rows: std::collections::HashSet<usize>,
    storage_dirty: std::collections::HashSet<usize>,
    storage_budget: nocterm_ai::history::HistoryBudget,
    pub fallback_history: bool,
    pub pending_controls: Vec<(acp::SessionId, acp::SessionUpdate)>,
    pub connecting_session: bool,
    pub permissions: Vec<PendingPermission>,
    pub tools: Vec<BridgeCall>,
    /// Advances only when a new request needs user approval.
    pub approval_generation: u64,
    pub context_bytes: usize,
    pub tool_bytes: usize,
    pub epoch: u64,
    pub turn: u64,
    ids: OpaqueIds,
    /// Ids of attached servers without a session, as agents see them.
    server_ids: OpaqueIds,
    grants: ApprovalGrants,
    executions: execution::Jobs,
    live_commands: std::collections::HashMap<String, Arc<nocterm_workspace::LiveCommandLease>>,
    tool_displays: tool_display::ToolDisplays,
    _release: Subscription,
}
impl AgentThread {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(crate) fn new(
        agent_id: String,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = cx.entity_id();
        let release = cx.on_release(move |this, cx| {
            this.cancel_pending();
            this.finalize_tool_displays();
            let snapshot = cx.ai_enabled().then(|| this.flush_snapshot(cx)).flatten();
            this.document_revision += 1;
            let revision = this.document_revision;
            this.cancel_execution();
            // The lease releases its connection when it is dropped.
            drop(this.lease.take());
            cx.defer(move |cx| {
                Runtime::global(cx).update(cx, |runtime, cx| {
                    if let Some(chat) = snapshot {
                        runtime.save_chat(chat, id, revision, cx);
                    }
                    runtime.unregister_document(id);
                });
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
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if !cx.ai_enabled() {
                            this.cancel_execution();
                        } else {
                            this.revoke_executions(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let chat = nocterm_ai::history::SavedChat::new(agent_id.clone());
        Self {
            lease: None,
            activation_pending: false,
            closing_session: false,
            document_revision: 0,
            persisted_revision: 0,
            persistence_error: None,
            operation_count: Default::default(),
            agent_id,
            chat_id: chat.id,
            updated: chat.updated,
            dormant: true,
            restore: None,
            name: None,
            pinned: false,
            composer: Composer::default(),
            draft_changed: false,
            history_queued: false,
            draft_save: None,
            state: Default::default(),
            last_prompt: None,
            last_model: None,
            status: "Ready — send a message to start the agent".into(),
            status_error: false,
            generating: false,
            stopped: false,
            auth_required: false,
            authenticating: false,
            accept_updates: true,
            info: None,
            workspace,
            window: None,
            background: Vec::new(),
            sign_ins: Vec::new(),
            prompt_attachments: None,
            dirty_rows: Default::default(),
            storage_dirty: Default::default(),
            storage_budget: Default::default(),
            fallback_history: false,
            pending_controls: Vec::new(),
            connecting_session: false,
            permissions: Vec::new(),
            tools: Vec::new(),
            approval_generation: 0,
            context_bytes: 0,
            tool_bytes: 0,
            epoch: 0,
            turn: 0,
            ids: Default::default(),
            server_ids: OpaqueIds::with_prefix("s"),
            grants: Default::default(),
            executions: Default::default(),
            live_commands: Default::default(),
            tool_displays: Default::default(),
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
            && self.composer.is_empty()
            && self.name.is_none()
            && !self.generating
            && self.permissions.is_empty()
            && self.tools.is_empty()
    }
    pub(crate) fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.cancel_pending();
        if !self.dormant {
            self.persist(cx);
        }
        self.epoch += 1;
        self.accept_updates = false;
        self.stopped = false;
        self.generating = false;
        self.auth_required = false;
        self.authenticating = false;
        self.composer.queue_paused = true;
        self.connecting_session = false;
        self.status = nocterm_ai::redact::redact(message);
        self.status_error = true;
        self.cancel_pending();
        // A grant belongs to this session; a restarted chat asks again.
        self.grants.clear();
        self.detach_session(cx);
        cx.notify();
    }
    fn require_authentication(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.composer.queue_paused = true;
        self.auth_required = true;
        self.authenticating = false;
        self.generating = false;
        self.status = nocterm_ai::redact::redact(message);
        self.status_error = true;
        self.cancel_pending();
        cx.notify();
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(crate) fn authenticate(
        &mut self,
        method: acp::AuthMethodId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() || !self.auth_required || self.authenticating {
            return;
        }
        let Some(commands) = self.commands().clone() else {
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
                let Some(opener) = crate::TerminalAuth::opener(cx) else {
                    return;
                };
                let Some(request) = self
                    .connection_key()
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
        let guard = self.hold_operation();
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            auth.await
        });
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
                        if this.session().is_some() {
                            this.status = "Ready".into();
                            cx.notify();
                        } else if let (Some(info), Some(workdir)) =
                            (this.info.clone(), this.session_workdir().clone())
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
    pub(crate) fn mark_dirty(&mut self, index: usize) {
        self.dirty_rows.insert(index);
        self.storage_dirty.insert(index);
    }
    pub(super) fn refresh_storage_budget(&mut self) -> Result<(), String> {
        let dirty = self.storage_dirty.drain().collect::<Vec<_>>();
        self.storage_budget.refresh(&self.state.entries, &dirty)
    }
    fn finish_pending_tools(&mut self) {
        self.finalize_tool_displays();
        for (index, entry) in self.state.entries.iter_mut().enumerate() {
            if let nocterm_ai::thread::Entry::Tool(call) = entry
                && matches!(
                    call.status,
                    acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress
                )
            {
                call.status = acp::ToolCallStatus::Failed;
                self.dirty_rows.insert(index);
                self.storage_dirty.insert(index);
            }
        }
    }
    fn cancel_pending(&mut self) {
        // Dropped responders answer `Cancelled`.
        self.permissions.clear();
        for call in std::mem::take(&mut self.tools) {
            self.finish(call, Err("Request cancelled.".into()));
        }
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
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        let epoch = self.epoch;
        let guard = self.hold_operation();
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            commands
                .set_config_option(acp::SetSessionConfigOptionRequest::new(session, id, value))
                .await
        });
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
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        let epoch = self.epoch;
        let guard = self.hold_operation();
        let request_id = id.clone();
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            commands
                .set_mode(acp::SetSessionModeRequest::new(session, request_id))
                .await
        });
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
        self.cancel_execution();
        self.composer.queue_paused = true;
        self.finish_pending_tools();
        self.cancel_pending();
        self.stopped = true;
        if !self.generating {
            cx.notify();
            return;
        }
        if let (Some(commands), Some(session)) = (&self.commands(), &self.session()) {
            commands.cancel(session.clone());
        }
        self.accept_updates = false;
        self.status = "Stopping…".into();
        let epoch = self.epoch;
        let turn = self.turn;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generating && this.epoch == epoch && this.turn == turn {
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
            .composer
            .attachments
            .iter()
            .position(|value| value == &attachment)
        {
            self.composer.attachments.remove(index);
            if !self.composer.editing() {
                if let Some(attached) = &mut self.prompt_attachments {
                    attached.retain(|value| value != &attachment);
                }
                self.grants.clear();
                self.cancel_pending();
                self.revoke_executions(cx);
                if !self.generating {
                    self.prune_background(cx);
                }
            }
        } else {
            self.composer.attachments.push(attachment);
        }
        self.save(cx);
        cx.notify();
    }
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

mod drafts;
