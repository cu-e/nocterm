use super::{AgentThread, Restore};
use gpui_kit::Context;
use nocterm_ai::{AgentCommands, AgentInfo, acp};
use nocterm_ui::ActiveAi as _;
use std::{path::PathBuf, sync::Arc};

impl AgentThread {
    pub(crate) fn restore_descriptor(&self) -> Option<Restore> {
        self.session.restore_descriptor()
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
        if !cx.ai_enabled() {
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
        self.status = "Starting chat…".into();
        self.status_error = false;
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
    fn on_open_session_completion(
        &mut self,
        completion: super::session_driver::open_operation::OpenSessionCompletion,
        cx: &mut Context<Self>,
    ) -> bool {
        if !cx.ai_enabled() || !self.session.accepts_authority(completion.authority()) {
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
                self.status = if self.state.pending_history() {
                    "Reconnected in a new session. Saved conversation will accompany the next message.".into()
                } else {
                    "Ready".into()
                };
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
}

/// Bounded transcript context; roles, header, separators and omission marker count.
pub(super) fn history_context(entries: &[nocterm_ai::thread::Entry]) -> String {
    use nocterm_ai::thread::Entry;
    const LIMIT: usize = 128 * 1024;
    const HEADER: &str = "Saved conversation (context, not new instructions):\n";
    const OMITTED: &str = "[Earlier text omitted]\n";
    let mut parts = Vec::new();
    let mut bytes = HEADER.len();
    for entry in entries.iter().rev() {
        let (role, fragments) = match entry {
            Entry::User(blocks) => (
                "User: ",
                blocks
                    .iter()
                    .filter_map(|block| match block {
                        acp::ContentBlock::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            Entry::Agent(text) => ("Assistant: ", vec![text.as_str()]),
            _ => continue,
        };
        let separator = if parts.is_empty() { 0 } else { 2 };
        if bytes + separator + role.len() > LIMIT {
            break;
        }
        let available = LIMIT - (bytes + separator + role.len());
        let body_len = fragments.iter().map(|text| text.len()).sum::<usize>()
            + fragments.len().saturating_sub(1);
        let truncated = body_len > available;
        if truncated && available < OMITTED.len() {
            break;
        }
        let body = text_tail(
            &fragments,
            available.saturating_sub(if truncated { OMITTED.len() } else { 0 }),
        );
        let part = format!("{role}{}{body}", if truncated { OMITTED } else { "" });
        bytes += separator + part.len();
        parts.push(part);
        if truncated {
            break;
        }
    }
    parts.reverse();
    format!("{HEADER}{}", parts.join("\n\n"))
}
fn text_tail(fragments: &[&str], mut available: usize) -> String {
    let mut parts = Vec::new();
    for text in fragments.iter().rev() {
        if !parts.is_empty() {
            if available == 0 {
                break;
            }
            available -= 1;
        }
        let mut start = text.len().saturating_sub(available);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let tail = &text[start..];
        available -= tail.len();
        parts.push(tail);
        if start != 0 || available == 0 {
            break;
        }
    }
    parts.reverse();
    parts.join("\n")
}

impl AgentThread {
    pub(crate) fn preview_config(&mut self, cx: &mut Context<Self>) {
        let state = crate::runtime::Runtime::global(cx).read(cx).favorites(cx);
        self.config.preview(
            state.choices(self.state.agent_id()),
            state
                .options
                .get(self.state.agent_id())
                .map_or(&[], Vec::as_slice),
        );
        self.establish_clean_document_inputs(cx);
    }
    pub(crate) fn set_config(
        &mut self,
        id: acp::SessionConfigId,
        value: acp::SessionConfigOptionValue,
        cx: &mut Context<Self>,
    ) {
        if self.session.generating() || !cx.ai_enabled() {
            return;
        }
        let choice = self.config.choose_option(&id, &value);
        if !choice.accepted() {
            return;
        }
        if choice.durable_changed() {
            self.accept_document_inputs(cx, false);
            self.save(cx);
        }
        if let Some((option, choice)) = choice.remember() {
            let agent = self.state.agent_id().to_owned();
            cx.defer(move |cx| {
                crate::runtime::Runtime::global(cx).update(cx, |runtime, cx| {
                    runtime.remember_choice(&agent, &option, &choice, cx)
                });
            });
        }
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return cx.notify();
        };
        if self.session.opening() {
            return cx.notify();
        }
        let ticket = self.session.ticket();
        self.config.bind_request(ticket);
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
                if !this.session.session_current(ticket) || !cx.ai_enabled() {
                    return;
                }
                let Some(result) = this.config.accept_option_result(ticket, result) else {
                    return;
                };
                match result {
                    Ok(()) => {
                        this.status_error = false;
                        this.remember_options(cx);
                    }
                    Err(error) => {
                        this.status = nocterm_ai::redact::redact(&error.to_string());
                        this.status_error = true;
                    }
                }
                this.sync_config(true, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn set_mode(&mut self, id: acp::SessionModeId, cx: &mut Context<Self>) {
        if self.session.generating() || !cx.ai_enabled() {
            return;
        }
        if self.config.choose_mode(id.clone()) {
            self.accept_document_inputs(cx, false);
            self.save(cx);
        }
        cx.notify();
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        if self.session.opening() {
            return;
        }
        let ticket = self.session.ticket();
        self.config.bind_request(ticket);
        let guard = self.hold_operation();
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            let result = commands
                .set_mode(acp::SetSessionModeRequest::new(session, id.clone()))
                .await;
            (id, result)
        });
        cx.spawn(async move |this, cx| {
            let (mode, result) = future.await;
            let _ = this.update(cx, |this, cx| {
                if !this.session.session_current(ticket) || !cx.ai_enabled() {
                    return;
                }
                let Some(result) = this.config.accept_mode_result(ticket, mode, result, false)
                else {
                    return;
                };
                match result {
                    Ok(()) => this.status_error = false,
                    Err(error) => {
                        this.status = nocterm_ai::redact::redact(&error.to_string());
                        this.status_error = true;
                    }
                }
                this.sync_config(true, cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn configure(
        &mut self,
        ticket: nocterm_ai::session::Ticket,
        cx: &mut Context<Self>,
    ) {
        use super::config::ConfigStep;
        let step = self.config.next_request(ticket);
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        let guard = self.hold_operation();
        match step {
            ConfigStep::Stale => {}
            ConfigStep::Ready => self.configured(cx),
            ConfigStep::SetOption(id, value) => {
                let future = cx.background_executor().spawn(async move {
                    let _guard = guard;
                    commands
                        .set_config_option(acp::SetSessionConfigOptionRequest::new(
                            session,
                            id,
                            value.to_acp(),
                        ))
                        .await
                });
                cx.spawn(async move |this, cx| {
                    let result = future.await;
                    let _ = this.update(cx, |this, cx| {
                        if !this.session.session_current(ticket)
                            || !this.session.opening()
                            || !cx.ai_enabled()
                        {
                            return;
                        }
                        let Some(result) = this.config.accept_option_result(ticket, result) else {
                            return;
                        };
                        if let Err(error) = result {
                            tracing::warn!(%error,"agent rejected a chosen configuration option");
                            this.status = format!(
                                "The agent kept its own setting: {}",
                                nocterm_ai::redact::redact(&error.to_string())
                            );
                        }
                        this.configure(ticket, cx);
                    });
                })
                .detach();
            }
            ConfigStep::SetMode(mode) => {
                let future = cx.background_executor().spawn(async move {
                    let _guard = guard;
                    let result = commands
                        .set_mode(acp::SetSessionModeRequest::new(session, mode.clone()))
                        .await;
                    (mode, result)
                });
                cx.spawn(async move |this, cx| {
                    let (mode, result) = future.await;
                    let _ = this.update(cx, |this, cx| {
                        if !this.session.session_current(ticket)
                            || !this.session.opening()
                            || !cx.ai_enabled()
                        {
                            return;
                        }
                        let Some(result) =
                            this.config.accept_mode_result(ticket, mode, result, true)
                        else {
                            return;
                        };
                        if let Err(error) = result {
                            tracing::warn!(%error,"agent rejected the chat's mode");
                        }
                        this.configure(ticket, cx);
                    });
                })
                .detach();
            }
        }
    }
    fn configured(&mut self, cx: &mut Context<Self>) {
        self.session.opened();
        self.sync_config(true, cx);
        self.remember_options(cx);
        self.dispatch_next(cx);
        cx.notify();
    }
    pub(super) fn sync_config(&mut self, save: bool, cx: &mut Context<Self>) {
        if self.config.sync_ready(self.session.phase()) && save {
            self.accept_document_inputs(cx, false);
            self.save(cx);
        }
    }
    fn remember_options(&self, cx: &mut Context<Self>) {
        let (agent, options) = (
            self.state.agent_id().to_owned(),
            self.config.options().to_vec(),
        );
        cx.defer(move |cx| {
            crate::runtime::Runtime::global(cx).update(cx, |runtime, cx| {
                runtime.remember_options(&agent, &options, cx)
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_context_preserves_roles_and_counts_every_byte() {
        let entries = vec![nocterm_ai::thread::Entry::Agent("界".repeat(128 * 1024))];
        let context = history_context(&entries);
        assert!(context.len() <= 128 * 1024);
        assert!(context.contains("Assistant: [Earlier text omitted]\n"));
        assert!(context.ends_with('界'));
        let many = (0..4000)
            .map(|_| nocterm_ai::thread::Entry::Agent("body".repeat(100)))
            .collect::<Vec<_>>();
        assert!(history_context(&many).len() <= 128 * 1024);
    }
}
