use crate::runtime::Runtime;
use futures::FutureExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, EntityId, Subscription, Window};
use nocterm_ai::{AgentInfo, acp, session::SessionPhase, thread::ThreadState};
use nocterm_ui::ActiveAi as _;
use std::{path::PathBuf, sync::Arc};

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
pub(crate) use approval_inbox::ToolApprovalCard;
pub(crate) use permissions::PermissionCard;
mod prompt;
mod queue;
mod tool_display;
pub(crate) use queue::QueuedPrompt;
mod lifecycle;
mod live;
mod restart;
mod session_driver;
mod tool_runtime;
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
    session: session_driver::SessionDriver,
    tools: tool_runtime::ToolRuntime,
    archive: archive::Archive,
    composer: Composer,
    config: config::ChatConfig,
    state: document::ChatDocument,
    status: String,
    status_error: bool,
    host: crate::chat_host::ChatHost,
    window: Option<AnyWindowHandle>,
    _release: Subscription,
    _workspace_authority: Option<Subscription>,
}

impl AgentThread {
    fn from_document(
        state: document::ChatDocument,
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let authority = host
            .workspace()
            .upgrade()
            .map(|workspace| cx.subscribe(&workspace, |this, _, _, cx| this.revoke_executions(cx)));
        let id = cx.entity_id();
        let release = cx.on_release(move |this, cx| {
            this.cancel_pending();
            this.finalize_tool_displays();
            let snapshot = cx.ai_enabled().then(|| this.capture_pending(cx)).flatten();
            this.cancel_execution();
            // The lease releases its connection when it is dropped.
            this.session.release_lease();
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
                let workspace = this.host.workspace().clone();
                let background = this.tools.take_close_effects();
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
            session: Default::default(),
            tools: Default::default(),
            archive: Default::default(),
            composer: Default::default(),
            state,
            config: Default::default(),
            status: "Ready — send a message to start the agent".into(),
            status_error: false,
            host,
            window: None,
            _release: release,
            _workspace_authority: authority,
        }
    }
    pub(crate) fn new(
        agent_id: String,
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let chat = host.new_chat(agent_id);
        Self::from_document(
            document::ChatDocument::new(chat.agent_id, chat.id, chat.updated),
            host,
            cx,
        )
    }
    pub(crate) fn archived(
        summary: nocterm_ai::history::ChatSummary,
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut state = document::ChatDocument::new(
            summary.agent_id.clone(),
            summary.id.clone(),
            summary.updated,
        );
        state.archived_clean(&summary);
        let mut thread = Self::from_document(state, host, cx);
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
        self.config.preview_options(self.session.opening())
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
        self.session.phase()
    }
    pub(crate) fn updated_at(&self) -> u64 {
        self.state.updated()
    }
    pub(crate) fn status(&self) -> String {
        self.status.clone()
    }
    pub(crate) fn bind_window(&mut self, window: AnyWindowHandle) {
        self.window = Some(window);
    }
    pub(crate) fn window(&self) -> Option<AnyWindowHandle> {
        self.window
    }
    #[cfg(test)]
    pub(crate) fn is_shown(&self) -> bool {
        self.session.shown()
    }
    #[cfg(test)]
    pub(crate) fn is_leased(&self) -> bool {
        self.session.leased()
    }
    pub(crate) fn agent_info(&self) -> Option<&AgentInfo> {
        self.session.info()
    }
    #[cfg(test)]
    pub(crate) fn pending_restore(&self) -> &Option<Restore> {
        self.session.restore()
    }
    #[cfg(test)]
    pub(crate) fn background_items(&self) -> &[EntityId] {
        self.tools.background()
    }
    pub(crate) fn sign_in_waits(&self) -> &[SignInWait] {
        self.tools.sign_ins()
    }
    #[cfg(test)]
    pub(crate) fn prompt_scope(&self) -> Option<&[Attachment]> {
        self.tools.prompt_scope()
    }
    pub(crate) fn context_bytes(&self) -> usize {
        self.tools.usage().0
    }
    pub(crate) fn tool_bytes(&self) -> usize {
        self.tools.usage().1
    }
    pub(crate) fn session_generating(&self) -> bool {
        self.session.generating()
    }
    pub(crate) fn session_accepts_updates(&self) -> bool {
        self.session.accepts_updates()
    }
    #[cfg(test)]
    pub(crate) fn session_stopped(&self) -> bool {
        self.session.stopped()
    }
    #[cfg(test)]
    pub(crate) fn session_queued(&self) -> bool {
        self.session.queued()
    }
    #[cfg(test)]
    pub(crate) fn session_opening(&self) -> bool {
        self.session.opening()
    }
    pub(crate) fn session_sign_in_required(&self) -> bool {
        self.session.sign_in_required()
    }
    pub(crate) fn session_signing_in(&self) -> bool {
        self.session.signing_in()
    }
    #[cfg(test)]
    pub(crate) fn session_closing(&self) -> bool {
        self.session.closing()
    }
    pub(crate) fn session_ticket(&self) -> nocterm_ai::session::Ticket {
        self.session.ticket()
    }
    pub(crate) fn session_request_current(&self, ticket: nocterm_ai::session::Ticket) -> bool {
        self.session.session_current(ticket)
    }
    pub(crate) fn has_status_error(&self) -> bool {
        self.status_error
    }
    pub(crate) fn release_history_delivery(&mut self) {
        self.archive.release_delivery_permit();
    }
    pub(crate) fn load_archive(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<Self>) {
        let request = self.archive.request(intent, self.session.ticket());
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
                let current = this.session.session_current(ticket)
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
            !self.session.leased(),
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
        self.tools.clear_displays();
        true
    }
    /// The session ended with an error; the next message reconnects.
    pub(crate) fn ended(&self) -> bool {
        self.session.failed()
    }
    /// Not connected to an agent: restored from history, idle or failed.
    pub(crate) fn dormant(&self) -> bool {
        !self.session.leased()
    }
    /// An empty chat nobody has named: dropped when the user moves on.
    pub(crate) fn is_draft(&self) -> bool {
        !self.is_archived()
            && !self.state.pinned()
            && self.state.entries().is_empty()
            && self.composer.is_empty()
            && self.state.name().is_none()
            && !self.session.generating()
            && !self.tools.approval_pending()
    }
    pub(crate) fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.cancel_pending();
        if !self.dormant() {
            self.persist(cx);
        }
        self.session.fail();
        self.composer.pause();
        self.status = nocterm_ai::redact::redact(message);
        self.status_error = true;
        self.cancel_pending();
        // A grant belongs to this session; a restarted chat asks again.
        self.tools.approval_revoke_scope();
        self.detach_session(cx);
        cx.notify();
    }
    fn require_authentication(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.composer.pause();
        self.session.require_sign_in();
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
        if !cx.ai_enabled() || self.session_phase() != SessionPhase::SignInRequired {
            return;
        }
        let Some(commands) = self.commands().clone() else {
            return;
        };
        let Some(method) = self
            .session
            .info()
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
                let completion = opener(self.host.workspace().clone(), request, window, cx);
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
        self.session.begin_sign_in();
        self.status = "Signing in…".into();
        self.status_error = false;
        let authority = self.session.authority();
        let operation = session_driver::authentication_operation::AuthenticationOperation::new(
            authority,
            self.hold_operation(),
            auth,
        );
        let future = cx.background_executor().spawn(operation.run());
        cx.spawn(async move |this, cx| {
            let (authority, result) = future.await;
            let _ = this.update(cx, |this, cx| {
                this.on_authentication_complete(authority, commands, result, cx);
            });
        })
        .detach();
        cx.notify();
    }
    fn on_authentication_complete(
        &mut self,
        authority: session_driver::SessionAuthority,
        commands: Arc<dyn nocterm_ai::AgentCommands>,
        result: Result<(), nocterm_ai::AgentError>,
        cx: &mut Context<Self>,
    ) {
        if !self.session.accepts_authority(&authority) || !cx.ai_enabled() {
            return;
        }
        let has_session = self.session().is_some();
        self.session.finish_sign_in(result.is_ok(), has_session);
        match result {
            Ok(()) => {
                self.status_error = false;
                if has_session {
                    self.status = "Ready".into();
                    cx.notify();
                } else if let (Some(info), Some(workdir)) =
                    (self.session.info().cloned(), self.session_workdir().clone())
                {
                    self.create_session(commands, info, workdir, cx);
                }
            }
            Err(error) => {
                self.status = nocterm_ai::redact::redact(&error.to_string());
                self.status_error = true;
                cx.notify();
            }
        }
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
        self.state.finish_pending_tools(self.host.timestamp());
    }
    fn cancel_pending(&mut self) {
        // Dropped responders answer `Cancelled`.
        for call in self.tools.approval_cancel_pending() {
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
        if !self.session.begin_cancel() {
            cx.notify();
            return;
        }
        if let (Some(commands), Some(session)) = (&self.commands(), &self.session()) {
            commands.cancel(session.clone());
        }
        self.status = "Stopping…".into();
        let operation =
            session_driver::cancel_operation::CancelOperation::new(self.session.authority());
        cx.spawn(async move |this, cx| {
            let authority = operation.run(cx.background_executor().clone()).await;
            let _ = this.update(cx, |this, cx| this.on_cancel_deadline(authority, cx));
        })
        .detach();
        cx.notify();
    }
    fn on_cancel_deadline(
        &mut self,
        authority: session_driver::SessionAuthority,
        cx: &mut Context<Self>,
    ) {
        if self.session.accepts_authority(&authority)
            && self.session.generating()
            && self.session.turn_current(authority.ticket())
        {
            self.fail(
                "Cancellation timed out. The next message reconnects the agent.",
                cx,
            );
        }
    }
    pub(crate) fn attach(&mut self, attachment: Attachment, cx: &mut Context<Self>) {
        if self.composer.toggle_attachment(attachment.clone()) && !self.composer.editing() {
            self.tools.detach_attachment(&attachment);
            self.tools.approval_revoke_scope();
            self.cancel_pending();
            self.revoke_executions(cx);
            if !self.session.generating() {
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
