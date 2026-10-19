//! Prompt snapshots and FIFO dispatch. Cancellation completes before dispatch resumes.
use super::tools::unreachable;
use super::{
    background_open::BackgroundOpen,
    bound_call::BoundToolCall,
    connection_wait::{ConnectionWaitError, WaitTarget},
    execution::jobs,
};
use gpui_kit::{App, Context, EntityId, Task, WeakEntity};
use nocterm_ai::{
    BridgeCall, PermissionResponder, TerminalCall, ToolFailure,
    history::source::ManagedPermission,
    model::{auth::ProviderPermission, ids::PermissionOptionId},
};
use nocterm_session::ExecRequest;

use nocterm_workspace::{ExpectedSession, TerminalEntry, TerminalStatus};

use super::{
    AgentThread, Attachment,
    approval_inbox::ApprovalToken,
    attachments::{LocalFacts, StableAttachments},
    composer::AttachmentSelection,
    permissions::PermissionGate,
};

mod submit_error;
pub(crate) use submit_error::SubmitError;

impl AgentThread {
    fn start_execution_watch(&mut self, cx: &mut Context<Self>) {
        if self.tools.has_watch() {
            return;
        }
        self.tools.install_watch(cx.spawn(async move |this, cx| {
            super::execution::ExecutionWatchOperation
                .run(&this, cx)
                .await
        }));
    }

    pub(super) fn poll_command_wait(
        &mut self,
        operation: &super::execution::CommandWaitOperation,
        cx: &mut Context<Self>,
    ) -> Result<Option<serde_json::Value>, ToolFailure> {
        if !self.session.session_current(operation.ticket())
            || !self.session.accepts_updates()
            || !self.host.enabled(cx)
        {
            return Err(ToolFailure::from("Chat is unavailable."));
        }
        self.revoke_executions(cx);
        let state = self
            .tools
            .command_snapshot(operation.id(), operation.terminal())?;
        operation.frame(state, self.host.ai(cx).approval.redact_secrets)
    }

    fn wait_command(
        &mut self,
        terminal: String,
        id: String,
        yield_ms: u64,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let operation = super::execution::CommandWaitOperation::new(
            terminal,
            id,
            yield_ms,
            self.session.ticket(),
            self.hold_operation(),
        );
        cx.spawn(async move |this, cx| operation.run(call, &this, cx).await)
            .detach();
    }

    pub(super) fn cancel_command(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        let TerminalCall::CancelCommand(request) = &call.call else {
            return;
        };
        let result = self
            .tools
            .cancel_command(&request.command_id, &request.terminal_id)
            .map(|state| self.command_payload(&request.command_id, state, cx));
        self.finish(call, result);
    }

    pub(super) fn read_command(&mut self, call: BridgeCall, cx: &mut Context<Self>) {
        let TerminalCall::ReadCommand(request) = &call.call else {
            return;
        };
        let terminal = request.terminal_id.clone();
        let id = request.command_id.clone();
        self.wait_command(terminal, id, request.yield_ms.unwrap_or(0), call, cx);
    }

