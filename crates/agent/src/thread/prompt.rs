use super::{AgentThread, PromptMetadata, config_label};
use crate::runtime::Runtime;
use gpui_kit::Context;
use nocterm_ai::acp;
use nocterm_ui::ActiveAi as _;

impl AgentThread {
    #[cfg(test)]
    pub(crate) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if let Err(error) = self.submit(text, None, cx) {
            self.status = error;
            self.status_error = true;
            cx.notify();
        }
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
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
        self.stopped = false;
        self.accept_updates = true;
        // ACP adapters inspect the first block for slash commands; local
        // administrative commands may require a single text block.
        let is_command = nocterm_ai::commands::invocation(&text, &self.state.commands).is_some();
        let context = (!is_command).then(|| {
            format!(
                "{}\n{}",
                nocterm_ai::context::TERMINAL_RULES,
                self.context(cx)
            )
        });
        if let Some(context) = &context {
            self.context_bytes += context.len();
        }
        let mut visible = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
        visible.extend(images);
        let history = (self.fallback_history && !is_command)
            .then(|| super::session::history_context(&self.state.entries));
        self.state.push_user(visible.clone());
        self.mark_dirty(self.state.entries.len() - 1);
        self.persist(cx);
        let mut content = Vec::new();
        if let Some(context) = context {
            content.push(acp::ContentBlock::Text(acp::TextContent::new(context)));
        }
        if let Some(history) = history {
            content.push(acp::ContentBlock::Text(acp::TextContent::new(history)));
        }
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
        self.turn += 1;
        let turn = self.turn;
        let epoch = self.epoch;
        let future = cx
            .background_executor()
            .spawn(commands.prompt(acp::PromptRequest::new(session, content)));
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || this.turn != turn || !cx.ai_enabled() {
                    return;
                }
                this.generating = false;
                this.prompt_attachments = None;
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
                        if !is_command
                            && !this.stopped
                            && response.stop_reason != acp::StopReason::Cancelled
                        {
                            this.fallback_history = false;
                            this.persist(cx);
                        }
                        this.dispatch_next(cx);
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
                        this.dispatch_next(cx);
                        cx.notify();
                    }
                    Err(error) => this.fail(&error.to_string(), cx),
                }
            });
        })
        .detach();
        cx.notify();
    }
}
