use super::ids::{ConfigChoiceId, ConfigGroupId, ConfigId, ModeId};

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigOption {
    pub id: ConfigId,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<ConfigCategory>,
    pub kind: ConfigKind,
}
impl ConfigOption {
    pub fn new(id: impl Into<ConfigId>, name: impl Into<String>, kind: ConfigKind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            category: None,
            kind,
        }
    }
    pub fn select(
        id: impl Into<ConfigId>,
        name: impl Into<String>,
        current: impl Into<ConfigChoiceId>,
        choices: impl Into<ConfigChoices>,
    ) -> Self {
        Self::new(
            id,
            name,
            ConfigKind::Select(ConfigSelect {
                current: current.into(),
                choices: choices.into(),
            }),
        )
    }
    pub fn boolean(id: impl Into<ConfigId>, name: impl Into<String>, current: bool) -> Self {
        Self::new(id, name, ConfigKind::Boolean(current))
    }
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    pub fn category(mut self, category: ConfigCategory) -> Self {
        self.category = Some(category);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigKind {
    Boolean(bool),
    Select(ConfigSelect),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigSelect {
    pub current: ConfigChoiceId,
    pub choices: ConfigChoices,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigChoices {
    Flat(Vec<ConfigChoice>),
    Grouped(Vec<ConfigGroup>),
}
impl From<Vec<ConfigChoice>> for ConfigChoices {
    fn from(choices: Vec<ConfigChoice>) -> Self {
        Self::Flat(choices)
    }
}
impl From<Vec<ConfigGroup>> for ConfigChoices {
    fn from(groups: Vec<ConfigGroup>) -> Self {
        Self::Grouped(groups)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigChoice {
    pub id: ConfigChoiceId,
    pub name: String,
    pub description: Option<String>,
}
impl ConfigChoice {
    pub fn new(id: impl Into<ConfigChoiceId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
        }
    }
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigGroup {
    pub id: ConfigGroupId,
    pub name: String,
    pub choices: Vec<ConfigChoice>,
}
impl ConfigGroup {
    pub fn new(
        id: impl Into<ConfigGroupId>,
        name: impl Into<String>,
        choices: Vec<ConfigChoice>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            choices,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigCategory {
    Mode,
    Model,
    ModelConfig,
    ThoughtLevel,
    Other(String),
}

/// The value to apply to one configuration option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigSetting {
    Boolean(bool),
    Choice(ConfigChoiceId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modes {
    pub current: ModeId,
    pub choices: Vec<ModeChoice>,
}

impl Modes {
    pub fn new(current: impl Into<ModeId>, choices: Vec<ModeChoice>) -> Self {
        Self {
            current: current.into(),
            choices,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModeChoice {
    pub id: ModeId,
    pub name: String,
    pub description: Option<String>,
}
impl ModeChoice {
    pub fn new(id: impl Into<ModeId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
        }
    }
}

/// A sparse text update: absent keeps, null clears.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Change<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}
