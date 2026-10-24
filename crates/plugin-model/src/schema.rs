//! The declarative settings schema of a plugin.
//!
//! A plugin describes its settings as plain data: groups of fields of a few closed kinds. The
//! host renders the form, validates values against the schema and applies edits on the current
//! value. Nothing here performs I/O or knows how values are stored.
//!
//! The kinds are toggle, choice, multi-choice (a closed option list), bounded integer and
//! single-line text. There is no colour, secret, masked or multi-line kind.

use std::borrow::Cow;

use crate::ContributionId;

mod edit;
mod marks;
mod text;
mod validate;
mod values;

pub use edit::{FieldError, apply, bounded, toggle_choice};
pub use text::sanitize_text;
pub use validate::{MAX_TEXT_LEN, SchemaError, SchemaLimit};
pub use values::{FieldEdit, SettingValue, SettingValues};

/// The settings a plugin declares, in groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsSchema {
    pub groups: Cow<'static, [FieldGroup]>,
}

impl SettingsSchema {
    /// A plugin without settings.
    pub const NONE: Self = Self {
        groups: Cow::Borrowed(&[]),
    };

    /// Every field of every group, in declaration order.
    pub fn fields(&self) -> impl Iterator<Item = &FieldSpec> {
        self.groups.iter().flat_map(|group| group.fields.iter())
    }

    /// Whether the schema declares no field at all.
    pub fn is_empty(&self) -> bool {
        self.fields().next().is_none()
    }

    pub fn field(&self, key: &str) -> Option<&FieldSpec> {
        self.fields().find(|field| field.key == key)
    }

    /// The default value of every field.
    pub fn defaults(&self) -> SettingValues {
        let mut values = SettingValues::default();
        for field in self.fields() {
            values.insert(field.key.to_string(), field.kind.default_value());
        }
        values
    }

    /// The groups that are shown for one contribution of the plugin. Plugin-level groups (no
    /// contribution) are not included.
    pub fn for_contribution<'a>(
        &'a self,
        id: &'a ContributionId,
    ) -> impl Iterator<Item = &'a FieldGroup> + 'a {
        self.groups
            .iter()
            .filter(move |group| group.contribution.as_ref() == Some(id))
    }
}

/// A titled run of fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldGroup {
    pub title: Cow<'static, str>,
    /// `None`: plugin-level; `Some`: shown for that contribution.
    pub contribution: Option<ContributionId>,
    pub fields: Cow<'static, [FieldSpec]>,
}

/// One setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// Equals the serde key in the owner's settings section.
    pub key: Cow<'static, str>,
    pub label: Cow<'static, str>,
    pub help: Option<Cow<'static, str>>,
    pub kind: FieldKind,
}

impl FieldSpec {
    /// The value this field has in `values`: the stored one when it has the field's kind, else the
    /// default. Every reader and every edit starts from this, so they never disagree.
    pub fn current(&self, values: &SettingValues) -> SettingValue {
        let default = self.kind.default_value();
        match values.get(&self.key) {
            Some(stored) if std::mem::discriminant(stored) == std::mem::discriminant(&default) => {
                stored.clone()
            }
            _ => default,
        }
    }
}

/// What a field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Toggle {
        default: bool,
    },
    Choice {
        options: Cow<'static, [ChoiceOption]>,
        default: Cow<'static, str>,
    },
    MultiChoice {
        options: Cow<'static, [ChoiceOption]>,
        default: Cow<'static, [Cow<'static, str>]>,
        /// Whether the order of the chosen options matters.
        ordered: bool,
        /// The most options that may be chosen.
        max: u8,
    },
    Number {
        min: i32,
        max: i32,
        step: i32,
        default: i32,
        unit: Option<Cow<'static, str>>,
    },
    /// Single-line text. Never masked; the value is data, not a secret.
    Text {
        default: Cow<'static, str>,
        /// In characters, 1 to [`MAX_TEXT_LEN`].
        max_len: u16,
        placeholder: Option<Cow<'static, str>>,
    },
}

impl FieldKind {
    /// The value a field of this kind has before the user sets it.
    pub fn default_value(&self) -> SettingValue {
        match self {
            Self::Toggle { default } => SettingValue::Bool(*default),
            Self::Choice { default, .. } => SettingValue::Choice(default.to_string()),
            Self::MultiChoice { default, .. } => {
                SettingValue::Choices(default.iter().map(ToString::to_string).collect())
            }
            Self::Number { default, .. } => SettingValue::Number(*default),
            Self::Text { default, .. } => SettingValue::Text(default.to_string()),
        }
    }
}

/// One option of a choice or multi-choice field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    pub key: Cow<'static, str>,
    pub label: Cow<'static, str>,
    pub help: Option<Cow<'static, str>>,
}

#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;
