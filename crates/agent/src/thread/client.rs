//! A thread is a client of the runtime: it applies the runtime's events to itself.
use super::tools::unreachable;
use super::{AgentThread, approval_inbox::ApprovalToken, permissions::PermissionGate};
use super::{
    approval_inbox::{ToolDecision, ToolRejection},
    bound_call::BoundToolCall,
};
use crate::runtime::SessionEvent;
use gpui_kit::{App, Context};
use nocterm_ai::{
    AgentCommands, AgentInfo, BridgeCall, BridgeRejection, TerminalCall, ToolFailure,
    thread::{Entry, ThreadChange},
    tool_display::{self, ToolDisplay},
};
use nocterm_ai::{PermissionResponder, acp};

use nocterm_workspace::TerminalEntry;
use std::path::PathBuf;
use std::sync::Arc;

impl AgentThread {
    pub(super) fn on_tool_completion(
        &mut self,
        call: BridgeCall,
        result: Result<serde_json::Value, ToolFailure>,
        cx: &mut Context<Self>,
    ) {
        self.finish(call, result);
        cx.notify();
    }

    pub(crate) fn revoke_executions(&mut self, cx: &gpui_kit::App) {
        if self.tools.active_commands().next().is_none() && self.tools.live_empty() {
            return;
        }
        let resolved = self.resolved(cx);
        let attached = resolved
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect::<Vec<_>>();
        let current = resolved
            .into_iter()
            .map(|(id, entry, _)| {
                let executor = entry.capabilities.state().info(cx).and_then(|info| {
                    entry
                        .capabilities
                        .execution()?
                        .executor(info.expected, cx)
                        .ok()
                });
                (id, executor)
            })
            .collect::<Vec<_>>();
        self.tools.revoke_replaced_executors(&current, &attached);
    }

    pub(super) fn on_execution_watch(&mut self, cx: &gpui_kit::App) -> bool {
        if !self.host.enabled(cx) {
            self.cancel_execution();
        } else {
            self.revoke_executions(cx);
        }
        if self.tools.active_commands().next().is_none() {
            self.tools.finish_watch();
            false
        } else {
            true
        }
    }

    pub(super) fn cancel_execution(&mut self) {
        self.tools.cancel_execution();
    }

    pub(super) fn command_payload(
        &self,
        id: &str,
        state: super::tool_runtime::CommandSnapshot,
        cx: &gpui_kit::App,
    ) -> serde_json::Value {
        super::tool_runtime::presentation::command_payload(
            id,
            state,
            self.host.ai(cx).approval.redact_secrets,
        )
    }

