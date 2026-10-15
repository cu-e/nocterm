//! Prompt snapshots and FIFO dispatch. Cancellation completes before dispatch resumes.
use super::tools::unreachable;
use super::{
    background_open::BackgroundOpen,
    bound_call::BoundToolCall,
    connection_wait::{ConnectionWaitError, WaitTarget},
    execution::jobs,
};
use gpui_kit::{App, Context, EntityId, Task, WeakEntity};
use nocterm_ai::history::{SavedAttachment, SavedPrompt};
use nocterm_ai::{BridgeCall, TerminalCall, ToolFailure, acp};
use nocterm_session::ExecRequest;
use nocterm_ui::ActiveAi as _;
use nocterm_ui::SettingsExt as _;
use nocterm_workspace::{ExpectedSession, TerminalEntry, TerminalStatus};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{AgentThread, Attachment};

mod submit_error;
pub(crate) use submit_error::SubmitError;

#[derive(Clone)]
pub(crate) struct QueuedPrompt {
    pub saved: Arc<SavedPrompt>,
    pub image_keys: Arc<Vec<Option<(usize, usize, bool)>>>,
    pub attachments: Vec<Attachment>,
    pub encoded_len: usize,
    pub memory_len: usize,
}

impl QueuedPrompt {
    #[cfg(test)]
    pub(crate) fn new(saved: SavedPrompt, attachments: Vec<Attachment>) -> Self {
        Self::prepared(saved, attachments, None)
    }
    pub(crate) fn prepared(
        saved: SavedPrompt,
        attachments: Vec<Attachment>,
        encoded_len: Option<usize>,
    ) -> Self {
        static NEXT_IMAGE: AtomicUsize = AtomicUsize::new(1);
        let encoded_len = encoded_len
            .unwrap_or_else(|| nocterm_ai::history::prompt_size(&saved).unwrap_or(usize::MAX));
        let image_keys = saved
            .images
            .iter()
            .map(|block| match block {
                nocterm_ai::acp::ContentBlock::Image(image) => Some((
                    NEXT_IMAGE.fetch_add(1, Ordering::Relaxed),
                    image.data.len(),
                    true,
                )),
                _ => None,
            })
            .collect();
        let memory_len = nocterm_ai::history::prompt_memory(&saved).unwrap_or(usize::MAX);
        Self {
            memory_len,
            saved: Arc::new(saved),
            attachments,
            image_keys: Arc::new(image_keys),
            encoded_len,
        }
    }
}

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
            || !cx.ai_enabled()
        {
            return Err(ToolFailure::from("Chat is unavailable."));
        }
        self.revoke_executions(cx);
        let state = self
            .tools
            .command_snapshot(operation.id(), operation.terminal())?;
        operation.frame(
            state,
            cx.setting::<nocterm_ai::AiSettings>()
                .approval
                .redact_secrets,
        )
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
                .prepare_open(&server, &input, self.composer.default_attachments(), cx)
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
        let redact = cx
            .setting::<nocterm_ai::AiSettings>()
            .approval
            .redact_secrets;
        let action = self.tools.dispatch(call, entry, expected, redact, cx);
        self.apply_dispatch_action(action, cx);
    }

    fn execute_tool_with(&mut self, bound: BoundToolCall, reconnect: bool, cx: &mut Context<Self>) {
        use super::tool_runtime::dispatch::{AdmissionRoute, ExecutionAdmission};
        let admission = ExecutionAdmission::new(
            cx.ai_enabled() && self.session.accepts_updates(),
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
            || !cx.ai_enabled()
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
            || !cx.ai_enabled()
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
                && cx.ai_enabled()
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
        operation.check_active(self.session.accepts_updates() && cx.ai_enabled())?;
        let entry = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| id == operation.id())
            .map(|(_, entry, _)| entry);
        let result = operation.poll(
            entry,
            cx.setting::<nocterm_ai::AiSettings>()
                .approval
                .redact_secrets,
            cx,
        )?;
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

    pub(super) fn start_prompt(
        &mut self,
        text: String,
        images: Vec<acp::ContentBlock>,
        cx: &mut Context<Self>,
    ) {
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        // A new turn: updates count again.
        let Some(ticket) = self.session.begin_turn() else {
            return;
        };
        // ACP adapters inspect the first block for slash commands; local
        // administrative commands may require a single text block.
        let is_command = nocterm_ai::commands::invocation(&text, self.config.commands()).is_some();
        let context = (!is_command).then(|| self.context(cx));
        let history = (self.state.pending_history() && !is_command)
            .then(|| super::session::history_context(self.state.entries()));
        let prepared = super::composer::prompt_preparation::PromptPreparation::new(
            text, images, is_command, context, history,
        );
        if let Some(bytes) = prepared.context_bytes() {
            self.tools.account_context(bytes);
        }
        self.config.bind_request(ticket);
        self.config.prompt_started();
        let attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        let resume =
            super::document::Resume::new(Some(session.to_string()), self.session_workdir().clone());
        let input = super::document::Inputs::new(&self.composer, &self.config, attachments, resume);
        self.state
            .append_user(prepared.visible().to_vec(), self.host.timestamp(), &input);
        self.mark_dirty(self.state.entries().len() - 1);
        self.persist(cx);
        self.notice.present("Working…".into());
        self.notice.clear_error();
        let is_command = prepared.is_command();
        let operation = super::session_driver::prompt_operation::PromptOperation::new(
            self.session.authority(),
            commands,
            prepared.request(session),
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

    #[cfg(test)]
    pub(crate) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if let Err(error) = self.submit(text, None, cx) {
            self.notice.present(error.to_string());
            self.notice.mark_error();
            cx.notify();
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

    pub(crate) fn stable_attachments(
        &self,
        values: &[Attachment],
        cx: &App,
    ) -> Vec<SavedAttachment> {
        let terminals = self
            .host
            .workspace()
            .upgrade()
            .map(|workspace| workspace.read(cx).terminals(cx))
            .unwrap_or_default();
        let mut saved = Vec::new();
        for attachment in values {
            let value = match attachment {
                Attachment::Connection(id) => Some(SavedAttachment::Connection(id.clone())),
                Attachment::Group(id) => Some(SavedAttachment::Group(id.clone())),
                Attachment::UnavailableLocal(title) => {
                    Some(SavedAttachment::LocalTerminal(title.clone()))
                }
                Attachment::Terminal(id) => terminals
                    .iter()
                    .find(|entry| entry.item == *id)
                    .and_then(|entry| entry.capabilities.state().info(cx))
                    .map(|info| match info.profile {
                        Some(id) => SavedAttachment::Connection(id.to_string()),
                        None => SavedAttachment::LocalTerminal(info.title.to_string()),
                    }),
            };
            if let Some(value) = value
                && !saved.contains(&value)
            {
                saved.push(value);
            }
        }
        saved
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
        if !cx.ai_enabled() || self.session.sign_in_required() {
            return Ok(false);
        }
        if text.trim().is_empty() && self.composer.images().is_empty() {
            return Ok(false);
        }
        let candidate = self.composer.prepare_submission(
            text,
            replace,
            self.stable_attachments(self.composer.attachments(), cx),
        )?;
        let queue = candidate.queue();
        self.refresh_storage_budget()?;
        let mut metadata = self.history_metadata(cx);
        // Sending moves ordinary text into the queue: budget it once.
        if replace.is_none() {
            metadata.draft = None;
        }
        self.state.validate_candidate(
            &metadata,
            queue.iter().map(|prompt| prompt.encoded_len),
            queue.iter().map(|prompt| prompt.memory_len),
        )?;
        self.composer.commit_prompt_change(candidate);
        self.accept_document_inputs(cx, false);
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
        let Some(prompt) = self.composer.take_next() else {
            return;
        };
        // Context is fixed for this turn, including terminal tool access.
        self.tools.capture_scope(prompt.attachments);
        let saved = Arc::unwrap_or_clone(prompt.saved);
        self.start_prompt(saved.text, saved.images, cx);
    }
}

pub(crate) fn restore_attachments(values: &[SavedAttachment]) -> Vec<Attachment> {
    values
        .iter()
        .map(|value| match value {
            SavedAttachment::Connection(id) => Attachment::Connection(id.clone()),
            SavedAttachment::Group(id) => Attachment::Group(id.clone()),
            SavedAttachment::LocalTerminal(title) => Attachment::UnavailableLocal(title.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_snapshot_shares_image_payload_and_precomputed_keys() {
        let prompt = QueuedPrompt::new(
            SavedPrompt {
                id: 1,
                text: "image".into(),
                images: vec![nocterm_ai::acp::ContentBlock::Image(
                    nocterm_ai::acp::ImageContent::new("a".repeat(2 * 1024 * 1024), "image/png"),
                )],
                attachments: Vec::new(),
            },
            Vec::new(),
        );
        let copy = prompt.clone();
        assert!(Arc::ptr_eq(&prompt.saved, &copy.saved));
        assert!(Arc::ptr_eq(&prompt.image_keys, &copy.image_keys));
        assert_eq!(copy.image_keys[0].unwrap().1, 2 * 1024 * 1024);
    }
}
