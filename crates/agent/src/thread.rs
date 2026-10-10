use crate::runtime::{Runtime, SessionLease};
use futures::FutureExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, EntityId, Subscription, Task, WeakEntity, Window,
};
use nocterm_ai::{
    AgentInfo, BridgeCall, acp,
    approval::ApprovalGrants,
    context::OpaqueIds,
    session::{SessionPhase, SessionState},
    thread::ThreadState,
};
use nocterm_ui::ActiveAi as _;
use nocterm_workspace::Workspace;
use std::{path::PathBuf, sync::Arc, time::Duration};

mod archive;
mod attachments;
mod client;
mod composer;
mod config;
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
    /// Where the agent session is; every change goes through its transitions.
    pub lifecycle: SessionState,
    /// A panel shows this chat: its idle session is kept warm.
    pub shown: bool,
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
    pub archive: Option<nocterm_ai::history::ChatSummary>,
    pub loading_archive: bool,
    archive_actions: Vec<archive::Loaded>,
    pending_archive_draft: Option<String>,
    resident_archive_bytes: usize,
    pub archive_permit: Option<nocterm_ai::history::HistoryPermit>,
    execution_watch: Option<Task<()>>,
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
    pub info: Option<AgentInfo>,
    pub workspace: WeakEntity<Workspace>,
    /// The window of the panel showing this chat; background sessions open
    /// there.
    pub window: Option<AnyWindowHandle>,
    /// Sessions this chat opened without a tab; closed with the chat.
    pub background: Vec<EntityId>,
    /// Background sessions waiting for the user to sign in from the chat.
    pub sign_ins: Vec<SignInWait>,
    /// Closed server terminals reconnecting for a tool call; calls that
    /// arrive meanwhile wait for the same connection.
    pub(crate) reconnecting: std::collections::HashSet<EntityId>,
    pub prompt_attachments: Option<Vec<Attachment>>,
    pub dirty_rows: std::collections::HashSet<usize>,
    storage_dirty: std::collections::HashSet<usize>,
    storage_budget: nocterm_ai::history::HistoryBudget,
    pub fallback_history: bool,
    pub pending_controls: Vec<(acp::SessionId, acp::SessionUpdate)>,
    /// Configuration chosen before the session opened; applied when it does.
    pub config_choices: nocterm_ai::session_config::Choices,
    pub permissions: Vec<PendingPermission>,
    pub tools: Vec<BridgeCall>,
    /// Advances only when a new request needs user approval.
    pub approval_generation: u64,
    pub context_bytes: usize,
    pub tool_bytes: usize,
    ids: OpaqueIds,
    /// Ids of attached servers without a session, as agents see them.
    server_ids: OpaqueIds,
    grants: ApprovalGrants,
    executions: execution::Jobs,
    live_commands: std::collections::HashMap<String, Arc<nocterm_workspace::LiveCommandLease>>,
    tool_displays: tool_display::ToolDisplays,
    _release: Subscription,
    _workspace_authority: Option<Subscription>,
}
impl AgentThread {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(crate) fn new(
        agent_id: String,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let authority = workspace
            .upgrade()
            .map(|workspace| cx.subscribe(&workspace, |this, _, _, cx| this.revoke_executions(cx)));
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
        let chat = nocterm_ai::history::SavedChat::new(agent_id.clone());
        Self {
            lease: None,
            lifecycle: SessionState::default(),
            shown: false,
            document_revision: 0,
            persisted_revision: 0,
            persistence_error: None,
            operation_count: Default::default(),
            agent_id,
            chat_id: chat.id,
            updated: chat.updated,
            archive: None,
            loading_archive: false,
            archive_actions: Vec::new(),
            archive_permit: None,
            resident_archive_bytes: 0,
            pending_archive_draft: None,
            execution_watch: None,
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
            info: None,
            workspace,
            window: None,
            background: Vec::new(),
            sign_ins: Vec::new(),
            reconnecting: Default::default(),
            prompt_attachments: None,
            dirty_rows: Default::default(),
            storage_dirty: Default::default(),
            storage_budget: Default::default(),
            fallback_history: false,
            pending_controls: Vec::new(),
            config_choices: Default::default(),
            permissions: Vec::new(),
            tools: Vec::new(),
            approval_generation: 0,
            context_bytes: 0,
            tool_bytes: 0,
            ids: Default::default(),
            server_ids: OpaqueIds::with_prefix("s"),
            grants: Default::default(),
            executions: Default::default(),
            live_commands: Default::default(),
            tool_displays: Default::default(),
            _release: release,
            _workspace_authority: authority,
        }
    }
    /// The session ended with an error; the next message reconnects.
    pub(crate) fn ended(&self) -> bool {
        self.lifecycle.failed()
    }
    /// Not connected to an agent: restored from history, idle or failed.
    pub(crate) fn dormant(&self) -> bool {
        self.lease.is_none()
    }
    /// An empty chat nobody has named: dropped when the user moves on.
    pub(crate) fn is_draft(&self) -> bool {
        self.archive.is_none()
            && !self.pinned
            && self.state.entries.is_empty()
            && self.composer.is_empty()
            && self.name.is_none()
            && !self.lifecycle.generating()
            && self.permissions.is_empty()
            && self.tools.is_empty()
    }
    pub(crate) fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.cancel_pending();
        if !self.dormant() {
            self.persist(cx);
        }
        self.lifecycle.fail();
        self.composer.queue_paused = true;
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
        self.lifecycle.require_sign_in();
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
        if !cx.ai_enabled() || self.lifecycle.phase() != SessionPhase::SignInRequired {
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
        self.lifecycle.begin_sign_in();
        self.status = "Signing in…".into();
        self.status_error = false;
        let ticket = self.lifecycle.ticket();
        let guard = self.hold_operation();
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            auth.await
        });
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if !this.lifecycle.session_current(ticket) || !cx.ai_enabled() {
                    return;
                }
                let has_session = this.session().is_some();
                this.lifecycle.sign_in_finished(result.is_ok(), has_session);
                match result {
                    Ok(()) => {
                        this.status_error = false;
                        if has_session {
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
    pub(crate) fn set_mode(&mut self, id: acp::SessionModeId, cx: &mut Context<Self>) {
        if self.lifecycle.generating() || !cx.ai_enabled() {
            return;
        }
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        let ticket = self.lifecycle.ticket();
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
                if !this.lifecycle.session_current(ticket) || !cx.ai_enabled() {
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
        if !self.lifecycle.stop() {
            cx.notify();
            return;
        }
        if let (Some(commands), Some(session)) = (&self.commands(), &self.session()) {
            commands.cancel(session.clone());
        }
        self.status = "Stopping…".into();
        let ticket = self.lifecycle.ticket();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.lifecycle.generating() && this.lifecycle.turn_current(ticket) {
                    this.fail(
                        "Cancellation timed out. The next message reconnects the agent.",
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
                if !self.lifecycle.generating() {
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
