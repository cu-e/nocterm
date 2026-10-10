//! Session configuration: the chat's own model, effort and mode, chosen
//! before a session opens or reported by an open one, and given to every
//! session the chat opens.
//!
//! An agent opens each session with its own defaults, so a chat whose
//! connection was released, failed or restarted would otherwise lose what
//! the user chose. The chat keeps its configuration in `config_choices` and
//! `mode_choice`; opening a session reconciles the agent's values with them,
//! and once the session is ready, what the agent reports becomes the chat's
//! configuration.
use super::AgentThread;
use crate::runtime::Runtime;
use gpui_kit::Context;
use nocterm_ai::{
    acp,
    session::{SessionPhase, Ticket},
    session_config::{self, ConfigValue},
};
use nocterm_ui::ActiveAi as _;

impl AgentThread {
    /// Shows a chat that is not connected with the options its agent last
    /// reported and the chat's configuration over them. A chat without its
    /// own choice starts with the model and reasoning effort last chosen for
    /// the agent.
    pub(crate) fn preview_config(&mut self, cx: &mut Context<Self>) {
        let state = &Runtime::global(cx).read(cx).favorites;
        let mut choices = state.choices(&self.agent_id);
        choices.extend(std::mem::take(&mut self.config_choices));
        self.config_choices = choices;
        let options =
            session_config::restore(state.options.get(&self.agent_id).map_or(&[], Vec::as_slice));
        self.state.config_options = session_config::with_choices(options, &self.config_choices);
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
        let Some(choice) = ConfigValue::from_acp(&value) else {
            return;
        };
        if let Some(option) = self
            .state
            .config_options
            .iter()
            .find(|option| option.id == id)
        {
            let (agent, option, choice) = (self.agent_id.clone(), option.clone(), choice.clone());
            cx.defer(move |cx| {
                Runtime::global(cx).update(cx, |runtime, cx| {
                    runtime.remember_choice(&agent, &option, &choice, cx)
                })
            });
        }
        // Kept at once, so a session lost before the agent answers still
        // gets it when the chat reconnects.
        if session_config::choose(&mut self.state.config_options, &id.0, &choice) {
            self.config_choices.insert(id.to_string(), choice);
            self.save(cx);
        }
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return cx.notify();
        };
        if self.lifecycle.opening() {
            // Opening applies the chat's configuration before it is ready.
            return cx.notify();
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
                // A rejected choice gives way to the agent's value.
                this.sync_config(true, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn set_mode(&mut self, id: acp::SessionModeId, cx: &mut Context<Self>) {
        if self.lifecycle.generating() || !cx.ai_enabled() {
            return;
        }
        self.mode_choice = Some(id.clone());
        self.state.current_mode = Some(id.clone());
        self.save(cx);
        cx.notify();
        let (Some(commands), Some(session)) = (self.commands().clone(), self.session().clone())
        else {
            return;
        };
        if self.lifecycle.opening() {
            return;
        }
        let ticket = self.lifecycle.ticket();
        let guard = self.hold_operation();
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            commands
                .set_mode(acp::SetSessionModeRequest::new(session, id))
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if !this.lifecycle.session_current(ticket) || !cx.ai_enabled() {
                    return;
                }
                match result {
                    Ok(()) => this.status_error = false,
                    Err(error) => {
                        // The agent's mode stands.
                        this.state.current_mode = None;
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

    /// Gives the just opened session the chat's configuration, one option at
    /// a time since a model decides the efforts it offers, then its mode,
    /// then makes the chat ready. `reported` holds the agent's own values;
    /// the chat shows its choices meanwhile. A value is asked for once per
    /// opening, so an agent that keeps its own cannot loop.
    pub(super) fn configure(
        &mut self,
        ticket: Ticket,
        reported: Vec<acp::SessionConfigOption>,
        mut asked: Vec<(String, ConfigValue)>,
        cx: &mut Context<Self>,
    ) {
        let next = session_config::requests(&reported, &self.config_choices)
            .into_iter()
            .find(|(id, value)| !asked.iter().any(|(old, was)| *old == *id.0 && was == value));
        let (Some((id, value)), Some(commands), Some(session)) =
            (next, self.commands().clone(), self.session().clone())
        else {
            self.state.config_options = reported;
            self.configure_mode(ticket, cx);
            return;
        };
        self.state.config_options =
            session_config::with_choices(reported.clone(), &self.config_choices);
        asked.push((id.to_string(), value.clone()));
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
                let reported = match result {
                    Ok(options) => options,
                    // The agent's own choice stands; the chat still opens.
                    Err(error) => {
                        tracing::warn!(%error, "agent rejected a chosen configuration option");
                        this.status = format!(
                            "The agent kept its own setting: {}",
                            nocterm_ai::redact::redact(&error.to_string())
                        );
                        reported
                    }
                };
                this.configure(ticket, reported, asked, cx);
            });
        })
        .detach();
    }

    /// The mode step of [`Self::configure`], for agents whose modes are not
    /// configuration options.
    fn configure_mode(&mut self, ticket: Ticket, cx: &mut Context<Self>) {
        let (Some(mode), Some(commands), Some(session)) = (
            self.pending_mode(),
            self.commands().clone(),
            self.session().clone(),
        ) else {
            return self.configured(cx);
        };
        let guard = self.hold_operation();
        let request = acp::SetSessionModeRequest::new(session, mode.clone());
        let future = cx.background_executor().spawn(async move {
            let _guard = guard;
            commands.set_mode(request).await
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
                    Ok(()) => this.state.current_mode = Some(mode),
                    Err(error) => {
                        tracing::warn!(%error, "agent rejected the chat's mode");
                    }
                }
                this.configured(cx);
            });
        })
        .detach();
    }

    /// The chat's mode, when the session offers it and has another one.
    fn pending_mode(&self) -> Option<acp::SessionModeId> {
        let wanted = self.mode_choice.as_ref()?;
        let modes = self.state.modes.as_ref()?;
        if self.state.config_overrides_modes() {
            return None;
        }
        let current = self
            .state
            .current_mode
            .as_ref()
            .unwrap_or(&modes.current_mode_id);
        (current != wanted && modes.available_modes.iter().any(|mode| &mode.id == wanted))
            .then(|| wanted.clone())
    }

    fn configured(&mut self, cx: &mut Context<Self>) {
        self.lifecycle.opened();
        self.sync_config(true, cx);
        self.remember_options(cx);
        self.dispatch_next(cx);
        cx.notify();
    }

    /// Once the session is ready, the chat's configuration is what the
    /// agent reports, including changes the agent makes on its own. Before
    /// that, reported values are the agent's defaults and change nothing.
    pub(super) fn sync_config(&mut self, save: bool, cx: &mut Context<Self>) {
        if !matches!(
            self.lifecycle.phase(),
            SessionPhase::Ready | SessionPhase::Prompting | SessionPhase::Cancelling
        ) {
            return;
        }
        let mut changed = false;
        for (id, value) in session_config::current(&self.state.config_options) {
            if self.config_choices.get(&id) != Some(&value) {
                self.config_choices.insert(id, value);
                changed = true;
            }
        }
        let mode = self.state.current_mode.clone().or_else(|| {
            self.state
                .modes
                .as_ref()
                .map(|modes| modes.current_mode_id.clone())
        });
        if mode.is_some() && mode != self.mode_choice {
            self.mode_choice = mode;
            changed = true;
        }
        if changed && save {
            self.save(cx);
        }
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
