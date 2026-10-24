//! Pure edits of setting values against a schema.

use thiserror::Error;

use super::{
    ChoiceOption, FieldEdit, FieldKind, SettingValue, SettingValues, SettingsSchema, sanitize_text,
};

/// An edit the schema refuses. The text is meant to appear under the field.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FieldError {
    #[error("There is no setting `{0}`.")]
    UnknownKey(String),
    #[error("This setting does not take that kind of value.")]
    WrongKind { key: String },
    #[error("`{option}` is not one of the options.")]
    UnknownOption { key: String, option: String },
    #[error("Enter a whole number from {min} to {max}.")]
    OutOfRange { key: String, min: i32, max: i32 },
    #[error("Choose at most {max}.")]
    TooMany { key: String, max: u8 },
    #[error("Use at most {max} characters.")]
    TooLong { key: String, max: u16 },
}

/// The new value of field `key` after `edit`, computed from `current`.
///
/// `current` is the value at write time, so two edits in a row both land.
pub fn apply(
    schema: &SettingsSchema,
    current: &SettingValues,
    key: &str,
    edit: FieldEdit,
) -> Result<SettingValue, FieldError> {
    let field = schema
        .field(key)
        .ok_or_else(|| FieldError::UnknownKey(key.to_string()))?;
    let wrong_kind = || FieldError::WrongKind {
        key: key.to_string(),
    };
    match (&field.kind, edit) {
        (FieldKind::Toggle { .. }, FieldEdit::Set(value @ SettingValue::Bool(_))) => Ok(value),
        (FieldKind::Choice { options, .. }, FieldEdit::Set(SettingValue::Choice(choice))) => {
            check_option(key, options, &choice)?;
            Ok(SettingValue::Choice(choice))
        }
        (
            FieldKind::MultiChoice { options, max, .. },
            FieldEdit::Set(SettingValue::Choices(chosen)),
        ) => {
            let mut distinct: Vec<String> = Vec::with_capacity(chosen.len());
            for option in chosen {
                check_option(key, options, &option)?;
                if !distinct.contains(&option) {
                    distinct.push(option);
                }
            }
            check_count(key, distinct.len(), *max)?;
            Ok(SettingValue::Choices(distinct))
        }
        (
            FieldKind::MultiChoice {
                options,
                ordered,
                max,
                ..
            },
            FieldEdit::Toggle { option, on },
        ) => {
            let key = field.key.as_ref();
            check_option(key, options, &option)?;
            let base = field
                .current(current)
                .as_choices()
                .map(<[String]>::to_vec)
                .unwrap_or_default();
            if base.contains(&option) == on {
                return Ok(SettingValue::Choices(base));
            }
            let next = toggle_choice(options, &base, &option, on, *ordered);
            check_count(key, next.len(), *max)?;
            Ok(SettingValue::Choices(next))
        }
        (FieldKind::Number { min, max, step, .. }, FieldEdit::Step { up }) => {
            let base = field.current(current).as_number().unwrap_or(*min);
            Ok(SettingValue::Number(stepped(base, up, (*min, *max, *step))))
        }
        (FieldKind::Number { min, max, .. }, FieldEdit::Set(SettingValue::Number(number))) => {
            if (*min..=*max).contains(&number) {
                Ok(SettingValue::Number(number))
            } else {
                Err(FieldError::OutOfRange {
                    key: key.to_string(),
                    min: *min,
                    max: *max,
                })
            }
        }
        (FieldKind::Text { max_len, .. }, FieldEdit::Set(SettingValue::Text(raw))) => {
            let text = sanitize_text(&raw);
            if text.chars().count() > usize::from(*max_len) {
                return Err(FieldError::TooLong {
                    key: key.to_string(),
                    max: *max_len,
                });
            }
            Ok(SettingValue::Text(text))
        }
        _ => Err(wrong_kind()),
    }
}

/// `base` moved one `step` up or down, held within `(min, max, step)`.
fn stepped(base: i32, up: bool, (min, max, step): (i32, i32, i32)) -> i32 {
    let next = if up {
        base.saturating_add(step)
    } else {
        base.saturating_sub(step)
    };
    next.clamp(min, max)
}

/// `values` as a stored document may hold them: text is sanitized and cut to its field's length.
///
/// A system plugin's owner reads its section with serde, which knows nothing of the schema, so the
/// values it reports can be hand-edited. Run them through this before showing them.
pub fn bounded(schema: &SettingsSchema, values: SettingValues) -> SettingValues {
    let mut clean = SettingValues::default();
    for (key, value) in values.iter() {
        let value = match (schema.field(key).map(|field| &field.kind), value) {
            (Some(FieldKind::Text { max_len, .. }), SettingValue::Text(text)) => {
                let text = sanitize_text(text);
                SettingValue::Text(text.chars().take(usize::from(*max_len)).collect())
            }
            _ => value.clone(),
        };
        clean.insert(key, value);
    }
    clean
}

/// `current` with `option` turned on or off.
///
/// Off removes the option. On inserts it at its rank in `options` when `ordered` is true and
/// `current` is already in that rank order, and appends it otherwise, so a list the user
/// ordered by hand keeps its order.
pub fn toggle_choice(
    options: &[ChoiceOption],
    current: &[String],
    option: &str,
    on: bool,
    ordered: bool,
) -> Vec<String> {
    let mut list: Vec<String> = current
        .iter()
        .filter(|chosen| chosen.as_str() != option)
        .cloned()
        .collect();
    if !on {
        return list;
    }
    let rank = |key: &str| options.iter().position(|candidate| candidate.key == key);
    if ordered && list.is_sorted_by_key(|chosen| rank(chosen)) {
        let own = rank(option);
        let at = list.partition_point(|chosen| rank(chosen) < own);
        list.insert(at, option.to_string());
    } else {
        list.push(option.to_string());
    }
    list
}

fn check_option(key: &str, options: &[ChoiceOption], option: &str) -> Result<(), FieldError> {
    if options.iter().any(|candidate| candidate.key == option) {
        Ok(())
    } else {
        Err(FieldError::UnknownOption {
            key: key.to_string(),
            option: option.to_string(),
        })
    }
}

fn check_count(key: &str, count: usize, max: u8) -> Result<(), FieldError> {
    if count > usize::from(max) {
        Err(FieldError::TooMany {
            key: key.to_string(),
            max,
        })
    } else {
        Ok(())
    }
}
