//! Transient display values read from the Live document's control slots.
//!
//! The document owns the advertised options, modes and commands; this view
//! copies the small strings a menu needs for one frame and keeps no state.
use super::config::ChatConfig;
use nocterm_ai::{
    history::physical::{
        CategorySlot, CommandSlot, ConfigChoiceSlot, ConfigKindSlot, ConfigOptionSlot,
        LiveControls, SelectChoicesSlot,
    },
    model::{
        controls::{
            ConfigCategory, ConfigChoice, ConfigGroup, ConfigKind, ConfigOption, Modes,
            SelectChoices,
        },
        ids::ModeId,
        notice::AdvertisedCommand,
    },
    session_config,
};

#[derive(Clone, Copy)]
pub(crate) struct ControlsView<'a> {
    controls: Option<&'a LiveControls>,
    config: &'a ChatConfig,
}
impl<'a> ControlsView<'a> {
    pub(super) fn new(controls: Option<&'a LiveControls>, config: &'a ChatConfig) -> Self {
        Self { controls, config }
    }
    /// The agent's options, else the host's preview, with the chosen values shown over them.
    pub(crate) fn options(&self) -> Vec<ConfigOption> {
        session_config::with_choices(
            self.config
                .effective_options(&Controls::read(self.controls)),
            self.config.choices(),
        )
    }
    pub(crate) fn modes(&self) -> Option<Modes> {
        self.controls?.modes().map(|modes| (*modes).clone())
    }
    pub(crate) fn current_mode(&self) -> Option<ModeId> {
        self.config
            .chosen_mode()
            .cloned()
            .or_else(|| Controls::read(self.controls).current_mode)
    }
    pub(crate) fn commands(&self) -> Vec<AdvertisedCommand> {
        self.controls.map_or_else(Vec::new, |controls| {
            controls.commands().iter().map(command).collect()
        })
    }
    pub(crate) fn commands_revision(&self) -> u64 {
        self.controls.map_or(0, LiveControls::commands_revision)
    }
    pub(crate) fn config_overrides_modes(&self) -> bool {
        self.config.overrides_modes(&Controls::read(self.controls))
    }
}

/// The agent's current controls as owned values, read before a config decision.
pub(super) struct Controls {
    pub(super) options: Vec<ConfigOption>,
    pub(super) modes: Option<Modes>,
    pub(super) current_mode: Option<ModeId>,
}
impl Controls {
    pub(super) fn read(controls: Option<&LiveControls>) -> Self {
        let Some(controls) = controls else {
            return Self {
                options: Vec::new(),
                modes: None,
                current_mode: None,
            };
        };
        let modes = controls.modes().map(|modes| (*modes).clone());
        let current_mode = controls
            .mode()
            .map(|mode| (*mode).clone())
            .or_else(|| modes.as_ref().map(|modes| modes.current.clone()));
        Self {
            options: controls.configuration().iter().map(option).collect(),
            modes,
            current_mode,
        }
    }
}

pub(super) fn option(slot: &ConfigOptionSlot) -> ConfigOption {
    ConfigOption {
        id: (*slot.id()).clone(),
        name: slot.name().to_string(),
        description: slot.description().map(|text| text.to_string()),
        category: slot.category().map(category),
        kind: kind(slot.kind()),
        extensions: None,
    }
}
fn category(slot: &CategorySlot) -> ConfigCategory {
    match slot {
        CategorySlot::Mode => ConfigCategory::Mode,
        CategorySlot::Model => ConfigCategory::Model,
        CategorySlot::ModelConfig => ConfigCategory::ModelConfig,
        CategorySlot::ThoughtLevel => ConfigCategory::ThoughtLevel,
        CategorySlot::Other(text) => ConfigCategory::Other(text.read().to_string()),
    }
}
fn kind(slot: &ConfigKindSlot) -> ConfigKind {
    match slot {
        ConfigKindSlot::Boolean(value) => ConfigKind::Boolean(*value),
        ConfigKindSlot::Unsupported(label) => ConfigKind::Unsupported {
            label: label.read().to_string(),
        },
        ConfigKindSlot::Select { current, choices } => ConfigKind::Select {
            current: (*current.read()).clone(),
            choices: match choices {
                SelectChoicesSlot::Flat(values) => {
                    SelectChoices::Flat(values.iter().map(choice).collect())
                }
                SelectChoicesSlot::Groups(groups) => SelectChoices::Groups(
                    groups
                        .iter()
                        .map(|group| ConfigGroup {
                            id: (*group.id()).clone(),
                            name: group.name().to_string(),
                            choices: group.choices().iter().map(choice).collect(),
                            extensions: None,
                        })
                        .collect(),
                ),
            },
        },
    }
}
fn choice(slot: &ConfigChoiceSlot) -> ConfigChoice {
    ConfigChoice {
        id: (*slot.id()).clone(),
        name: slot.name().to_string(),
        description: slot.description().map(|text| text.to_string()),
        extensions: None,
    }
}
fn command(slot: &CommandSlot) -> AdvertisedCommand {
    AdvertisedCommand {
        name: slot.name().to_string(),
        description: slot.description().to_string(),
        input_hint: slot.input_hint().map(|hint| hint.to_string()),
    }
}
