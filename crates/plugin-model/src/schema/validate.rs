//! The limits a settings schema must stay within.

use std::{collections::HashSet, fmt};

use thiserror::Error;

use super::{ChoiceOption, FieldKind, FieldSpec, SettingsSchema, sanitize_text};
use crate::ContributionId;

const MAX_GROUPS: usize = 8;
const MAX_FIELDS: usize = 32;
const MAX_OPTIONS: usize = 16;
const MAX_KEY: usize = 32;
const MAX_LABEL: usize = 48;
const MAX_HELP: usize = 160;
/// The longest a text field may be allowed to grow, in characters.
pub const MAX_TEXT_LEN: u16 = 256;

/// What a schema has too many of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaLimit {
    Groups,
    Fields,
    Options,
}

impl fmt::Display for SchemaLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Groups => "groups",
            Self::Fields => "fields",
            Self::Options => "options in one field",
        })
    }
}

/// A schema that breaks a limit. The catalog refuses it at bootstrap.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SchemaError {
    #[error("a schema may have at most {max} {limit}")]
    TooMany { limit: SchemaLimit, max: usize },
    #[error("`{key}` is not a lowercase letters, digits and underscores key")]
    Key { key: String },
    #[error("`{key}` is declared twice")]
    DuplicateKey { key: String },
    #[error("group {index} has a title that is empty, too long or not plain text")]
    GroupTitle { index: usize },
    #[error("`{key}` has a label or help that is empty, too long or not plain text")]
    Label { key: String },
    #[error("`{key}` has no options, or options with repeated or malformed keys")]
    Options { key: String },
    #[error("`{key}` has an empty or inverted range, a zero step or a bad limit")]
    Range { key: String },
    #[error("the default or placeholder of `{key}` is not valid for its kind")]
    Default { key: String },
    #[error("a group names `{id}`, which is not a contribution of the plugin")]
    UnknownContribution { id: ContributionId },
}

impl SettingsSchema {
    /// Checks the limits of the schema. `own` lists the plugin's contributions.
    pub fn validate(&self, own: &[ContributionId]) -> Result<(), SchemaError> {
        if self.groups.len() > MAX_GROUPS {
            return Err(SchemaError::TooMany {
                limit: SchemaLimit::Groups,
                max: MAX_GROUPS,
            });
        }
        if self.fields().count() > MAX_FIELDS {
            return Err(SchemaError::TooMany {
                limit: SchemaLimit::Fields,
                max: MAX_FIELDS,
            });
        }
        let mut keys = HashSet::new();
        for (index, group) in self.groups.iter().enumerate() {
            if !plain(&group.title, MAX_LABEL) {
                return Err(SchemaError::GroupTitle { index });
            }
            if let Some(id) = &group.contribution
                && !own.contains(id)
            {
                return Err(SchemaError::UnknownContribution { id: id.clone() });
            }
            for field in group.fields.iter() {
                check_field(field)?;
                if !keys.insert(field.key.as_ref()) {
                    return Err(SchemaError::DuplicateKey {
                        key: field.key.to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn check_field(field: &FieldSpec) -> Result<(), SchemaError> {
    let key = || field.key.to_string();
    if !key_syntax(&field.key) {
        return Err(SchemaError::Key { key: key() });
    }
    let label_ok = plain(&field.label, MAX_LABEL)
        && field
            .help
            .as_deref()
            .is_none_or(|help| plain(help, MAX_HELP));
    if !label_ok {
        return Err(SchemaError::Label { key: key() });
    }
    match &field.kind {
        FieldKind::Toggle { .. } => Ok(()),
        FieldKind::Choice { options, default } => {
            check_options(&field.key, options)?;
            if !has_option(options, default) {
                return Err(SchemaError::Default { key: key() });
            }
            Ok(())
        }
        FieldKind::MultiChoice {
            options,
            default,
            max,
            ..
        } => {
            check_options(&field.key, options)?;
            if *max == 0 || usize::from(*max) > options.len() {
                return Err(SchemaError::Range { key: key() });
            }
            let distinct = default.iter().collect::<HashSet<_>>().len() == default.len();
            let known = default.iter().all(|option| has_option(options, option));
            if !(distinct && known && default.len() <= usize::from(*max)) {
                return Err(SchemaError::Default { key: key() });
            }
            Ok(())
        }
        FieldKind::Number {
            min,
            max,
            step,
            default,
            unit,
        } => check_number(&field.key, (*min, *max, *step, *default), unit.as_deref()),
        FieldKind::Text {
            default,
            max_len,
            placeholder,
        } => {
            if !(1..=MAX_TEXT_LEN).contains(max_len) {
                return Err(SchemaError::Range { key: key() });
            }
            let default_ok = sanitize_text(default) == *default
                && default.chars().count() <= usize::from(*max_len);
            let placeholder_ok = placeholder.as_deref().is_none_or(|text| {
                sanitize_text(text) == text && text.chars().count() <= usize::from(MAX_TEXT_LEN)
            });
            if default_ok && placeholder_ok {
                Ok(())
            } else {
                Err(SchemaError::Default { key: key() })
            }
        }
    }
}

fn check_number(
    key: &str,
    (min, max, step, default): (i32, i32, i32, i32),
    unit: Option<&str>,
) -> Result<(), SchemaError> {
    if min > max || step <= 0 {
        return Err(SchemaError::Range {
            key: key.to_string(),
        });
    }
    let aligned = (i64::from(default) - i64::from(min)) % i64::from(step) == 0;
    if !(min..=max).contains(&default) || !aligned {
        return Err(SchemaError::Default {
            key: key.to_string(),
        });
    }
    if unit.is_some_and(|unit| !plain(unit, MAX_LABEL)) {
        return Err(SchemaError::Label {
            key: key.to_string(),
        });
    }
    Ok(())
}

fn check_options(key: &str, options: &[ChoiceOption]) -> Result<(), SchemaError> {
    if options.len() > MAX_OPTIONS {
        return Err(SchemaError::TooMany {
            limit: SchemaLimit::Options,
            max: MAX_OPTIONS,
        });
    }
    let malformed = options.is_empty()
        || options.iter().any(|option| !key_syntax(&option.key))
        || options
            .iter()
            .map(|option| option.key.as_ref())
            .collect::<HashSet<_>>()
            .len()
            != options.len();
    if malformed {
        return Err(SchemaError::Options {
            key: key.to_string(),
        });
    }
    let labels_ok = options.iter().all(|option| {
        plain(&option.label, MAX_LABEL)
            && option
                .help
                .as_deref()
                .is_none_or(|help| plain(help, MAX_HELP))
    });
    if labels_ok {
        Ok(())
    } else {
        Err(SchemaError::Label {
            key: key.to_string(),
        })
    }
}

fn has_option(options: &[ChoiceOption], key: &str) -> bool {
    options.iter().any(|option| option.key == key)
}

/// `[a-z][a-z0-9_]{0,31}`.
fn key_syntax(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && key.len() <= MAX_KEY
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Non-empty text of at most `max` characters that sanitizing would not change.
fn plain(text: &str, max: usize) -> bool {
    !text.is_empty() && text.chars().count() <= max && sanitize_text(text) == text
}