    /// Every response while the chat exists passes through its recorded result.
    pub(super) fn finish(
        &mut self,
        call: BridgeCall,
        result: Result<serde_json::Value, ToolFailure>,
    ) {
        self.tools
            .finish_display(call.display_token, result.clone());
        self.normalize_tool_displays();
        let _ = call.respond.send(result);
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

    pub(crate) fn approve_tool(
        &mut self,
        token: ApprovalToken,
        allow: bool,
        grant: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(call) = self.tools.approval_tool(token).cloned() else {
            return;
        };
        let gate = self.tool_gate(&call, cx);
        let decision = self.tools.approval_choose_tool(token, allow, grant, &gate);
        self.apply_tool_decision(decision, cx);
    }

    pub(crate) fn handle_tool(&mut self, mut call: BridgeCall, cx: &mut Context<Self>) {
        if !self.host.enabled(cx)
            || !self.session.accepts_updates()
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
        let expected = gate.expected();
        let decision = self.tools.approval_receive_tool(
            BoundToolCall::new(call, expected),
            &gate,
            &self.host.ai(cx).approval,
        );
        self.apply_tool_decision(decision, cx);
    }

    fn tool_destination(
        &mut self,
        request: &TerminalCall,
        entry: Option<&TerminalEntry>,
        cx: &App,
    ) -> Option<String> {
        if let Some(entry) = entry {
            let info = entry.capabilities.state().info(cx)?;
            if info.local {
                return Some("This computer".into());
            }
            let target = info.target?;
            return Some(format!("{} · {target}", entry.title));
        }
        let server = request.server_id()?;
        self.offline_servers(cx)
            .into_iter()
            .find(|(descriptor, _)| descriptor.server_id == server)
            .map(|(descriptor, summary)| format!("{} · {}", descriptor.name, summary.target))
    }

    fn normalize_tool_displays(&mut self) {
        let patches = self.tools.normalize_displays(self.state.entries());
        self.state.patch_tool_rows(patches, self.host.timestamp());
    }

    pub(super) fn finalize_tool_displays(&mut self) {
        self.tools.finalize_displays(self.session.turn());
        self.normalize_tool_displays();
    }

    pub(crate) fn record_rejected_tool(
        &mut self,
        rejected: BridgeRejection,
        cx: &mut Context<Self>,
    ) {
        if !self.host.enabled(cx)
            || !self.session.accepts_updates()
            || self.session.stopped()
            || self.registration().as_ref().map(|value| value.id) != Some(rejected.registration_id)
        {
            return;
        }
        let server = tool_display::bridge_server_name(rejected.registration_id);
        let mut display = ToolDisplay::requested(rejected.tool.clone(), rejected.arguments.clone());
        display.finish(Err(rejected.error.into()));
        if self.host.ai(cx).approval.redact_secrets {
            display.redact();
        }
        self.tools
            .prepare_displays(self.session.turn(), self.state.entries().len());
        self.tools
            .record_display(server, (rejected.tool, rejected.arguments), display);
        self.normalize_tool_displays();
        cx.notify();
    }

    pub(crate) fn record_tool_display(
        &mut self,
        call: &mut BridgeCall,
        provided_entry: Option<&TerminalEntry>,
        cx: &mut Context<Self>,
    ) {
        let Some(registration) = &self.registration() else {
            return;
        };
        let server = tool_display::bridge_server_name(registration.id);
        let (tool, normalized) = tool_display::request(&call.call);
        let arguments = call.arguments.clone().unwrap_or(normalized);
        let entry = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.call.terminal_id())
            .map(|(_, entry, _)| entry);
        let destination = self.tool_destination(&call.call, provided_entry.or(entry.as_ref()), cx);
        let mut display = ToolDisplay::requested(tool.into(), arguments.clone());
        display.destination = destination.map(|text| nocterm_ai::redact::redact(&text));
        if self.host.ai(cx).approval.redact_secrets {
            display.redact();
        }
        self.tools
            .prepare_displays(self.session.turn(), self.state.entries().len());
        call.display_token = self
            .tools
            .record_display(server, (tool.into(), arguments), display)
            .map(|id| (self.session.turn(), id));
        self.normalize_tool_displays();
        cx.notify();
    }

    pub(crate) fn apply_presented_update(
        &mut self,
        mut update: acp::SessionUpdate,
    ) -> ThreadChange {
        self.tools
            .prepare_displays(self.session.turn(), self.state.entries().len());
        tool_display::strip_provider_meta(&mut update);
        let change = if self.config.report_control(&update) {
            ThreadChange::Controls
        } else {
            self.state.apply_transcript(update, self.host.timestamp())
        };
        let patches = self.tools.normalize_displays(self.state.entries());
        self.state.patch_tool_rows(patches, self.host.timestamp());
        change
    }

    pub(super) fn on_prompt_completion(
        &mut self,
        authority: super::session_driver::SessionAuthority,
        result: Result<acp::PromptResponse, nocterm_ai::AgentError>,
        is_command: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.host.enabled(cx) || !self.session.accepts_authority(&authority) {
            return;
        }
        let completed = matches!(
            &result,
            Ok(response) if response.stop_reason != acp::StopReason::Cancelled
        ) && !self.session.stopped();
        if !self.session.finish_turn(authority.ticket(), completed) {
            return;
        }
        self.tools.clear_scope();
        self.cancel_pending();
        self.finalize_tool_displays();
        self.persist(cx);
        match result {
            Ok(response) => {
                if response.usage.is_some() {
                    self.state.tokens(response.usage);
                }
                // Agents that keep limits in their own files have
                // just written this turn's.
                let agent = self.state.agent_id().to_owned();
                let runtime = self.host.runtime().clone();
                cx.defer(move |cx| runtime.refresh_limits(&agent, cx));
                self.notice.present(if self.session.stopped() {
                    "Stopped".into()
                } else {
                    format!("{:?}", response.stop_reason)
                });
                if !is_command && completed {
                    self.state.set_pending_history(false, self.host.timestamp());
                    self.persist(cx);
                }
                self.dispatch_next(cx);
                cx.notify();
            }
            Err(nocterm_ai::AgentError::AuthRequired(message)) => {
                self.require_authentication(&message, cx);
            }
            // Some agents answer a cancelled prompt with an error; the
            // session itself goes on.
            Err(error) if self.session.stopped() => {
                tracing::debug!(%error, "stopped prompt ended with an error");
                self.notice.present("Stopped".into());
                self.dispatch_next(cx);
                cx.notify();
            }
            Err(error) => self.fail(&error.to_string(), cx),
        }
    }

