//! Feature-owned configuration decisions over the document's advertised controls.
pub(in crate::thread) mod operation;
mod pending;
use super::controls::Controls;
use nocterm_ai::{
    history::source::ManagedNotice,
    model::{
        controls::{ConfigCategory, ConfigKind, ConfigOption, Modes, SelectChoices},
        ids::{ConfigId, ModeId, SessionId},
    },
    session::{SessionPhase, Ticket},
    session_config::{self, Choices, ConfigValue, StoredOption},
};
use pending::PendingNotices;

#[derive(Default)]
pub(crate) struct ChatConfig {
    choices: Choices,
    mode_choice: Option<ModeId>,
    /// The user's mode choice shown before the agent confirms it.
    chosen_mode: Option<ModeId>,
    /// Host-built options shown until an agent advertises its own.
    preview_options: Vec<ConfigOption>,
    last_prompt_model: Option<String>,
    last_saved_model: Option<String>,
    pending: PendingNotices,
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
    chosen_mode: Option<ModeId>,
    preview_options: Vec<ConfigOption>,
    last_prompt_model: Option<String>,
    last_saved_model: Option<String>,
}
impl ChatConfig {
    #[cfg(test)]
    pub(crate) fn preview_options(&self) -> &[ConfigOption] {
        &self.preview_options
    }
    /// The agent's own options, or the host's preview before it advertises any.
    pub(super) fn effective_options(&self, controls: &Controls) -> Vec<ConfigOption> {
        if controls.options.is_empty() {
            self.preview_options.clone()
        } else {
            controls.options.clone()
        }
    }
    pub(super) fn chosen_mode(&self) -> Option<&ModeId> {
        self.chosen_mode.as_ref()
    }
    pub(super) fn choices(&self) -> &Choices {
        &self.choices
    }
    pub(super) fn mode_choice(&self) -> Option<&ModeId> {
        self.mode_choice.as_ref()
    }
    pub(crate) fn model(&self) -> Option<String> {
        self.last_prompt_model
            .clone()
            .or_else(|| self.last_saved_model.clone())
    }
    pub(super) fn overrides_modes(&self, controls: &Controls) -> bool {
        self.effective_options(controls)
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
        self.preview_options =
            session_config::with_choices(session_config::restore(stored), &self.choices);
    }
    pub(super) fn choose_option(
        &mut self,
        id: &ConfigId,
        value: &ConfigValue,
        controls: &Controls,
    ) -> ChoiceOutcome {
        let ignored = || ChoiceOutcome {
            accepted: false,
            durable_changed: false,
            remember: None,
        };
        let choice = value.clone();
        let Some(option) = self
            .effective_options(controls)
            .into_iter()
            .find(|option| option.id == *id)
        else {
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
        session_config::choose(&mut self.preview_options, id.as_str(), &choice);
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
        self.chosen_mode = Some(mode);
        changed
    }
    pub(super) fn opening_started(&mut self, ticket: Ticket) {
        self.pending.clear();
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
    pub(super) fn current(&self, ticket: Ticket) -> bool {
        self.reconciliation
            .as_ref()
            .is_some_and(|state| state.ticket == ticket)
    }
    /// The opened session advertises its own controls; the host's preview ends.
    pub(super) fn opened(&mut self, ticket: Ticket) -> bool {
        if !self.current(ticket) {
            return false;
        }
        self.preview_options.clear();
        self.chosen_mode = None;
        true
    }
    pub(super) fn keep_control(&mut self, ticket: Ticket, notice: ManagedNotice) {
        if self.current(ticket) {
            self.pending.keep(notice);
        }
    }
    /// Control notices received before the session opened, for `session` only.
    pub(super) fn take_pending(&mut self, session: &SessionId) -> Vec<ManagedNotice> {
        self.pending.take_for(session)
    }
    /// The agent reported its own mode; the user's unconfirmed choice no longer applies.
    pub(super) fn agent_set_mode(&mut self) {
        self.chosen_mode = None;
    }
    pub(super) fn next_request(&mut self, ticket: Ticket, controls: &Controls) -> ConfigStep {
        if !self.current(ticket) {
            return ConfigStep::Stale;
        }
        let options = self.effective_options(controls);
        let wanted_mode = self.pending_mode(controls);
        let Some(state) = self.reconciliation.as_mut() else {
            return ConfigStep::Stale;
        };
        if let Some((id, value)) = session_config::requests(&options, &self.choices).into_iter().find(|(id, value)| !state.attempted.iter().any(|old| matches!(old, Attempt::Option(was, choice) if was.as_str() == id.as_str() && choice == value))) {
            state.attempted.push(Attempt::Option(id.to_string(), value.clone()));
            return ConfigStep::SetOption(id, value);
        }
        if let Some(wanted) = wanted_mode
            && !state
                .attempted
                .iter()
                .any(|old| matches!(old, Attempt::Mode(mode) if mode == &wanted))
        {
            state.attempted.push(Attempt::Mode(wanted.clone()));
            return ConfigStep::SetMode(wanted);
        }
        ConfigStep::Ready
    }
    fn current_mode(&self, controls: &Controls) -> Option<ModeId> {
        self.chosen_mode
            .clone()
            .or_else(|| controls.current_mode.clone())
    }
    fn pending_mode(&self, controls: &Controls) -> Option<ModeId> {
        if self.overrides_modes(controls) {
            return None;
        }
        let wanted = self.mode_choice.as_ref()?;
        let modes: &Modes = controls.modes.as_ref()?;
        let current = self
            .current_mode(controls)
            .unwrap_or_else(|| modes.current.clone());
        (&current != wanted && modes.choices.iter().any(|mode| &mode.id == wanted))
            .then(|| wanted.clone())
    }
    /// A failed mode request not made while opening drops the unconfirmed choice.
    pub(super) fn mode_failed(&mut self, opening: bool) {
        if !opening {
            self.chosen_mode = None;
        }
    }
    pub(super) fn sync_ready(&mut self, phase: SessionPhase, controls: &Controls) -> bool {
        if !matches!(
            phase,
            SessionPhase::Ready | SessionPhase::Prompting | SessionPhase::Cancelling
        ) {
            return false;
        }
        let mut changed = false;
        for (id, value) in session_config::current(&self.effective_options(controls)) {
            if self.choices.get(&id) != Some(&value) {
                self.choices.insert(id, value);
                changed = true;
            }
        }
        let mode = self.current_mode(controls);
        if mode.is_some() && mode != self.mode_choice {
            self.mode_choice = mode;
            changed = true;
        }
        changed
    }
    pub(super) fn prompt_started(&mut self, controls: &Controls) {
        self.last_prompt_model = self
            .effective_options(controls)
            .iter()
            .find(|option| option.category == Some(ConfigCategory::Model))
            .map(super::config_label)
            .filter(|model| !model.is_empty());
    }
    pub(super) fn detached(&mut self) {
        self.pending.clear();
        self.reconciliation = None;
    }
    pub(super) fn disabled(&mut self) {
        self.detached();
        self.preview_options.clear();
        self.chosen_mode = None;
    }
    pub(super) fn restart_snapshot(&self) -> RestartConfig {
        RestartConfig {
            choices: self.choices.clone(),
            mode_choice: self.mode_choice.clone(),
            chosen_mode: self.chosen_mode.clone(),
            preview_options: self.preview_options.clone(),
            last_prompt_model: self.last_prompt_model.clone(),
            last_saved_model: self.last_saved_model.clone(),
        }
    }
    pub(super) fn hydrate_restart(&mut self, saved: RestartConfig) {
        *self = Self {
            choices: saved.choices,
            mode_choice: saved.mode_choice,
            chosen_mode: saved.chosen_mode,
            preview_options: saved.preview_options,
            last_prompt_model: saved.last_prompt_model,
            last_saved_model: saved.last_saved_model,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests;

pub(crate) fn config_label(option: &ConfigOption) -> String {
    match &option.kind {
        ConfigKind::Select { current, choices } => {
            let options = match choices {
                SelectChoices::Flat(values) => values.iter().collect::<Vec<_>>(),
                SelectChoices::Groups(groups) => groups
                    .iter()
                    .flat_map(|group| group.choices.iter())
                    .collect(),
            };
            options
                .iter()
                .find(|value| value.id == *current)
                .map(|value| value.name.clone())
                .unwrap_or_else(|| current.to_string())
        }
        ConfigKind::Boolean(value) => {
            if *value {
                "On".into()
            } else {
                "Off".into()
            }
        }
        ConfigKind::Unsupported { .. } => String::new(),
    }
}
