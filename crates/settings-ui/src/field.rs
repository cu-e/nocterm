//! A text setting that saves itself: shortly after typing stops, on Enter and
//! when it loses focus. Invalid text stays in the field with its error and
//! is never written.
use std::{rc::Rc, time::Duration};

/// How long typing must pause before a field saves.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(600);

use gpui_kit::{
    App, Context, Entity, Focusable as _, SharedString, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
    prelude::*,
};
use nocterm_settings::{SettingsDocument, SettingsSection};
use nocterm_store::SettingsHandle;

type Read = Rc<dyn Fn(&SettingsDocument) -> String>;
type Write = Rc<dyn Fn(&mut SettingsDocument, &str) -> Result<(), FieldError>>;

/// Why text typed into a field cannot be saved; `Display` is the message
/// shown beside the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FieldError {
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
    #[error("Enter an absolute path, or leave empty for the private workspace.")]
    InvalidWorkingDirectory,
    #[error("Inherited variables: enter a JSON array of names.")]
    InheritedNotNames,
    #[error("An agent named `{0}` already exists.")]
    DuplicateAgent(String),
    #[error(transparent)]
    Agent(#[from] crate::ai::AgentValidationError),
}

pub(crate) struct Field {
    settings: SettingsHandle,
    pub input: Entity<InputState>,
    read: Read,
    write: Write,
    pub error: Option<SharedString>,
    /// The text last saved or loaded; unchanged text is not written again.
    saved: String,
    /// A save waiting for typing to pause.
    pub debounce: Option<Task<()>>,
    _subscription: Subscription,
}

impl Field {
    /// A field showing `read` of section `S` and saving through `write`.
    pub(crate) fn new<S: SettingsSection, V: 'static>(
        settings: SettingsHandle,
        read: impl Fn(&S) -> String + 'static,
        write: impl Fn(&mut S, &str) -> Result<(), FieldError> + 'static,
        window: &mut Window,
        cx: &mut Context<V>,
        on_event: impl Fn(&mut V, &InputEvent, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        let read: Read = Rc::new(move |document| read(document.get::<S>()));
        let write: Write = Rc::new(move |document, text| {
            let mut section = document.get::<S>().clone();
            write(&mut section, text)?;
            document.set(section);
            Ok(())
        });
        let saved = read(settings.document(cx));
        let input = cx.new(|cx| InputState::new(window, cx).default_value(saved.clone()));
        let subscription = cx.subscribe_in(&input, window, move |view, _, event, window, cx| {
            on_event(view, event, window, cx)
        });
        Self {
            settings,
            input,
            read,
            write,
            error: None,
            saved,
            debounce: None,
            _subscription: subscription,
        }
    }

    /// Validates the text against the current settings and saves it.
    pub(crate) fn commit(&mut self, cx: &mut App) {
        self.debounce = None;
        let text = self.input.read(cx).value().to_string();
        if text == self.saved && self.error.is_none() {
            return;
        }
        let mut probe = self.settings.document(cx).clone();
        if let Err(error) = (self.write)(&mut probe, &text) {
            self.error = Some(error.to_string().into());
            return;
        }
        self.error = None;
        self.saved = text.clone();
        let write = self.write.clone();
        self.settings
            .edit(cx, move |settings| {
                // Checked above; the settings may have changed since, so a
                // value that no longer fits is left out rather than forced in.
                let _ = write(settings, &text);
            })
            .detach();
    }

    /// Shows a value saved elsewhere, unless the user is editing this field.
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut App) {
        let value = (self.read)(self.settings.document(cx));
        if value == self.saved || self.debounce.is_some() || self.error.is_some() {
            return;
        }
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        if focused && self.input.read(cx).value() != self.saved.as_str() {
            return;
        }
        self.saved = value.clone();
        self.input
            .update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

/// An optional value: empty text means unset.
pub(crate) fn optional(text: &str) -> Option<String> {
    Some(text.trim().to_owned()).filter(|text| !text.is_empty())
}

/// A number within `range`, without silently clamping.
pub(crate) fn number<T>(
    text: &str,
    name: &str,
    range: std::ops::RangeInclusive<T>,
) -> Result<T, FieldError>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display,
{
    let value = text
        .trim()
        .parse::<T>()
        .map_err(|_| FieldError::NotANumber {
            name: name.to_owned(),
        })?;
    if !range.contains(&value) {
        return Err(FieldError::OutOfRange {
            name: name.to_owned(),
            start: range.start().to_string(),
            end: range.end().to_string(),
        });
    }
    Ok(value)
}

/// An optional number: empty text means the theme's value.
pub(crate) fn optional_number<T>(
    text: &str,
    name: &str,
    range: std::ops::RangeInclusive<T>,
) -> Result<Option<T>, FieldError>
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
pub(crate) fn string_list(text: &str, name: &str) -> Result<Vec<String>, FieldError> {
    let list: Vec<String> = serde_json::from_str(text).map_err(|_| FieldError::NotStringList {
        name: name.to_owned(),
    })?;
    if list.iter().any(|value| value.contains('\0')) {
        return Err(FieldError::ContainsNul {
            name: name.to_owned(),
        });
    }
    Ok(list)
}

/// A JSON object of environment variables with shell-identifier names.
pub(crate) fn environment(
    text: &str,
    name: &str,
) -> Result<std::collections::BTreeMap<String, String>, FieldError> {
    let env: std::collections::BTreeMap<String, String> =
        serde_json::from_str(text).map_err(|_| FieldError::NotStringMap {
            name: name.to_owned(),
        })?;
    if env.keys().any(|key| !is_identifier(key)) {
        return Err(FieldError::InvalidEnvironmentName {
            name: name.to_owned(),
        });
    }
    if env.values().any(|value| value.contains('\0')) {
        return Err(FieldError::ContainsNul {
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
pub(crate) fn json<T: serde::Serialize>(value: &T) -> String {
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

    #[test]
    fn agent_field_errors_keep_their_messages() {
        assert_eq!(
            FieldError::DuplicateAgent("codex".into()).to_string(),
            "An agent named `codex` already exists."
        );
        assert_eq!(
            FieldError::from(crate::ai::AgentValidationError::InvalidId).to_string(),
            "Agent ids use letters, numbers, underscore or dash, up to 64 characters."
        );
    }
}
