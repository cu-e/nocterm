//! Session configuration: chosen before the session opens, applied once it
//! does, and remembered for the agent's next chats.
use super::AgentThread;
use crate::runtime::Runtime;
use gpui_kit::Context;
use nocterm_ai::{
    acp,
    session::Ticket,
    session_config::{self, ConfigValue},
};
use nocterm_ui::ActiveAi as _;

impl AgentThread {
    /// Starts a new chat with the options its agent last reported and the
    /// model and reasoning effort last chosen for it, so they can be changed
    /// before the session opens.
    pub(crate) fn adopt_agent_config(&mut self, cx: &mut Context<Self>) {
        let state = &Runtime::global(cx).read(cx).favorites;
        self.state.config_options = state.preview(&self.agent_id);
        self.config_choices = state.choices(&self.agent_id);
    }

    pub(crate) fn set_config(
        &mut self,
        id: acp::SessionConfigId,
        value: acp::SessionConfigOptionValue,
        cx: &mut Context<Self>,
    ) {
        if self.lifecycle.generating() || !cx.ai_enabled() {
            return;
        }
        let choice = ConfigValue::from_acp(&value);
        if let (Some(choice), Some(option)) = (
            &choice,
            self.state
                .config_options
                .iter()
                .find(|option| option.id == id),
        ) {
            let (agent, option, choice) = (self.agent_id.clone(), option.clone(), choice.clone());
            cx.defer(move |cx| {
                Runtime::global(cx).update(cx, |runtime, cx| {
                    runtime.remember_choice(&agent, &option, &choice, cx)
                })
            });
        }
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return self.choose_before_open(&id, choice);
        };
        if self.lifecycle.opening() {
            return self.choose_before_open(&id, choice);
        }
        let ticket = self.lifecycle.ticket();
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
                if !this.lifecycle.session_current(ticket) || !cx.ai_enabled() {
                    return;
                }
                match result {
                    Ok(options) => {
                        this.state.config_options = options;
                        this.status_error = false;
                        this.remember_options(cx);
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

    /// Shows a choice made before the session opened and keeps it for when
    /// it does.
    fn choose_before_open(&mut self, id: &acp::SessionConfigId, choice: Option<ConfigValue>) {
        let Some(choice) = choice else {
            return;
        };
        if session_config::choose(&mut self.state.config_options, &id.0, &choice) {
            self.config_choices.insert(id.to_string(), choice);
        }
    }

    /// Gives the just opened session the configuration chosen before, one
    /// option at a time since a model decides the efforts it offers, then
    /// makes the chat ready.
    pub(super) fn configure(&mut self, ticket: Ticket, cx: &mut Context<Self>) {
        let next = session_config::requests(&self.state.config_options, &self.config_choices)
            .into_iter()
            .next();
        let (Some((id, value)), Some(commands), Some(session)) =
            (next, self.commands().clone(), self.session().clone())
        else {
            self.config_choices.clear();
            self.remember_options(cx);
            self.lifecycle.opened();
            self.dispatch_next(cx);
            cx.notify();
            return;
        };
        self.config_choices.remove(&*id.0);
        let guard = self.hold_operation();
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
                if !this.lifecycle.session_current(ticket)
                    || !this.lifecycle.opening()
                    || !cx.ai_enabled()
                {
                    return;
                }
                match result {
                    Ok(options) => this.state.config_options = options,
                    // The agent's own choice stands; the chat still opens.
                    Err(error) => {
                        tracing::warn!(%error, "agent rejected a chosen configuration option");
                        this.status = format!(
                            "The agent kept its own setting: {}",
                            nocterm_ai::redact::redact(&error.to_string())
                        );
                    }
                }
                this.configure(ticket, cx);
            });
        })
        .detach();
    }

    /// Keeps the options the agent reported, for chats not connected yet.
    fn remember_options(&self, cx: &mut Context<Self>) {
        let (agent, options) = (self.agent_id.clone(), self.state.config_options.clone());
        cx.defer(move |cx| {
            Runtime::global(cx).update(cx, |runtime, cx| {
                runtime.remember_options(&agent, &options, cx)
            })
        });
    }
}