    pub(super) fn clear_settled_disabled_document(&mut self, cx: &gpui_kit::App) {
        if !self.host.enabled(cx) && self.state.release_disabled() {
            self.config.disabled();
            self.composer.reset_after_disable();
        }
    }

    /// The chat's connection stopped. With AI turned off, the chat also
    /// forgets its transcript and what was attached.
    pub(crate) fn connection_stopped(&mut self, message: &str, cx: &mut Context<Self>) {
        self.fail(message, cx);
        self.clear_settled_disabled_document(cx);
    }

    /// Applies one notification from the chat's connection. Notifications
    /// for other sessions sharing the connection are ignored.
    pub(crate) fn session_update(
        &mut self,
        notification: &acp::SessionNotification,
        cx: &mut Context<Self>,
    ) {
        if self.session.opening() && self.session().is_none() {
            self.config
                .keep_control(self.session.ticket(), notification);
        }
        if self.session().as_ref() != Some(&notification.session_id)
            || !self.session.accepts_updates()
            || matches!(notification.update, acp::SessionUpdate::UserMessageChunk(_))
        {
            return;
        }
        let index = match &notification.update {
            acp::SessionUpdate::ToolCallUpdate(update) => self.state.entries().iter().position(
                |entry| matches!(entry, Entry::Tool(call) if call.tool_call_id == update.tool_call_id),
            ),
            _ => self.state.entries().len().checked_sub(1),
        };
        let configuration = matches!(
            notification.update,
            acp::SessionUpdate::ConfigOptionUpdate(_) | acp::SessionUpdate::CurrentModeUpdate(_)
        );
        match self.apply_presented_update(notification.update.clone()) {
            ThreadChange::Transcript => {
                if let Some(index) = index {
                    self.mark_dirty(index);
                }
                if let Some(index) = self.state.entries().len().checked_sub(1) {
                    self.mark_dirty(index);
                }
            }
            ThreadChange::Metadata => self.persist(cx),
            // The agent changed its configuration itself, e.g. left plan mode.
            ThreadChange::Controls if configuration => self.sync_config(true, cx),
            _ => {}
        }
        cx.notify();
    }

    fn on_configuration_complete(
        &mut self,
        ticket: nocterm_ai::session::Ticket,
        reconciling: bool,
        response: super::config::operation::ConfigurationResult,
        cx: &mut Context<Self>,
    ) {
        use super::config::operation::ConfigurationResult;
        if !self.session.session_current(ticket)
            || !self.host.enabled(cx)
            || (reconciling && !self.session.opening())
        {
            return;
        }
        let option = matches!(&response, ConfigurationResult::Option(_));
        let result = match response {
            ConfigurationResult::Option(result) => self.config.accept_option_result(ticket, result),
            ConfigurationResult::Mode(id, result) => {
                self.config
                    .accept_mode_result(ticket, id, result, reconciling)
            }
        };
        let Some(result) = result else {
            return;
        };
        if reconciling {
            if let Err(error) = result {
                tracing::warn!(%error, "{}", super::config::operation::rejection_description(option));
                if option {
                    self.notice.present(format!(
                        "The agent kept its own setting: {}",
                        nocterm_ai::redact::redact(&error.to_string())
                    ));
                }
            }
            self.configure(ticket, cx);
        } else {
            match result {
                Ok(()) => {
                    self.notice.clear_error();
                    if option {
                        self.remember_options(cx);
                    }
                }
                Err(error) => {
                    self.notice
                        .present(nocterm_ai::redact::redact(&error.to_string()));
                    self.notice.mark_error();
                }
            }
            self.sync_config(true, cx);
            cx.notify();
        }
    }

    pub(super) fn start_configuration(
        &mut self,
        ticket: nocterm_ai::session::Ticket,
        reconciling: bool,
        commands: Arc<dyn AgentCommands>,
        session: acp::SessionId,
        request: super::config::operation::ConfigurationRequest,
        cx: &mut Context<Self>,
    ) {
        let operation = super::config::operation::ConfigurationOperation::new(
            ticket,
            reconciling,
            self.hold_operation(),
            commands,
            session,
            request,
        );
        let future = cx.background_executor().spawn(operation.run());
        cx.spawn(async move |this, cx| {
            let (ticket, reconciling, result) = future.await;
            let _ = this.update(cx, |this, cx| {
                this.on_configuration_complete(ticket, reconciling, result, cx)
            });
        })
        .detach();
    }

