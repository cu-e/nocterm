use crate::runtime::SessionLease;
use approval_inbox::ToolGate;
use futures::FutureExt as _;
use gpui::{AnyWindowHandle, App, AppContext as _, Context, EntityId, Window};
use nocterm_ai::context::{ServerDescriptor, TerminalDescriptor};
use nocterm_ai::model::auth::AuthFlow;
use nocterm_ai::model::controls::{ConfigChoices, ConfigKind, ConfigOption};
use nocterm_ai::model::ids::{AuthMethodId, SessionId};
use nocterm_ai::{
    AgentCommands, AgentInfo, BridgeRegistration, TerminalCall, session::SessionPhase,
    thread::ThreadState,
};

use nocterm_workspace_ports::{ConnectionSummary, ExpectedSession, TerminalEntry};
use session_driver::OperationGuard;
use std::{path::PathBuf, sync::Arc};

pub mod archive;
mod attachments;
mod bindings;
mod client;
mod client_port;
mod composer;
mod config;
mod notice;
pub use client_port::client;
pub use composer::Composer;
pub use config::ChatConfig;
mod approval_inbox;
mod background_open;
mod bound_call;
mod connection_wait;
mod execution;
mod permissions;
mod reconnects;
mod session_grants;
pub use approval_inbox::ToolApprovalCard;
pub use permissions::PermissionCard;
mod queue;
mod tool_display;
pub use queue::QueuedPrompt;
mod live;
mod restart;
mod session_driver;
mod tool_runtime;
pub use restart::RestartData;
mod document;
mod saved;
mod sign_in_wait;
pub use sign_in_wait::SignInWait;
mod session;
mod tools;
/// A chat's title from its name and transcript, as the history shows it.
#[cfg(any(test, feature = "test-support"))]
pub fn chat_title(name: Option<&str>, state: &ThreadState) -> String {
    saved::chat_title(name, state)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attachment {
    Terminal(EntityId),
    Connection(String),
    Group(String),
    UnavailableLocal(String),
}
/// An agent session from a saved chat, to reopen on the next connection.
#[derive(Clone, Debug)]
pub struct Restore {
    session: SessionId,
    workdir: PathBuf,
    /// Open a copy of the session rather than the session itself.
    fork: bool,
}
impl Restore {
    pub fn session(&self) -> &SessionId {
        &self.session
    }
    /// Whether a copy of the session opens rather than the session itself.
    pub fn fork(&self) -> bool {
        self.fork
    }
}
/// What a forked chat starts from.
pub struct Fork {
    chat: nocterm_ai::history::SavedChat,
    restore: Option<Restore>,
    attachments: Vec<Attachment>,
}

impl Fork {
    pub(crate) fn prepared(mut self) -> Result<Self, nocterm_ai::history::HistoryCodecError> {
        self.chat = nocterm_ai::history::prepare(self.chat)?;
        Ok(self)
    }
    pub(crate) fn resident_bytes(&self) -> usize {
        self.chat.resident_bytes()
    }
    pub(crate) fn key(&self) -> nocterm_ai::execution_origin::ChatKey {
        nocterm_ai::execution_origin::ChatKey::new(
            self.chat.header.scope,
            self.chat.header.id.clone(),
        )
    }
}

pub struct AgentThread {
    session: session_driver::SessionDriver,
    tools: tool_runtime::ToolRuntime,
    archive: archive::Archive,
    composer: Composer,
    config: config::ChatConfig,
    state: document::ChatDocument,
    notice: notice::ThreadNotice,
    deps: crate::chat_deps::ChatDeps,
    bindings: bindings::ThreadBindings,
}

impl AgentThread {
    pub fn on_reconnect_wait_finished(&mut self, receipt: ExpectedSession) {
        self.tools.finish_reconnect(receipt);
    }

    pub fn forget_background_open(&mut self, item: EntityId) {
        self.tools.forget_opened(item);
    }

    pub fn on_connection_wait_complete(
        &mut self,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) {
        self.tools.clear_wait(expected);
        cx.notify();
    }

    /// The original terminal collection captured when this document was opened.
    pub fn workspace_seat(&self) -> &crate::seat::Seat {
        self.deps.seat()
    }

    /// Ends background sessions this chat opened that it can no longer use.
    pub fn prune_background(&mut self, cx: &mut Context<Self>) {
        let resolved: Vec<EntityId> = self
            .resolved(cx)
            .into_iter()
            .map(|(_, entry, _)| entry.item)
            .collect();
        let close = self.tools.prune_background(&resolved);
        if let Some(window) = self.bindings.window() {
            let seat = self.deps.seat().clone();
            cx.defer(move |cx| {
                for item in close {
                    let _ = cx.update_window(window, |_, window, cx| {
                        seat.close_background(item, window, cx)
                    });
                }
            });
        }
    }

    fn tool_gate(&mut self, call: &TerminalCall, cx: &App) -> ToolGate {
        let resolved = self.resolved(cx);
        let offline = if call.terminal_id().is_none() && call.server_id().is_some() {
            self.offline_servers(cx)
        } else {
            Vec::new()
        };
        ToolGate::from_context(
            call,
            self.deps.enabled(cx) && self.session.accepts_updates(),
            self.registration().as_ref().map(|value| value.id),
            &resolved,
            &offline,
            cx,
        )
    }

    pub fn unsafe_live_input(&mut self, call: &TerminalCall, cx: &App) -> bool {
        if !matches!(call, TerminalCall::SendInput(_)) {
            return false;
        }
        self.resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .and_then(|(_, entry, _)| entry.capabilities.state().info(cx))
            .is_none_or(|info| info.at_prompt != Some(true) || info.dirty_input || info.alt_screen)
    }

    pub fn tool_cards(&self) -> Vec<ToolApprovalCard> {
        self.tools.approval_tool_cards()
    }

    pub fn permission_cards(&self) -> Vec<PermissionCard> {
        self.tools.approval_permission_cards()
    }

    pub fn approval_notice_generation(&self) -> u64 {
        self.tools.approval_generation()
    }

    pub fn has_pending_approvals(&self) -> bool {
        self.tools.approval_pending()
    }

    pub fn describe_target(&mut self, call: &TerminalCall, cx: &App) -> String {
        let input = self::tool_runtime::resolution::Resolution::observe(self.deps.seat(), cx);
        self.tools
            .describe_target(call, &input, self.composer.default_attachments(), cx)
    }

    pub fn context(&mut self, cx: &App) -> String {
        let input = self::tool_runtime::resolution::Resolution::observe(self.deps.seat(), cx);
        self.tools
            .context(&input, self.composer.default_attachments(), cx)
    }

    pub fn offline_servers(&mut self, cx: &App) -> Vec<(ServerDescriptor, ConnectionSummary)> {
        let input = self::tool_runtime::resolution::Resolution::observe(self.deps.seat(), cx);
        self.tools
            .offline_servers(&input, self.composer.default_attachments(), cx)
    }

    pub fn resolved(&mut self, cx: &App) -> Vec<(String, TerminalEntry, TerminalDescriptor)> {
        let input = self::tool_runtime::resolution::Resolution::observe(self.deps.seat(), cx);
        self.tools
            .resolved(&input, self.composer.default_attachments(), cx)
    }

    pub fn server_is_attached(&self, profile: &str, cx: &App) -> bool {
        let input = self::tool_runtime::resolution::Resolution::observe(self.deps.seat(), cx);
        input.summaries().iter().any(|summary| {
            summary.id.as_ref() == profile
                && self
                    .attachment_scope()
                    .iter()
                    .any(|attachment| attachment.covers_server(summary))
        })
    }

    pub fn begin_lease(&mut self, lease: SessionLease) {
        self.session.accept_lease(lease);
    }

    pub fn release_resources(&mut self, cx: &mut Context<Self>) {
        self.composer.pause();
        self.cancel_pending();
        self.finalize_tool_displays();
        self.detach_session(cx);
        self.session.release();
        self.notice
            .present("Agent resources released. Conversation retained.".into());
        cx.notify();
    }

    /// Detaches ACP without touching background terminals, execution jobs or shell leases.
    pub fn detach_session(&mut self, cx: &mut Context<Self>) {
        self.session.import_restore(self.restore_descriptor());
        self.finish_pending_tools();
        self.save(cx);
        // Dropping the lease releases the connection.
        if !self.session.release_lease() {
            return;
        }
        self.session.detach();
        self.config.detached();
        for call in self.tools.approval_end_session() {
            self.finish(call, Err("Request cancelled.".into()));
        }
    }

    /// With warm start on, a shown chat connects at once, so its model and
    /// options can be chosen before the first message. A chat opened from
    /// the history connects too once it is loaded; its restored queue stays
    /// paused until the user sends. A failed chat waits for the next
    /// message, so a broken agent is not restarted on every visit, and a
    /// chat that cannot save waits until it can.
    pub fn warm(&mut self, cx: &mut Context<Self>) {
        if self.session.shown()
            && !self.is_archived()
            && self.state.persistence_error().is_none()
            && self.deps.enabled(cx)
            && self.deps.ai(cx).sessions.warm_start
            && self.session.phase() == SessionPhase::Detached
        {
            self.request_activation(cx);
        }
    }

    /// A panel starts or stops showing the chat. A chat hidden before it
    /// was admitted gives up its place unless it has messages to send.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.session.set_shown(shown);
        if shown {
            self.warm(cx);
        } else if !self.composer.dispatchable() && self.session.withdraw_if_idle() {
            self.notice.clear_text();
            cx.notify();
        }
    }

    /// Asks the runtime for a connection. A failed chat recovers this way.
    pub fn request_activation(&mut self, cx: &mut Context<Self>) {
        if self.is_archived() || self.session.leased() || !self.session.queue_activation() {
            return;
        }
        self.notice.clear_error();
        self.notice.present("Waiting for an agent slot…".into());
        let owner = self::client(&cx.entity());
        let runtime = self.deps.runtime().clone();
        cx.defer(move |cx| {
            if let Err(error) = runtime.request_activation(owner.clone(), cx) {
                owner.emit(crate::runtime::SessionEvent::Failed(error.to_string()), cx);
            }
        });
        cx.notify();
    }

    pub fn session_busy(&self) -> bool {
        self.session.busy() || self.tools.busy() || self.composer.dispatchable()
    }

    pub fn hold_operation(&self) -> OperationGuard {
        self.session.hold_operation()
    }

    pub fn connection_key(&self) -> Option<u64> {
        self.session.connection_key()
    }

    pub fn session_workdir(&self) -> &Option<PathBuf> {
        self.session.workdir()
    }

    pub fn registration(&self) -> &Option<BridgeRegistration> {
        self.session.registration()
    }

    pub fn commands(&self) -> &Option<Arc<dyn AgentCommands>> {
        self.session.commands()
    }

    pub fn session(&self) -> &Option<SessionId> {
        self.session.session()
    }

    fn on_release(&mut self, id: EntityId, cx: &mut gpui::App) {
        self.cancel_pending();
        self.finalize_tool_displays();
        let snapshot = self
            .deps
            .enabled(cx)
            .then(|| self.capture_pending(cx))
            .flatten();
        self.cancel_execution();
        // The lease releases its connection when it is dropped.
        self.session.release_lease();
        let runtime = self.deps.runtime().clone();
        cx.defer(move |cx| {
            if let Some((chat, revision)) = snapshot
                && let Err(error) = runtime.save_chat(chat, id, revision, cx)
            {
                tracing::warn!(%error,"released chat admission failed");
            }
            if let Err(error) = runtime.unregister_document(id, cx) {
                tracing::warn!(%error, "could not retire released chat document");
            }
        });
        if let Some(window) = self.bindings.window() {
            let seat = self.deps.seat().clone();
            let background = self.tools.take_close_effects();
            cx.defer(move |cx| {
                for item in background {
                    let _ = cx.update_window(window, |_, window, cx| {
                        seat.close_background(item, window, cx)
                    });
                }
            });
        }
    }
    fn on_workspace_authority_changed(&mut self, cx: &gpui::App) {
        self.revoke_executions(cx);
    }

    fn from_document(
        state: document::ChatDocument,
        deps: crate::chat_deps::ChatDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let authority = deps.seat().observe(
            Box::new(move |cx| {
                let _ = this.update(cx, |this, cx| this.on_workspace_authority_changed(cx));
            }),
            cx,
        );
        let id = cx.entity_id();
        let release = cx.on_release(move |this, cx| this.on_release(id, cx));
        Self {
            session: Default::default(),
            tools: Default::default(),
            archive: Default::default(),
            composer: Default::default(),
            state,
            config: Default::default(),
            notice: Default::default(),
            deps,
            bindings: bindings::ThreadBindings::new(release, authority),
        }
    }
    pub(crate) fn new(
        chat: nocterm_ai::history::SavedChat,
        deps: crate::chat_deps::ChatDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::from_document(
            document::ChatDocument::new(
                chat.header.agent_id,
                nocterm_ai::execution_origin::ChatKey::new(chat.header.scope, chat.header.id),
                chat.header.updated,
            ),
            deps,
            cx,
        )
    }
    pub(crate) fn archived(
        row: nocterm_agent_runtime::CatalogRow,
        deps: crate::chat_deps::ChatDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        let summary = row.summary();
        let mut state = document::ChatDocument::new(
            summary.agent_id.clone(),
            nocterm_ai::execution_origin::ChatKey::new(summary.scope, summary.id.clone()),
            summary.updated,
        );
        state.archived_clean(summary);
        let mut thread = Self::from_document(state, deps, cx);
        thread.config.saved_model(summary.model.clone());
        thread.notice.present("Saved chat".into());
        thread.archive = archive::Archive::archived(row);
        thread
    }
    pub fn is_archived(&self) -> bool {
        self.archive.is_archived()
    }
    pub fn archive_summary(&self) -> Option<&nocterm_ai::history::ChatSummary> {
        self.archive.summary()
    }
    pub fn chat_id(&self) -> &str {
        self.state.chat_id()
    }
    pub fn history_entries(&self) -> &[nocterm_ai::thread::Entry] {
        self.state.entries()
    }
    pub fn take_changed_rows(&mut self) -> std::collections::HashSet<usize> {
        self.state.take_changed_rows()
    }
    pub fn document(&self) -> &document::ChatDocument {
        &self.state
    }
    pub fn transcript(&self) -> &ThreadState {
        self.state.transcript()
    }
    pub fn composer(&self) -> &Composer {
        &self.composer
    }
    pub fn configuration(&self) -> &config::ChatConfig {
        &self.config
    }
    pub fn config_options(&self) -> Vec<ConfigOption> {
        self.config.preview_options(self.session.opening())
    }
    pub fn draft_text(&self) -> Option<&str> {
        self.composer.draft().map(String::as_str)
    }
    pub fn queued_prompt_ids(&self) -> impl ExactSizeIterator<Item = u64> + '_ {
        self.composer.queue().iter().map(|prompt| prompt.saved().id)
    }
    pub fn queue_dispatchable(&self) -> bool {
        self.composer.dispatchable()
    }
    pub fn session_phase(&self) -> SessionPhase {
        self.session.phase()
    }
    pub fn updated_at(&self) -> u64 {
        self.state.updated()
    }
    pub fn status(&self) -> String {
        self.notice.text()
    }
    pub fn bind_window(&mut self, window: AnyWindowHandle) {
        self.bindings.bind_window(window);
    }
    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.bindings.window()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_shown(&self) -> bool {
        self.session.shown()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_leased(&self) -> bool {
        self.session.leased()
    }
    pub fn agent_info(&self) -> Option<&AgentInfo> {
        self.session.info()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_restore(&self) -> &Option<Restore> {
        self.session.restore()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn background_items(&self) -> &[EntityId] {
        self.tools.background()
    }
    pub fn sign_in_waits(&self) -> &[SignInWait] {
        self.tools.sign_ins()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn prompt_scope(&self) -> Option<&[Attachment]> {
        self.tools.prompt_scope()
    }
    pub fn context_bytes(&self) -> usize {
        self.tools.usage().0
    }
    pub fn tool_bytes(&self) -> usize {
        self.tools.usage().1
    }
    pub fn session_generating(&self) -> bool {
        self.session.generating()
    }
    pub fn session_accepts_updates(&self) -> bool {
        self.session.accepts_updates()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn session_stopped(&self) -> bool {
        self.session.stopped()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn session_queued(&self) -> bool {
        self.session.queued()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn session_opening(&self) -> bool {
        self.session.opening()
    }
    pub fn session_sign_in_required(&self) -> bool {
        self.session.sign_in_required()
    }
    pub fn session_signing_in(&self) -> bool {
        self.session.signing_in()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn session_closing(&self) -> bool {
        self.session.closing()
    }
    pub fn session_ticket(&self) -> nocterm_ai::session::Ticket {
        self.session.ticket()
    }
    pub fn session_request_current(&self, ticket: nocterm_ai::session::Ticket) -> bool {
        self.session.session_current(ticket)
    }
    pub fn has_status_error(&self) -> bool {
        self.notice.is_error()
    }

    /// The session ended with an error; the next message reconnects.
    pub fn ended(&self) -> bool {
        self.session.failed()
    }
    /// Not connected to an agent: restored from history, idle or failed.
    pub fn dormant(&self) -> bool {
        !self.session.leased()
    }
    /// An empty chat nobody has named: dropped when the user moves on.
    pub fn is_draft(&self) -> bool {
        !self.is_archived()
            && !self.state.pinned()
            && self.state.entries().is_empty()
            && self.composer.is_empty()
            && self.state.name().is_none()
            && !self.session.generating()
            && !self.tools.approval_pending()
    }
    pub fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.cancel_execution();
        self.finish_pending_tools();
        self.cancel_pending();
        if !self.dormant() {
            self.persist(cx);
        }
        self.session.fail();
        self.composer.pause();
        self.notice.present(nocterm_ai::redact::redact(message));
        self.notice.mark_error();
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
        self.notice.present(nocterm_ai::redact::redact(message));
        self.notice.mark_error();
        self.cancel_pending();
        cx.notify();
    }
    pub fn authenticate(
        &mut self,
        method: AuthMethodId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.deps.enabled(cx) || self.session_phase() != SessionPhase::SignInRequired {
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
                    .find(|available| available.id == method)
            })
            .cloned()
        else {
            return;
        };
        let auth = match &method.flow {
            AuthFlow::AgentManaged => commands.authenticate(method.id.clone()),
            AuthFlow::TerminalSetup(_) => {
                let runtime = self.deps.runtime();
                let Some(sign_in) = self.deps.sign_in().cloned() else {
                    return;
                };
                let Some(request) = self
                    .connection_key()
                    .and_then(|key| runtime.terminal_auth_request(key, &method, cx))
                else {
                    return;
                };
                let completion = sign_in.open(request, window, cx);
                async move {
                    completion
                        .await
                        .map_err(|_| nocterm_ai::AgentError::Io("Sign-in cancelled".into()))?
                        .map_err(|e| nocterm_ai::AgentError::Io(e.to_string()))
                }
                .boxed()
            }
        };
        self.session.begin_sign_in();
        self.notice.present("Signing in…".into());
        self.notice.clear_error();
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
        if !self.session.accepts_authority(&authority) || !self.deps.enabled(cx) {
            return;
        }
        let has_session = self.session().is_some();
        self.session.finish_sign_in(result.is_ok(), has_session);
        match result {
            Ok(()) => {
                self.notice.clear_error();
                if has_session {
                    self.notice.present("Ready".into());
                    cx.notify();
                } else if let (Some(info), Some(workdir)) =
                    (self.session.info().cloned(), self.session_workdir().clone())
                {
                    self.create_session(commands, info, workdir, cx);
                }
            }
            Err(error) => {
                self.notice
                    .present(nocterm_ai::redact::redact(&error.to_string()));
                self.notice.mark_error();
                cx.notify();
            }
        }
    }
    pub fn mark_dirty(&mut self, index: usize) {
        self.state.mark_row(index);
    }
    pub fn refresh_storage_budget(&mut self) -> Result<(), nocterm_ai::history::HistoryCodecError> {
        self.state.refresh_budget()
    }
    fn finish_pending_tools(&mut self) {
        self.finalize_tool_displays();
        self.state.finish_pending_tools(self.deps.timestamp());
    }
    fn cancel_pending(&mut self) {
        // Dropped responders answer `Cancelled`.
        for call in self.tools.approval_cancel_pending() {
            self.finish(call, Err("Request cancelled.".into()));
        }
    }
    /// Cancels the turn in progress. The session stays open for the next
    /// prompt.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
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
        self.notice.present("Stopping…".into());
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
}

pub fn config_label(option: &ConfigOption) -> String {
    match &option.kind {
        ConfigKind::Select(select) => {
            let options = match &select.choices {
                ConfigChoices::Flat(values) => values.iter().collect::<Vec<_>>(),
                ConfigChoices::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| group.choices.iter())
                    .collect(),
            };
            options
                .iter()
                .find(|value| value.id == select.current)
                .map(|value| value.name.clone())
                .unwrap_or_else(|| select.current.to_string())
        }
        ConfigKind::Boolean(value) => {
            if *value {
                "On".into()
            } else {
                "Off".into()
            }
        }
    }
}
