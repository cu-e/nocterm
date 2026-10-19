//! Feature-specific settings syntax and validation errors.

/// Why text typed into a field cannot be saved; `Display` is the message
/// shown beside the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsInputError {
    #[error("{name}: enter a number.")]
    NotANumber { name: String },
    #[error("{name}: enter a value from {start} to {end}.")]
    OutOfRange {
        name: String,
        start: String,
        end: String,
    },
    #[error("{name}: enter a JSON array of strings, e.g. [\"-l\"].")]
    NotStringList { name: String },
    #[error("{name}: enter a JSON object of string values.")]
    NotStringMap { name: String },
    #[error("{name}: names must be shell identifiers.")]
    InvalidEnvironmentName { name: String },
    #[error("{name}: values cannot contain NUL.")]
    ContainsNul { name: String },
    #[error("Executable cannot contain NUL.")]
    ExecutableContainsNul,
    #[error("Directory cannot contain NUL.")]
    DirectoryContainsNul,
}

/// An optional value: empty text means unset.
pub fn optional(text: &str) -> Option<String> {
    Some(text.trim().to_owned()).filter(|text| !text.is_empty())
}

/// A number within `range`, without silently clamping.
pub fn number<T>(
    text: &str,
    name: &str,
    range: std::ops::RangeInclusive<T>,
) -> Result<T, SettingsInputError>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display,
{
    let value = text
        .trim()
        .parse::<T>()
        .map_err(|_| SettingsInputError::NotANumber {
            name: name.to_owned(),
        })?;
    if !range.contains(&value) {
        return Err(SettingsInputError::OutOfRange {
            name: name.to_owned(),
            start: range.start().to_string(),
            end: range.end().to_string(),
        });
    }
    Ok(value)
}

/// An optional number: empty text means the theme's value.
pub fn optional_number<T>(
    text: &str,
    name: &str,
    range: std::ops::RangeInclusive<T>,
) -> Result<Option<T>, SettingsInputError>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display,
{
    if text.trim().is_empty() {
        Ok(None)
    } else {
        number(text, name, range).map(Some)
    }
}

/// A JSON array of strings, such as program arguments.
pub fn string_list(text: &str, name: &str) -> Result<Vec<String>, SettingsInputError> {
    let list: Vec<String> =
        serde_json::from_str(text).map_err(|_| SettingsInputError::NotStringList {
            name: name.to_owned(),
        })?;
    if list.iter().any(|value| value.contains('\0')) {
        return Err(SettingsInputError::ContainsNul {
            name: name.to_owned(),
        });
    }
    Ok(list)
}

/// A JSON object of environment variables with shell-identifier names.
pub fn environment(
    text: &str,
    name: &str,
) -> Result<std::collections::BTreeMap<String, String>, SettingsInputError> {
    let env: std::collections::BTreeMap<String, String> =
        serde_json::from_str(text).map_err(|_| SettingsInputError::NotStringMap {
            name: name.to_owned(),
        })?;
    if env.keys().any(|key| !is_identifier(key)) {
        return Err(SettingsInputError::InvalidEnvironmentName {
            name: name.to_owned(),
        });
    }
    if env.values().any(|value| value.contains('\0')) {
        return Err(SettingsInputError::ContainsNul {
            name: name.to_owned(),
        });
    }
    Ok(env)
}

fn is_identifier(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
}

/// Serializes a value shown as JSON text.
pub fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("settings values serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_checked_against_their_range_without_clamping() {
        assert_eq!(number("  12 ", "Size", 6..=72), Ok(12));
        assert_eq!(
            number("73", "Size", 6..=72).unwrap_err().to_string(),
            "Size: enter a value from 6 to 72."
        );
        assert_eq!(
            number::<u8>("x", "Size", 6..=72).unwrap_err().to_string(),
            "Size: enter a number."
        );
        assert!(number("NaN", "Size", 6.0..=72.0).is_err());
        assert_eq!(optional_number::<f32>(" ", "Size", 6.0..=72.0), Ok(None));
    }

    #[test]
    fn json_lists_and_environments_are_validated() {
        assert_eq!(
            string_list(r#"["-c","printf '%s' \"a b\""]"#, "Arguments").unwrap(),
            vec!["-c", "printf '%s' \"a b\""]
        );
        assert_eq!(
            string_list("-l", "Arguments").unwrap_err().to_string(),
            "Arguments: enter a JSON array of strings, e.g. [\"-l\"]."
        );
        assert_eq!(
            string_list(r#"["a\u0000"]"#, "Arguments")
                .unwrap_err()
                .to_string(),
            "Arguments: values cannot contain NUL."
        );
        assert!(environment(r#"{"PATH":"/bin"}"#, "Environment").is_ok());
        assert_eq!(
            environment(r#"{"BAD;NAME":"x"}"#, "Environment")
                .unwrap_err()
                .to_string(),
            "Environment: names must be shell identifiers."
        );
        assert!(environment(r#"{"1X":"x"}"#, "Environment").is_err());
    }
}