    pub(super) fn configure(
        &mut self,
        ticket: nocterm_ai::session::Ticket,
        cx: &mut Context<Self>,
    ) {
        use super::config::{ConfigStep, operation::ConfigurationRequest};
        let step = self.config.next_request(ticket);
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        let request = match step {
            ConfigStep::Stale => return,
            ConfigStep::Ready => return self.configured(cx),
            ConfigStep::SetOption(id, value) => ConfigurationRequest::Option(id, value.to_acp()),
            ConfigStep::SetMode(id) => ConfigurationRequest::Mode(id),
        };
        self.start_configuration(ticket, true, commands, session, request, cx);
    }

    fn on_open_session_completion(
        &mut self,
        completion: super::session_driver::open_operation::OpenSessionCompletion,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.host.enabled(cx) || !self.session.accepts_authority(completion.authority()) {
            return false;
        }
        let (authority, result, restored, workdir, pending_restore) = completion.into_parts();
        let ticket = authority.ticket();
        if restored.is_none() && result.is_err() {
            self.session.retain_failed_restore(ticket, pending_restore);
        }
        match result {
            Ok(response) => {
                let session = response.session_id;
                if !self.session.accept_open(
                    ticket,
                    session.clone(),
                    (restored == Some(false)).then_some(workdir),
                ) {
                    return false;
                }
                self.config.start_open(
                    ticket,
                    response.modes,
                    response.config_options.unwrap_or_default(),
                );
                self.config.begin_reconciliation(&session, ticket);
                self.state.set_pending_history(
                    self.state.pending_history()
                        || (restored != Some(true) && !self.state.entries().is_empty()),
                    self.host.timestamp(),
                );
                self.notice.present(if self.state.pending_history() {
                    "Reconnected in a new session. Saved conversation will accompany the next message.".into()
                } else {
                    "Ready".into()
                });
                self.accept_document_inputs(cx, false);
                self.save(cx);
                self.configure(ticket, cx);
            }
            Err(nocterm_ai::AgentError::AuthRequired(message)) => {
                self.require_authentication(&message, cx)
            }
            Err(error) => self.fail(&error.to_string(), cx),
        }
        true
    }

    /// Opens the chat's agent session on a fresh connection. A saved session
    /// is reopened, or copied for a fork; when the agent cannot do that, a
    /// new session starts and the saved conversation goes with the next
    /// message. Only a sign-in keeps the chat from going on.
    pub(crate) fn create_session(
        &mut self,
        commands: Arc<dyn AgentCommands>,
        info: AgentInfo,
        workdir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.host.enabled(cx) {
            return;
        }
        let Some(server) = self.registration().as_ref().map(|r| r.mcp_server()) else {
            self.fail("Terminal bridge is unavailable.", cx);
            return;
        };
        if !self
            .session
            .begin_open(commands.clone(), info, workdir.clone())
        {
            return;
        }
        self.notice.present("Starting chat…".into());
        self.notice.clear_error();
        self.session.discard_broken_restore();
        let restore = self
            .session
            .restore()
            .clone()
            .filter(|restore| !self.state.entries().is_empty() && restore.workdir.is_absolute())
            .map(|mut restore| {
                restore.workdir = workdir.clone();
                restore
            });
        let authority = self.session.authority();
        self.config.opening_started(authority.ticket());
        let operation = super::session_driver::open_operation::OpenSessionOperation::new(
            commands.clone(),
            authority,
            restore,
            workdir,
            vec![server],
            cx.background_executor().clone(),
        );
        let future = cx.background_executor().spawn(operation.run());
        cx.spawn(async move |this, cx| {
            let completion = future.await;
            let orphan = completion.orphan();
            let transferred = this
                .update(cx, |this, cx| {
                    this.on_open_session_completion(completion, cx)
                })
                .unwrap_or(false);
            if !transferred && let Some(session) = orphan {
                let cleanup = commands.close_session(session);
                cx.background_executor()
                    .spawn(async move {
                        let _ = cleanup.await;
                        let _ = commands.shutdown_gracefully().await;
                    })
                    .detach();
            }
        })
        .detach();
        cx.notify();
    }