    pub(super) fn execute_command(
        &mut self,
        entry: TerminalEntry,
        expected: nocterm_workspace::ExpectedSession,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let TerminalCall::ExecCommand(request) = &call.call else {
            return;
        };
        let executor = match entry
            .capabilities
            .execution()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::StructuredExecution,
            ))
            .and_then(|port| port.executor(expected, cx))
        {
            Ok(executor) => executor,
            Err(error) => {
                self.finish(call, Err(error.to_string().into()));
                return;
            }
        };
        let program = ExecRequest {
            program: request.program.clone(),
            args: request.args.clone(),
            stdin: request
                .stdin
                .as_ref()
                .map(|stdin| stdin.as_bytes().to_vec()),
        };
        let timer = cx
            .background_executor()
            .timer(jobs::timeout(request.timeout_ms));
        let started =
            match self
                .tools
                .start_command(request.terminal_id.clone(), executor, program, timer)
            {
                Ok(started) => started,
                Err(error) => {
                    self.finish(call, Err(error));
                    return;
                }
            };
        let (id, run) = started.into_run();
        cx.background_executor().spawn(run).detach();
        self.start_execution_watch(cx);
        let terminal = request.terminal_id.clone();
        let yield_ms = request.yield_ms.unwrap_or(1000);
        self.wait_command(terminal, id, yield_ms, call, cx);
    }

    fn reconnect_target(
        &mut self,
        entry: &TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) -> Result<WaitTarget, ConnectionWaitError> {
        let window = self
            .bindings
            .window()
            .ok_or(ConnectionWaitError::WorkspaceClosed)?;
        let receipt = self.tools.prepare_reconnect(entry, expected, cx)?;
        Ok(WaitTarget::new(
            self.host.workspace().clone(),
            window,
            receipt,
            self.session.ticket(),
            self.registration().as_ref().map(|value| value.id),
        ))
    }
    /// Open-terminal waits keep their original guard through the complete wait.
    fn reconnect(
        &mut self,
        entry: &TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) -> Task<Result<(ExpectedSession, serde_json::Value), ConnectionWaitError>> {
        let target = match self.reconnect_target(entry, expected, cx) {
            Ok(target) => target,
            Err(error) => return Task::ready(Err(error)),
        };
        let guard = self.hold_operation();
        let operation = super::tool_runtime::reconnect_operation::ReconnectOperation::new(target);
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            operation.run(&this, cx).await
        })
    }
    /// Accepted calls wait independently but commit dispatch in receipt order.
    fn reconnect_then_execute(
        &mut self,
        call: BoundToolCall,
        entry: &TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) {
        let target = match self.reconnect_target(entry, expected, cx) {
            Ok(target) => target,
            Err(error) => return self.finish(call.into_call(), Err(error.into_failure())),
        };
        let scope = super::tool_runtime::reconnect_dispatch::DispatchScope::new(&target);
        let guard = self.hold_operation();
        let identity = self.tools.admit_reconnect(scope, call, guard);
        let operation = super::tool_runtime::reconnect_operation::ReconnectOperation::new(target);
        cx.spawn(async move |this, cx| {
            let result = operation.run(&this, cx).await.map(|(receipt, _)| receipt);
            let _ = this.update(cx, |this, cx| {
                this.on_reconnect_complete(identity, result, cx)
            });
        })
        .detach();
    }

    fn finish_unreachable(&mut self, call: BridgeCall, reason: &str) {
        self.finish(call, Err(unreachable(reason)));
    }

    fn open_terminal(&mut self, server: String, call: BridgeCall, cx: &mut Context<Self>) {
        use super::tool_runtime::resolution::{OpenSelection, Resolution};
        let input = Resolution::observe(self.host.workspace(), cx);
        let summary =
            match self
                .tools
                .prepare_open(&server, &input, self.state.default_attachments(), cx)
            {
                OpenSelection::Respond(payload) => return self.finish(call, Ok(payload)),
                OpenSelection::Unknown => {
                    return self.finish_unreachable(
                        call,
                        "Unknown server. Call list_terminals for the attached ones.",
                    );
                }
                OpenSelection::Closed => {
                    return self.finish_unreachable(call, "The terminal was closed.");
                }
                OpenSelection::Reconnect(entry, expected) => {
                    let connected = self.reconnect(&entry, expected, cx);
                    cx.spawn(async move |this, cx| {
                        let result = connected
                            .await
                            .map(|(_, payload)| payload)
                            .map_err(ConnectionWaitError::into_failure);
                        let _ =
                            this.update(cx, |this, cx| this.on_tool_completion(call, result, cx));
                    })
                    .detach();
                    return;
                }
                OpenSelection::Create(summary) => summary,
            };
        let (Some(window), Some(directory)) = (
            self.bindings.window(),
            self.host
                .workspace()
                .upgrade()
                .and_then(|workspace| workspace.read(cx).connection_directory()),
        ) else {
            self.finish_unreachable(call, "The workspace is unavailable.");
            return;
        };
        let opening = BackgroundOpen::new(
            self.host.workspace().clone(),
            window,
            directory,
            summary.id.to_string(),
            self.session.ticket(),
            self.registration().as_ref().map(|value| value.id),
        );
        let guard = self.hold_operation();
        cx.spawn(async move |this, cx| {
            let _guard = guard;
            let result = opening.open(&this, cx).await;
            let _ = this.update(cx, |this, cx| {
                this.on_tool_completion(
                    call,
                    result.map_err(ConnectionWaitError::into_failure),
                    cx,
                );
            });
        })
        .detach();
    }

    fn apply_dispatch_action(
        &mut self,
        action: super::tool_runtime::dispatch::DispatchAction,
        cx: &mut Context<Self>,
    ) {
        use super::tool_runtime::dispatch::DispatchAction;
        match action {
            DispatchAction::Respond(call, result, _) => self.finish(call, result),
            DispatchAction::List(call) => {
                let payload = self.context(cx);
                self.finish(call, Ok(serde_json::json!({"context": payload})));
            }
            DispatchAction::Open(server, call) => self.open_terminal(server, call, cx),
            DispatchAction::Reconnect(call, entry, expected) => {
                self.reconnect_then_execute(call, &entry, expected, cx)
            }
            DispatchAction::ExecuteAdmitted(call, entry, expected) => {
                self.dispatch_tool(call, entry, expected, cx)
            }
            DispatchAction::Live(entry, expected, request, call) => {
                self.run_command(entry, expected, request, call, cx)
            }
            DispatchAction::Execute(entry, expected, call) => {
                self.execute_command(entry, expected, call, cx)
            }
            DispatchAction::Read(call) => self.read_command(call, cx),
            DispatchAction::Cancel(call) => self.cancel_command(call, cx),
            DispatchAction::Ignore => {}
        }
    }

    fn dispatch_tool(
        &mut self,
        call: BridgeCall,
        entry: TerminalEntry,
        expected: ExpectedSession,
        cx: &mut Context<Self>,
    ) {
        let redact = self.host.ai(cx).approval.redact_secrets;
        let action = self.tools.dispatch(call, entry, expected, redact, cx);
        self.apply_dispatch_action(action, cx);
    }

    fn execute_tool_with(&mut self, bound: BoundToolCall, reconnect: bool, cx: &mut Context<Self>) {
        use super::tool_runtime::dispatch::{AdmissionRoute, ExecutionAdmission};
        let admission = ExecutionAdmission::new(
            self.host.enabled(cx) && self.session.accepts_updates(),
            self.registration().as_ref().map(|value| value.id),
        );
        let bound = match admission.route(bound) {
            AdmissionRoute::Action(action) => return self.apply_dispatch_action(*action, cx),
            AdmissionRoute::Resolve(bound) => *bound,
        };
        let entry = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == bound.call().call.terminal_id())
            .map(|(_, entry, _)| entry);
        let reconnecting = entry
            .as_ref()
            .is_some_and(|entry| self.tools.reconnecting(&entry.item));
        let action = admission.resolve(bound, entry, reconnect, reconnecting, cx);
        self.apply_dispatch_action(action, cx);
    }

    pub(super) fn execute_tool(&mut self, call: BoundToolCall, cx: &mut Context<Self>) {
        self.execute_tool_with(call, true, cx);
    }

    pub(super) fn poll_connection_wait(
        &mut self,
        target: &super::connection_wait::WaitTarget,
        cx: &mut Context<Self>,
    ) -> Result<
        (
            Option<TerminalStatus>,
            bool,
            nocterm_ai::context::TerminalDescriptor,
        ),
        ConnectionWaitError,
    > {
        if self.host.workspace().entity_id() != target.workspace().entity_id()
            || self.registration().as_ref().map(|value| value.id) != target.registration()
            || !self.session.session_current(target.ticket())
            || !self.session.accepts_updates()
            || !self.host.enabled(cx)
        {
            return Err(ConnectionWaitError::Stopped);
        }
        let item = target.expected().item();
        let entry = self
            .resolved(cx)
            .into_iter()
            .find(|(_, entry, _)| entry.item == item);
        let (status, wait, descriptor) = target.probe(entry, cx)?;
        let asking = wait.is_some();
        if self.tools.observe_prompt(item, wait) {
            cx.notify();
        }
        Ok((status, asking, descriptor))
    }

    pub(super) fn accept_background_open(
        &mut self,
        opening: &BackgroundOpen,
        item: EntityId,
        cx: &App,
    ) -> Result<(), ConnectionWaitError> {
        self.authorize_background_open(
            opening.workspace(),
            opening.ticket(),
            opening.registration(),
            opening.profile(),
            cx,
        )?;
        self.tools.remember_opened(item);
        Ok(())
    }

    pub(super) fn authorize_background_open(
        &self,
        workspace: &WeakEntity<nocterm_workspace::Workspace>,
        ticket: nocterm_ai::session::Ticket,
        registration: Option<u64>,
        profile: &str,
        cx: &App,
    ) -> Result<(), ConnectionWaitError> {
        if self.host.workspace().entity_id() != workspace.entity_id()
            || self.registration().as_ref().map(|value| value.id) != registration
            || !self.session.session_current(ticket)
            || !self.session.accepts_updates()
            || !self.host.enabled(cx)
        {
            return Err(ConnectionWaitError::Stopped);
        }
        if !self.server_is_attached(profile, cx) {
            return Err(ConnectionWaitError::Detached);
        }
        Ok(())
    }

    fn on_reconnect_complete(
        &mut self,
        identity: super::tool_runtime::reconnect_dispatch::DispatchTicket,
        result: Result<ExpectedSession, ConnectionWaitError>,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.tools.ready_reconnect(&identity, result) else {
            return;
        };
        while let Some(effect) = self.tools.take_ready_reconnect(scope) {
            let (accepted, call, result, guard) = effect.into_parts();
            let current = self.session.accepts_updates()
                && self.host.enabled(cx)
                && accepted.is_current(
                    self.host.workspace(),
                    self.bindings.window(),
                    self.session.session_current(accepted.ticket()),
                    self.registration().as_ref().map(|value| value.id),
                    cx,
                );
            let result = result.and_then(|receipt| {
                if current && receipt == accepted.receipt() {
                    Ok(receipt)
                } else {
                    Err(ConnectionWaitError::Stopped)
                }
            });
            match result {
                Ok(receipt) => self.execute_tool_with(call.reconnected(receipt), false, cx),
                Err(error) => self.finish(call.into_call(), Err(error.into_failure())),
            }
            drop(guard);
        }
        cx.notify();
    }

    pub(super) fn on_live_command_complete(
        &mut self,
        id: String,
        lease: std::sync::Arc<nocterm_workspace::LiveCommandLease>,
        call: BridgeCall,
        result: Result<serde_json::Value, super::live::TerminalOperationError>,
        cx: &mut Context<Self>,
    ) {
        self.tools.finish_live_if_same(&id, &lease);
        drop(lease);
        self.on_tool_completion(
            call,
            result.map_err(super::live::TerminalOperationError::into_failure),
            cx,
        );
    }

    pub(super) fn poll_live_command(
        &mut self,
        operation: &mut super::live::LiveCommandOperation,
        cx: &mut Context<Self>,
    ) -> Result<Option<serde_json::Value>, super::live::TerminalOperationError> {
        operation.check_active(self.session.accepts_updates() && self.host.enabled(cx))?;
        let entry = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| id == operation.id())
            .map(|(_, entry, _)| entry);
        let result = operation.poll(entry, self.host.ai(cx).approval.redact_secrets, cx)?;
        Ok(result.map(|(payload, bytes)| {
            self.tools.account_outcome(bytes);
            payload
        }))
    }

    pub(super) fn run_command(
        &mut self,
        entry: TerminalEntry,
        expected: ExpectedSession,
        request: nocterm_ai::RunCommand,
        call: BridgeCall,
        cx: &mut Context<Self>,
    ) {
        let mut operation =
            match super::live::LiveCommandOperation::acquire(entry, expected, request, cx) {
                Ok(operation) => operation,
                Err(error) => {
                    self.finish(call, Err(error.into_failure()));
                    return;
                }
            };
        self.tools
            .install_live(operation.id().to_owned(), operation.lease());
        operation.hold(self.hold_operation());
        cx.spawn(async move |this, cx| operation.run(call, &this, cx).await)
            .detach();
    }

    pub(super) fn dispatch_failed(
        &mut self,
        ticket: nocterm_ai::session::Ticket,
        error: &impl std::fmt::Display,
        cx: &mut Context<Self>,
    ) {
        self.session.finish_turn(ticket, false);
        self.composer.pause();
        self.notice
            .present(nocterm_ai::redact::redact(&error.to_string()));
        self.notice.mark_error();
        cx.notify();
    }

    /// Stages the first queued prompt, serializes it natively and only then commits the turn.
    pub(super) fn start_prompt(&mut self, cx: &mut Context<Self>) {
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        // A new turn: updates count again.
        let Some(ticket) = self.session.begin_turn() else {
            return;
        };
        let is_command = self.queued_command();
        // ACP adapters inspect the first block for slash commands; local
        // administrative commands may require a single text block.
        let context = (!is_command).then(|| {
            format!(
                "{}\n{}",
                nocterm_ai::context::TERMINAL_RULES,
                self.context(cx)
            )
        });
        let history = (self.state.pending_history() && !is_command)
            .then(|| nocterm_ai::session::history_context(self.state.rows()));
        if let Some(context) = &context {
            self.tools.account_context(context.len());
        }
        self.config.bind_request(ticket);
        let controls = super::controls::Controls::read(self.live_controls());
        self.config.prompt_started(&controls);
        let framing = super::document::DispatchRequest::new(
            &session,
            context.as_deref(),
            history.as_deref(),
            is_command,
        );
        let (native, active) = match self
            .state
            .dispatch(framing, self.host.timestamp(), &commands)
        {
            Ok(staged) => staged,
            Err(error) => return self.dispatch_failed(ticket, &error, cx),
        };
        self.tools.capture_scope(active.scope());
        // The chat records the model of its latest accepted prompt.
        self.record_resume();
        if let Some(row) = self.state.rows().len().checked_sub(1) {
            self.mark_dirty(row);
        }
        self.persist(cx);
        self.notice.present("Working…".into());
        self.notice.clear_error();
        let operation = super::session_driver::prompt_operation::PromptOperation::new(
            self.session.authority(),
            native.bind(active),
        );
        let future = cx.background_executor().spawn(operation.run());
        cx.spawn(async move |this, cx| {
            let (authority, result) = future.await;
            let _ = this.update(cx, |this, cx| {
                this.on_prompt_completion(authority, result, is_command, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Whether the first queued prompt invokes a command the agent advertised.
    fn queued_command(&self) -> bool {
        let Some(prompt) = self
            .state
            .composer()
            .and_then(|composer| composer.queue().first())
        else {
            return false;
        };
        let text = prompt.text();
        self.live_controls().is_some_and(|controls| {
            nocterm_ai::commands::invocation_managed(&text, controls.commands()).is_some()
        })
    }

    #[cfg(test)]
    pub(crate) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if let Err(error) = self.submit(text, None, cx) {
            self.notice.present(error.to_string());
            self.notice.mark_error();
            cx.notify();
        }
    }

    pub(crate) fn attach(&mut self, attachment: Attachment, cx: &mut Context<Self>) {
        match self.edit_attachment(
            nocterm_ai::history::live::AttachmentEdit::Toggle(attachment.live_ref()),
            cx,
        ) {
            Ok(true) if !self.composer().editing() => {
                self.tools.detach_attachment(&attachment);
                self.tools.approval_revoke_scope();
                self.cancel_pending();
                self.revoke_executions(cx);
                if !self.session.generating() {
                    self.prune_background(cx);
                }
            }
            Ok(_) => {}
            Err(error) => return self.composer_refused(error, cx),
        }
        self.save(cx);
        cx.notify();
    }

    pub(crate) fn submit(
        &mut self,
        text: String,
        replace: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<bool, SubmitError> {
        if self.is_archived() {
            return Err(SubmitError::Loading);
        }
        if !self.host.enabled(cx) || self.session.sign_in_required() {
            return Ok(false);
        }
        if text.trim().is_empty() && self.composer().images().is_empty() {
            return Ok(false);
        }
        let stable = self.stable_attachments(self.composer().attachments(), cx);
        let stable = stable.refs();
        let replaced =
            self.state
                .queue_text(self.composer.next_queue_id(), &text, &stable, replace)?;
        self.composer.queued(replaced);
        if replace.is_none() {
            self.state.cancel_debounce();
        }
        self.persist(cx);
        if !self.session.generating() && replace.is_none() {
            self.composer.resume();
            self.dispatch_next(cx);
        }
        cx.notify();
        Ok(true)
    }

    pub(crate) fn dispatch_next(&mut self, cx: &mut Context<Self>) {
        if !self.queue_dispatchable()
            || self.session.generating()
            || self.session.sign_in_required()
        {
            return;
        }
        if self.state.persistence_error().is_some()
            || self.state.published_revision() < self.state.desired_revision()
        {
            return;
        }
        if self.session().is_none() {
            self.request_activation(cx);
            return;
        }
        if self.session.phase() != nocterm_ai::session::SessionPhase::Ready {
            return;
        }
        self.start_prompt(cx);
    }
}

impl AgentThread {
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
        request: ManagedPermission,
        respond: PermissionResponder,
        cx: &mut Context<Self>,
    ) {
        let request: ProviderPermission = request.permission().clone();
        self.provider_permission(request, respond, cx);
    }
    pub(crate) fn provider_permission(
        &mut self,
        request: ProviderPermission,
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
        choice: Option<PermissionOptionId>,
        cx: &mut Context<Self>,
    ) {
        let gate = self.permission_gate(cx);
        if self.tools.approval_choose_provider(token, choice, &gate) {
            cx.notify();
        }
    }

    pub(super) fn local_facts(&self, cx: &App) -> LocalFacts {
        let terminals = self
            .host
            .workspace()
            .upgrade()
            .map(|workspace| workspace.read(cx).terminals(cx))
            .unwrap_or_default();
        LocalFacts::read(&terminals, cx)
    }

    pub(super) fn stable_attachments(
        &self,
        selection: AttachmentSelection<'_>,
        cx: &App,
    ) -> StableAttachments {
        StableAttachments::select(selection, &self.local_facts(cx))
    }

    /// Edits the composer's future context; true when a value was removed.
    pub(super) fn edit_attachment(
        &mut self,
        edit: nocterm_ai::history::live::AttachmentEdit<'_>,
        cx: &App,
    ) -> Result<bool, super::document::DocumentError> {
        let facts = self.local_facts(cx);
        self.state
            .edit_attachments(edit, &facts.as_facts(), self.host.timestamp())
    }
}
