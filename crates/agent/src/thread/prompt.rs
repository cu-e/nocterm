use super::AgentThread;
use crate::runtime::Runtime;
use gpui_kit::Context;
use nocterm_ai::acp;
use nocterm_ui::ActiveAi as _;

impl AgentThread {
    #[cfg(test)]
    pub(crate) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if let Err(error) = self.submit(text, None, cx) {
            self.status = error.to_string();
            self.status_error = true;
            cx.notify();
        }
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
        let context = (!is_command).then(|| {
            format!(
                "{}\n{}",
                nocterm_ai::context::TERMINAL_RULES,
                self.context(cx)
            )
        });
        if let Some(context) = &context {
            self.tools.account_context(context.len());
        }
        let mut visible = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
        visible.extend(images);
        let history = (self.state.pending_history() && !is_command)
            .then(|| super::session::history_context(self.state.entries()));
        self.config.bind_request(ticket);
        self.config.prompt_started();
        let attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        let resume =
            super::document::Resume::new(Some(session.to_string()), self.session_workdir().clone());
        let input = super::document::Inputs::new(&self.composer, &self.config, attachments, resume);
        self.state
            .append_user(visible.clone(), self.host.timestamp(), &input);
        self.mark_dirty(self.state.entries().len() - 1);
        self.persist(cx);
        let mut content = Vec::new();
        if let Some(context) = context {
            content.push(acp::ContentBlock::Text(acp::TextContent::new(context)));
        }
        if let Some(history) = history {
            content.push(acp::ContentBlock::Text(acp::TextContent::new(history)));
        }
        content.extend(visible);
        self.status = "Working…".into();
        self.status_error = false;
        let operation = super::session_driver::prompt_operation::PromptOperation::new(
            self.session.authority(),
            commands,
            acp::PromptRequest::new(session, content),
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
    fn on_prompt_completion(
        &mut self,
        authority: super::session_driver::SessionAuthority,
        result: Result<acp::PromptResponse, nocterm_ai::AgentError>,
        is_command: bool,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() || !self.session.accepts_authority(&authority) {
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
                cx.defer(move |cx| {
                    Runtime::global(cx).update(cx, |runtime, cx| runtime.refresh_limits(&agent, cx))
                });
                self.status = if self.session.stopped() {
                    "Stopped".into()
                } else {
                    format!("{:?}", response.stop_reason)
                };
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
                self.status = "Stopped".into();
                self.dispatch_next(cx);
                cx.notify();
            }
            Err(error) => self.fail(&error.to_string(), cx),
        }
    }
}
