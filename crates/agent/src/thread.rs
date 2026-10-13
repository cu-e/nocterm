use crate::runtime::{Runtime, SessionLease};
use futures::FutureExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, EntityId, Subscription, Task, WeakEntity, Window,
};
use nocterm_ai::{
    AgentInfo, acp,
    context::OpaqueIds,
    session::{SessionPhase, SessionState},
    thread::ThreadState,
};
use nocterm_ui::ActiveAi as _;
use nocterm_workspace::Workspace;
use std::{path::PathBuf, sync::Arc, time::Duration};

pub(crate) mod archive;
mod attachments;
mod client;
mod composer;
mod config;
pub(crate) use client::client;
pub(crate) use composer::Composer;
pub(crate) use config::ChatConfig;
mod approval_inbox;
mod background_open;
mod bound_call;
mod connection_wait;
mod execution;
mod permissions;
mod reconnects;
mod session_grants;
use approval_inbox::ApprovalInbox;
pub(crate) use approval_inbox::ToolApprovalCard;
pub(crate) use permissions::PermissionCard;
mod prompt;
mod queue;
mod tool_display;
pub(crate) use queue::QueuedPrompt;
mod lifecycle;
mod live;
mod restart;
pub(crate) use restart::RestartData;
mod document;
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
    prompts: std::rc::Rc<dyn nocterm_workspace::SignInPrompts>,
    pub prompt: nocterm_workspace::SignInPrompt,
    /// Unlocking the vault answers the prompt with a saved secret.
    pub vault: bool,
}
impl SignInWait {
    pub(crate) fn new(
        item: EntityId,
        title: gpui_kit::SharedString,
        prompts: std::rc::Rc<dyn nocterm_workspace::SignInPrompts>,
        prompt: nocterm_workspace::SignInPrompt,
        vault: bool,
    ) -> Self {
        Self {
            item,
            title,
            prompts,
            prompt,
            vault,
        }
    }
    pub(crate) fn prompts(&self) -> std::rc::Rc<dyn nocterm_workspace::SignInPrompts> {
        self.prompts.clone()
    }
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
    pub operation_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    archive: archive::Archive,
    execution_watch: Option<Task<()>>,
    pub restore: Option<Restore>,
    /// The input state, kept apart from the conversation.
    composer: Composer,
    config: config::ChatConfig,
    /// Private durable metadata, transcript and semantic persistence owner.
    state: document::ChatDocument,
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
    reconnecting: reconnects::Reconnects,
    pub prompt_attachments: Option<Vec<Attachment>>,
    approvals: ApprovalInbox,
    pub context_bytes: usize,
    pub tool_bytes: usize,
    ids: OpaqueIds,
    /// Ids of attached servers without a session, as agents see them.
    server_ids: OpaqueIds,
    executions: execution::Jobs,
    live_commands: std::collections::HashMap<String, Arc<nocterm_workspace::LiveCommandLease>>,
    tool_displays: tool_display::ToolDisplays,
    _release: Subscription,
    _workspace_authority: Option<Subscription>,
}
impl AgentThread {
    fn from_document(
        state: document::ChatDocument,
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
            let snapshot = cx.ai_enabled().then(|| this.capture_pending(cx)).flatten();
            this.cancel_execution();
            // The lease releases its connection when it is dropped.
            drop(this.lease.take());
            cx.defer(move |cx| {
                Runtime::global(cx).update(cx, |runtime, cx| {
                    if let Some((chat, revision)) = snapshot
                        && let Err(error) = runtime.save_chat(chat, id, revision, cx)
                    {
                        tracing::warn!(%error,"released chat admission failed");
                    }
                    runtime.unregister_document(id, cx);
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
        Self {
            lease: None,
            lifecycle: SessionState::default(),
            shown: false,
            operation_count: Default::default(),
            archive: archive::Archive::default(),
            execution_watch: None,
            restore: None,
            composer: Composer::default(),
            state,
            config: Default::default(),
            status: "Ready — send a message to start the agent".into(),
            status_error: false,
            info: None,
            workspace,
            window: None,
            background: Vec::new(),
            sign_ins: Vec::new(),
            reconnecting: Default::default(),
            prompt_attachments: None,
            approvals: ApprovalInbox::default(),
            context_bytes: 0,
            tool_bytes: 0,
            ids: Default::default(),
            server_ids: OpaqueIds::with_prefix("s"),
            executions: Default::default(),
            live_commands: Default::default(),
            tool_displays: Default::default(),
            _release: release,
            _workspace_authority: authority,
        }
    }
    pub(crate) fn new(
        agent_id: String,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let chat = nocterm_ai::history::SavedChat::new(agent_id);
        Self::from_document(
            document::ChatDocument::new(chat.agent_id, chat.id, chat.updated),
            workspace,
            cx,
        )
    }
    pub(crate) fn archived(
        summary: nocterm_ai::history::ChatSummary,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut state = document::ChatDocument::new(
            summary.agent_id.clone(),
            summary.id.clone(),
            summary.updated,
        );
        state.archived_clean(&summary);
        let mut thread = Self::from_document(state, workspace, cx);
        thread.config.saved_model(summary.model.clone());
        thread.status = "Saved chat".into();
        thread.archive = archive::Archive::archived(summary);
        thread
    }
    pub(crate) fn is_archived(&self) -> bool {
        self.archive.is_archived()
    }
    pub(crate) fn archive_summary(&self) -> Option<&nocterm_ai::history::ChatSummary> {
        self.archive.summary()
    }
    pub(crate) fn chat_id(&self) -> &str {
        self.state.chat_id()
    }
    pub(crate) fn history_entries(&self) -> &[nocterm_ai::thread::Entry] {
        self.state.entries()
    }
    pub(crate) fn take_changed_rows(&mut self) -> std::collections::HashSet<usize> {
        self.state.take_changed_rows()
    }
    pub(crate) fn document(&self) -> &document::ChatDocument {
        &self.state
    }
    pub(crate) fn transcript(&self) -> &ThreadState {
        self.state.transcript()
    }
    pub(crate) fn composer(&self) -> &Composer {
        &self.composer
    }
    pub(crate) fn configuration(&self) -> &config::ChatConfig {
        &self.config
    }
    pub(crate) fn config_options(&self) -> Vec<acp::SessionConfigOption> {
        self.config.preview_options(self.lifecycle.opening())
    }
    pub(crate) fn draft_text(&self) -> Option<&str> {
        self.composer.draft().map(String::as_str)
    }
    pub(crate) fn queued_prompt_ids(&self) -> impl ExactSizeIterator<Item = u64> + '_ {
        self.composer.queue().iter().map(|prompt| prompt.saved.id)
    }
    pub(crate) fn queue_dispatchable(&self) -> bool {
        self.composer.dispatchable()
    }
    pub(crate) fn session_phase(&self) -> SessionPhase {
        self.lifecycle.phase()
    }
    pub(crate) fn updated_at(&self) -> u64 {
        self.state.updated()
    }
    pub(crate) fn has_status_error(&self) -> bool {
        self.status_error
    }
    pub(crate) fn release_history_delivery(&mut self) {
        self.archive.release_delivery_permit();
    }
    pub(crate) fn load_archive(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<Self>) {
        let request = self.archive.request(intent, self.lifecycle.ticket());
        let archive::RequestOutcome::Start {
            load_id,
            ticket,
            chat_id,
        } = request
        else {
            if let archive::RequestOutcome::RunNow(intent) = request {
                self.apply_archive_intent(intent, cx);
            }
            return;
        };
        self.status = "Loading saved chat…".into();
        let reader = Runtime::global(cx).read(cx).history_reader();
        let loading = cx
            .background_executor()
            .spawn(async move { reader.load(chat_id).await });
        cx.spawn(async move |this, cx| {
            let (result, permit) = loading.await;
            let _ = this.update(cx, |this, cx| {
                let current = this.lifecycle.session_current(ticket)
                    && cx.ai_enabled()
                    && Runtime::global(cx)
                        .read(cx)
                        .document_is_current(this.state.chat_id(), cx.entity_id());
                match this.archive.accept(load_id, current, result, permit) {
                    archive::LoadOutcome::Stale => return,
                    archive::LoadOutcome::Failed(error) => {
                        this.status = format!("Could not load saved chat: {error}");
                        this.status_error = true;
                    }
                    archive::LoadOutcome::Hydrate {
                        chat,
                        overlay,
                        intents,
                    } => {
                        this.restore_content_with_overlay(*chat, overlay);
                        this.preview_config(cx);
                        if this.state.draft_changed() {
                            this.save(cx);
                        }
                        for intent in intents {
                            this.apply_archive_intent(intent, cx);
                        }
                        this.warm(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply_archive_intent(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<Self>) {
        match intent {
            archive::ArchiveIntent::Rename { name } => self.rename(name, cx),
            archive::ArchiveIntent::Pin { pinned } => self.set_pinned(pinned, cx),
            archive::ArchiveIntent::Open {
                panel,
                source,
                revision,
            } => cx.defer(move |cx| {
                let _ = panel.update(cx, |panel, cx| panel.archive_opened(source, revision, cx));
            }),
            archive::ArchiveIntent::Fork {
                panel,
                source,
                window,
                last,
            } => cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.fork_loaded_archive(source, last, window, cx)
                    });
                });
            }),
        }
    }
    pub(crate) fn resident_history_bytes(&self) -> usize {
        if self.is_archived() {
            return 0;
        }
        self.archive
            .resident_bytes()
            .max(self.state.resident_bytes())
            + self
                .composer
                .queue()
                .iter()
                .map(|prompt| prompt.memory_len)
                .sum::<usize>()
            + self.composer.draft().map_or(0, String::len)
    }
    pub(crate) fn evict_history(&mut self, cx: &Context<Self>) -> bool {
        let gate = archive::EvictionGate::new(
            self.lease.is_none(),
            !self.session_busy(),
            self.state.desired_revision() == self.state.published_revision(),
            !self.state.draft_changed(),
            self.state.persistence_error().is_none(),
            self.state.history_present(),
        );
        let mut summary = nocterm_ai::history::ChatSummary::from_chat(&self.history_metadata(cx));
        summary.title = Some(self.title());
        summary.has_content = !self.state.entries().is_empty() || !self.composer.is_empty();
        if !self.archive.request_evict(gate, summary) {
            return false;
        }
        self.state.evict_clean();
        self.composer = Default::default();
        self.tool_displays = Default::default();
        true
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
        !self.is_archived()
            && !self.state.pinned()
            && self.state.entries().is_empty()
            && self.composer.is_empty()
            && self.state.name().is_none()
            && !self.lifecycle.generating()
            && !self.approvals.pending()
    }
    pub(crate) fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.cancel_pending();
        if !self.dormant() {
            self.persist(cx);
        }
        self.lifecycle.fail();
        self.composer.pause();
        self.status = nocterm_ai::redact::redact(message);
        self.status_error = true;
        self.cancel_pending();
        // A grant belongs to this session; a restarted chat asks again.
        self.approvals.revoke_scope();
        self.detach_session(cx);
        cx.notify();
    }
    fn require_authentication(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.composer.pause();
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
        if !cx.ai_enabled() || self.session_phase() != SessionPhase::SignInRequired {
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
                        .map_err(|e| nocterm_ai::AgentError::Io(e.to_string()))
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
        self.state.mark_row(index);
    }
    pub(super) fn refresh_storage_budget(
        &mut self,
    ) -> Result<(), nocterm_ai::history::HistoryCodecError> {
        self.state.refresh_budget()
    }
    fn finish_pending_tools(&mut self) {
        self.finalize_tool_displays();
        self.state.finish_pending_tools(nocterm_ai::time::now());
    }
    fn cancel_pending(&mut self) {
        // Dropped responders answer `Cancelled`.
        for call in self.approvals.cancel_pending() {
            self.finish(call, Err("Request cancelled.".into()));
        }
    }
    /// Cancels the turn in progress. The session stays open for the next
    /// prompt.
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.composer.pause();
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
        if self.composer.toggle_attachment(attachment.clone()) && !self.composer.editing() {
            if let Some(attached) = &mut self.prompt_attachments {
                attached.retain(|value| value != &attachment);
            }
            self.approvals.revoke_scope();
            self.cancel_pending();
            self.revoke_executions(cx);
            if !self.lifecycle.generating() {
                self.prune_background(cx);
            }
        }
        self.accept_document_inputs(cx, false);
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
