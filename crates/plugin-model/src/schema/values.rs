//! Values of settings fields and the edits that change them.

use std::collections::BTreeMap;

/// The value of one field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingValue {
    Bool(bool),
    Choice(String),
    Choices(Vec<String>),
    Number(i32),
    Text(String),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(on) => Some(*on),
            _ => None,
        }
    }

    pub fn as_choice(&self) -> Option<&str> {
        match self {
            Self::Choice(choice) => Some(choice),
            _ => None,
        }
    }

    pub fn as_choices(&self) -> Option<&[String]> {
        match self {
            Self::Choices(choices) => Some(choices),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i32> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }
}

/// The values of a plugin's fields by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingValues(BTreeMap<String, SettingValue>);

impl SettingValues {
    pub fn get(&self, key: &str) -> Option<&SettingValue> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &SettingValue)> {
        self.0.iter().map(|(key, value)| (key.as_str(), value))
    }

    pub fn insert(&mut self, key: impl Into<String>, value: SettingValue) {
        self.0.insert(key.into(), value);
    }
}

/// A change to one field, applied by [`apply`](crate::apply) to the value current at write time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldEdit {
    /// Replaces the whole value.
    Set(SettingValue),
    /// Turns one option of a multi-choice field on or off.
    Toggle { option: String, on: bool },
    /// Moves a number field one step up or down, stopping at its bounds.
    Step { up: bool },
}
