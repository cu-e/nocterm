//! Feature-owned chat configuration; legacy wire values remain at existing seams.
use nocterm_ai::model::commands::AvailableCommand;
use nocterm_ai::model::controls::{ConfigCategory, ConfigOption, ConfigSetting, Modes};
use nocterm_ai::model::ids::{ConfigId, ModeId, SessionId};
use nocterm_ai::model::session::{SessionNotice, SessionUpdate};
pub(in crate::thread) mod operation;
use nocterm_ai::{
    AgentError,
    session::{SessionPhase, Ticket},
    session_config::{self, Choices, ConfigValue, StoredOption},
};

#[derive(Default)]
pub struct ChatConfig {
    choices: Choices,
    mode_choice: Option<ModeId>,
    options: Vec<ConfigOption>,
    modes: Option<Modes>,
    current_mode: Option<ModeId>,
    commands: Vec<AvailableCommand>,
    commands_revision: u64,
    last_prompt_model: Option<String>,
    last_saved_model: Option<String>,
    pending_controls: Vec<(SessionId, SessionUpdate)>,
    reconciliation: Option<Reconciliation>,
}
struct Reconciliation {
    ticket: Ticket,
    attempted: Vec<Attempt>,
}
enum Attempt {
    Option(String, ConfigValue),
    Mode(ModeId),
}
pub(super) enum ConfigStep {
    SetOption(ConfigId, ConfigValue),
    SetMode(ModeId),
    Ready,
    Stale,
}
pub(super) struct ChoiceOutcome {
    accepted: bool,
    durable_changed: bool,
    remember: Option<(ConfigOption, ConfigValue)>,
}
impl ChoiceOutcome {
    pub(super) fn accepted(&self) -> bool {
        self.accepted
    }
    pub(super) fn durable_changed(&self) -> bool {
        self.durable_changed
    }
    pub(super) fn remember(self) -> Option<(ConfigOption, ConfigValue)> {
        self.remember
    }
}
#[derive(Clone)]
pub(super) struct RestartConfig {
    choices: Choices,
    mode_choice: Option<ModeId>,
    options: Vec<ConfigOption>,
    modes: Option<Modes>,
    current_mode: Option<ModeId>,
    commands: Vec<AvailableCommand>,
    commands_revision: u64,
    last_prompt_model: Option<String>,
    last_saved_model: Option<String>,
}
impl ChatConfig {
    pub fn options(&self) -> &[ConfigOption] {
        &self.options
    }
    pub fn preview_options(&self, opening: bool) -> Vec<ConfigOption> {
        if opening {
            session_config::with_choices(self.options.clone(), &self.choices)
        } else {
            self.options.clone()
        }
    }
    pub fn modes(&self) -> Option<&Modes> {
        self.modes.as_ref()
    }
    pub fn current_mode(&self) -> Option<&ModeId> {
        self.current_mode.as_ref()
    }
    pub fn commands(&self) -> &[AvailableCommand] {
        &self.commands
    }
    pub fn commands_revision(&self) -> u64 {
        self.commands_revision
    }
    pub(super) fn choices(&self) -> &Choices {
        &self.choices
    }
    pub(super) fn mode_choice(&self) -> Option<&ModeId> {
        self.mode_choice.as_ref()
    }
    pub fn model(&self) -> Option<String> {
        self.last_prompt_model
            .clone()
            .or_else(|| self.last_saved_model.clone())
    }
    pub fn config_overrides_modes(&self) -> bool {
        self.options
            .iter()
            .any(|option| option.category == Some(ConfigCategory::Mode))
    }
    pub(super) fn restore(
        &mut self,
        choices: Choices,
        mode: Option<ModeId>,
        model: Option<String>,
    ) {
        self.choices = choices;
        self.mode_choice = mode;
        self.last_saved_model = model;
    }
    pub(super) fn saved_model(&mut self, model: Option<String>) {
        self.last_saved_model = model;
    }
    pub(super) fn preview(&mut self, mut defaults: Choices, stored: &[StoredOption]) {
        defaults.extend(std::mem::take(&mut self.choices));
        self.choices = defaults;
        self.options = session_config::with_choices(session_config::restore(stored), &self.choices);
    }
    pub(super) fn choose_option(&mut self, id: &ConfigId, value: &ConfigSetting) -> ChoiceOutcome {
        let ignored = || ChoiceOutcome {
            accepted: false,
            durable_changed: false,
            remember: None,
        };
        let choice = ConfigValue::from_setting(value);
        let Some(option) = self.options.iter().find(|option| option.id == *id).cloned() else {
            return ignored();
        };
        let mut offered = vec![option.clone()];
        // choose reports whether the displayed value changes; current is also a valid selection.
        let current = session_config::current(&offered)
            .get(&id.to_string())
            .cloned();
        if current.as_ref() != Some(&choice)
            && !session_config::choose(&mut offered, id.as_str(), &choice)
        {
            return ignored();
        }
        session_config::choose(&mut self.options, id.as_str(), &choice);
        let changed = self.choices.get(&id.to_string()) != Some(&choice);
        self.choices.insert(id.to_string(), choice.clone());
        ChoiceOutcome {
            accepted: true,
            durable_changed: changed,
            remember: Some((option, choice)),
        }
    }
    pub(super) fn choose_mode(&mut self, mode: ModeId) -> bool {
        let changed = self.mode_choice.as_ref() != Some(&mode);
        self.mode_choice = Some(mode.clone());
        self.current_mode = Some(mode);
        changed
    }
    pub(super) fn opening_started(&mut self, ticket: Ticket) {
        self.pending_controls.clear();
        self.reconciliation = Some(Reconciliation {
            ticket,
            attempted: Vec::new(),
        });
    }
    pub(super) fn bind_request(&mut self, ticket: Ticket) {
        if !self.current(ticket) {
            self.reconciliation = Some(Reconciliation {
                ticket,
                attempted: Vec::new(),
            });
        }
    }
    fn current(&self, ticket: Ticket) -> bool {
        self.reconciliation
            .as_ref()
            .is_some_and(|state| state.ticket == ticket)
    }
    pub(super) fn start_open(
        &mut self,
        ticket: Ticket,
        modes: Option<Modes>,
        options: Vec<ConfigOption>,
    ) {
        if !self.current(ticket) {
            return;
        }
        self.modes = modes;
        self.current_mode = None;
        self.options = options;
    }
    pub(super) fn begin_reconciliation(&mut self, session: &SessionId, ticket: Ticket) {
        if !self.current(ticket) {
            return;
        }
        for (id, update) in std::mem::take(&mut self.pending_controls) {
            if &id == session {
                self.report_control(&update);
            }
        }
    }
    pub(super) fn keep_control(&mut self, ticket: Ticket, notification: &SessionNotice) {
        if !self.current(ticket) || !Self::is_control(&notification.update) {
            return;
        }
        let kind = std::mem::discriminant(&notification.update);
        self.pending_controls.retain(|(id, update)| {
            id != &notification.session || std::mem::discriminant(update) != kind
        });
        if self.pending_controls.len() == 64 {
            self.pending_controls.remove(0);
        }
        self.pending_controls
            .push((notification.session.clone(), notification.update.clone()));
    }
    fn is_control(update: &SessionUpdate) -> bool {
        matches!(
            update,
            SessionUpdate::AvailableCommandsUpdate(_)
                | SessionUpdate::CurrentModeUpdate(_)
                | SessionUpdate::ConfigOptionUpdate(_)
        )
    }
    pub(super) fn report_control(&mut self, update: &SessionUpdate) -> bool {
        match update {
            SessionUpdate::AvailableCommandsUpdate(update) => {
                self.commands = update.clone();
                self.commands_revision = self.commands_revision.wrapping_add(1);
            }
            SessionUpdate::CurrentModeUpdate(update) => self.current_mode = Some(update.clone()),
            SessionUpdate::ConfigOptionUpdate(update) => self.options = update.clone(),
            _ => return false,
        }
        true
    }
    pub(super) fn next_request(&mut self, ticket: Ticket) -> ConfigStep {
        if !self.current(ticket) {
            return ConfigStep::Stale;
        }
        let Some(state) = self.reconciliation.as_mut() else {
            return ConfigStep::Stale;
        };
        if let Some((id, value)) = session_config::requests(&self.options, &self.choices).into_iter().find(|(id, value)| !state.attempted.iter().any(|old| matches!(old, Attempt::Option(was, choice) if *was == *id.as_str() && choice == value))) {
            state.attempted.push(Attempt::Option(id.to_string(), value.clone()));
            return ConfigStep::SetOption(id, value);
        }
        if let Some(wanted) = self.pending_mode() {
            let Some(state) = self.reconciliation.as_mut() else {
                return ConfigStep::Stale;
            };
            if !state
                .attempted
                .iter()
                .any(|old| matches!(old, Attempt::Mode(mode) if mode == &wanted))
            {
                state.attempted.push(Attempt::Mode(wanted.clone()));
                return ConfigStep::SetMode(wanted);
            }
        }
        ConfigStep::Ready
    }
    fn pending_mode(&self) -> Option<ModeId> {
        if self.config_overrides_modes() {
            return None;
        }
        let wanted = self.mode_choice.as_ref()?;
        let modes = self.modes.as_ref()?;
        let current = self.current_mode.as_ref().unwrap_or(&modes.current);
        (current != wanted && modes.choices.iter().any(|mode| &mode.id == wanted))
            .then(|| wanted.clone())
    }
    pub(super) fn accept_option_result(
        &mut self,
        ticket: Ticket,
        result: Result<Vec<ConfigOption>, AgentError>,
    ) -> Option<Result<(), AgentError>> {
        if !self.current(ticket) {
            return None;
        }
        Some(result.map(|options| self.options = options))
    }
    pub(super) fn accept_mode_result(
        &mut self,
        ticket: Ticket,
        mode: ModeId,
        result: Result<(), AgentError>,
        opening: bool,
    ) -> Option<Result<(), AgentError>> {
        if !self.current(ticket) {
            return None;
        }
        Some(
            result
                .map(|()| self.current_mode = Some(mode))
                .inspect_err(|_| {
                    if !opening {
                        self.current_mode = None;
                    }
                }),
        )
    }
    pub(super) fn sync_ready(&mut self, phase: SessionPhase) -> bool {
        if !matches!(
            phase,
            SessionPhase::Ready | SessionPhase::Prompting | SessionPhase::Cancelling
        ) {
            return false;
        }
        let mut changed = false;
        for (id, value) in session_config::current(&self.options) {
            if self.choices.get(&id) != Some(&value) {
                self.choices.insert(id, value);
                changed = true;
            }
        }
        let mode = self
            .current_mode
            .clone()
            .or_else(|| self.modes.as_ref().map(|modes| modes.current.clone()));
        if mode.is_some() && mode != self.mode_choice {
            self.mode_choice = mode;
            changed = true;
        }
        changed
    }
    pub(super) fn prompt_started(&mut self) {
        self.last_prompt_model = self
            .options
            .iter()
            .find(|option| option.category == Some(ConfigCategory::Model))
            .map(super::config_label)
            .filter(|model| !model.is_empty());
    }
    pub(super) fn detached(&mut self) {
        self.pending_controls.clear();
        self.reconciliation = None;
    }
    pub(super) fn disabled(&mut self) {
        self.detached();
        self.options.clear();
        self.modes = None;
        self.current_mode = None;
        self.commands.clear();
        self.commands_revision = 0;
    }
    pub(super) fn restart_snapshot(&self) -> RestartConfig {
        RestartConfig {
            choices: self.choices.clone(),
            mode_choice: self.mode_choice.clone(),
            options: self.options.clone(),
            modes: self.modes.clone(),
            current_mode: self.current_mode.clone(),
            commands: self.commands.clone(),
            commands_revision: self.commands_revision,
            last_prompt_model: self.last_prompt_model.clone(),
            last_saved_model: self.last_saved_model.clone(),
        }
    }
    pub(super) fn hydrate_restart(&mut self, saved: RestartConfig) {
        *self = Self {
            choices: saved.choices,
            mode_choice: saved.mode_choice,
            options: saved.options,
            modes: saved.modes,
            current_mode: saved.current_mode,
            commands: saved.commands,
            commands_revision: saved.commands_revision,
            last_prompt_model: saved.last_prompt_model,
            last_saved_model: saved.last_saved_model,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests;
