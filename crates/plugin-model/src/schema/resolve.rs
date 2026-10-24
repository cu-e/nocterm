//! Stored settings of a user plugin, brought up to date with the plugin's current schema.

use alloc::{string::String, string::ToString, vec::Vec};
use thiserror::Error;

use super::{FieldEdit, FieldError, SettingValues, SettingsSchema, apply};

/// A stored value the schema could not keep. The field shows its default instead.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FieldIssue {
    #[error("The saved value of `{key}` has the wrong kind, so the default is used.")]
    WrongKind { key: String },
    #[error("The saved value of `{key}` names `{option}`, which no longer exists.")]
    RemovedOption { key: String, option: String },
    #[error("The saved value of `{key}` is out of bounds, so the default is used.")]
    Invalid { key: String },
}

/// The values of a user plugin's fields, from what its data document stored.
///
/// - A value stored under a field's `renamed_from` key moves to the field's key, unless the
///   document already holds a value under the new key.
/// - Keys the schema does not declare are dropped.
/// - A value of the wrong kind, with a removed option or out of bounds falls back to the field's
///   default and is reported as an issue.
/// - A field the document does not hold takes its default.
///
/// Every value passes through [`apply`], so text is sanitized and bounded as an edit would be.
pub fn resolve(schema: &SettingsSchema, raw: &SettingValues) -> (SettingValues, Vec<FieldIssue>) {
    let mut values = SettingValues::default();
    let mut issues = Vec::new();
    for field in schema.fields() {
        let stored = raw.get(&field.key).or_else(|| {
            field
                .renamed_from
                .as_deref()
                .and_then(|previous| raw.get(previous))
        });
        let value = match stored {
            None => field.kind.default_value(),
            Some(stored) => match apply(
                schema,
                &SettingValues::default(),
                &field.key,
                FieldEdit::Set(stored.clone()),
            ) {
                Ok(value) => value,
                Err(error) => {
                    issues.push(issue(error, &field.key));
                    field.kind.default_value()
                }
            },
        };
        values.insert(field.key.to_string(), value);
    }
    (values, issues)
}

fn issue(error: FieldError, key: &str) -> FieldIssue {
    match error {
        FieldError::UnknownOption { option, .. } => FieldIssue::RemovedOption {
            key: key.to_string(),
            option,
        },
        FieldError::WrongKind { .. } | FieldError::UnknownKey(_) => FieldIssue::WrongKind {
            key: key.to_string(),
        },
        FieldError::OutOfRange { .. } | FieldError::TooMany { .. } | FieldError::TooLong { .. } => {
            FieldIssue::Invalid {
                key: key.to_string(),
            }
        }
    }
}