    fn permission_gate(&self, cx: &App) -> PermissionGate {
        PermissionGate::new(
            self.host.enabled(cx),
            self.session.accepts_updates(),
            self.session.stopped(),
            self.session().clone(),
        )
    }
    pub(crate) fn permission(
        &mut self,
        request: acp::RequestPermissionRequest,
        respond: PermissionResponder,
        cx: &mut Context<Self>,
    ) {
        let gate = self.permission_gate(cx);
        let policy = self.host.ai(cx).approval.agent_permissions;
        if self
            .tools
            .approval_receive_provider(request, respond, &gate, policy)
        {
            cx.notify();
        }
    }
    pub(crate) fn apply_permission_policy(&mut self, cx: &mut Context<Self>) {
        let gate = self.permission_gate(cx);
        let policy = self.host.ai(cx).approval.agent_permissions;
        if self.tools.approval_apply_provider_policy(&gate, policy) {
            cx.notify();
        }
    }
    pub(crate) fn choose_permission_request(
        &mut self,
        token: ApprovalToken,
        choice: Option<acp::PermissionOptionId>,
        cx: &mut Context<Self>,
    ) {
        let gate = self.permission_gate(cx);
        if self.tools.approval_choose_provider(token, choice, &gate) {
            cx.notify();
        }
    }
    /// Whether the chat still holds connection `key`.
    fn holds(&self, key: u64) -> bool {
        self.connection_key() == Some(key)
    }
    pub(crate) fn document_admitted(&mut self, revision: u64, cx: &mut Context<Self>) {
        self.state.acknowledge_admission(revision);
        self.clear_settled_disabled_document(cx);
        cx.notify();
    }
    pub(super) fn document_save_failed(
        &mut self,
        revision: u64,
        error: crate::runtime::AgentPersistenceError,
        cx: &mut Context<Self>,
    ) {
        let message = format!(
            "Could not save chat: {error}. Queued messages are retained; check disk space before continuing."
        );
        if !self.state.failed(revision, error) {
            return;
        }
        self.notice.present(message);
        self.composer.pause();
        self.notice.mark_error();
        cx.notify();
    }
    pub(super) fn runtime_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Leased(lease) => self.begin_lease(lease),
            SessionEvent::Connected {
                connection,
                commands,
                info,
                workdir,
            } => {
                if self.holds(connection) && self.session.opening() {
                    self.create_session(commands, *info, workdir, cx);
                }
            }
            SessionEvent::Update(notification) => self.session_update(&notification, cx),
            SessionEvent::Permission { request, respond } => {
                self.permission(*request, respond, cx);
            }
            SessionEvent::Tool(call) => self.handle_tool(call, cx),
            SessionEvent::ToolRejected(rejected) => self.record_rejected_tool(rejected, cx),
            SessionEvent::Stopped {
                connection,
                message,
            } => {
                if self.holds(connection) {
                    self.connection_stopped(&message, cx);
                }
            }
            SessionEvent::Failed(message) => self.fail(&message, cx),
            SessionEvent::AgentRemoved => {
                self.fail("The agent is no longer configured.", cx);
            }
            SessionEvent::Idle { connection } => {
                if !self.holds(connection) {
                    return;
                }
                self.detach_session(cx);
                self.notice
                    .present("Agent idle; conversation retained.".into());
                cx.notify();
            }
            SessionEvent::CleanupUnconfirmed(error) => {
                self.notice.present(format!(
                    "Agent process cleanup could not be confirmed: {error}"
                ));
                self.notice.mark_error();
                self.composer.pause();
                cx.notify();
            }
            SessionEvent::SessionClosed => {
                self.session.connection_closed();
                cx.notify();
            }
            SessionEvent::SaveAdmitted { revision } => self.document_admitted(revision, cx),
            SessionEvent::Saved { revision } => {
                if self.state.published(revision) {
                    if self.host.enabled(cx) {
                        self.dispatch_next(cx);
                    } else {
                        self.clear_settled_disabled_document(cx);
                    }
                }
                cx.notify();
            }
            SessionEvent::SaveFailed { revision, error } => {
                self.document_save_failed(revision, error, cx)
            }
            SessionEvent::PolicyChanged => self.apply_permission_policy(cx),
            SessionEvent::Shutdown => self.release_resources(cx),
        }
    }
}
