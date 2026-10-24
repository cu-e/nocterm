//! Names of presets: what the user may call one.

use crate::{BuiltIn, LayoutSettings, PresetId};

/// Longest preset name, in characters.
pub const MAX_NAME_CHARS: usize = 48;

/// Why a preset name was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("the name is empty")]
    Empty,
    #[error("the name is longer than {MAX_NAME_CHARS} characters")]
    TooLong,
    #[error("\"{0}\" is the name of a built-in layout")]
    Reserved(String),
    #[error("another layout is already called \"{0}\"")]
    Duplicate(String),
}

/// Checks a name for a preset and returns it trimmed.
///
/// `renaming` is the preset the name is for, which may keep its own name; `None` checks a name
/// for a new preset. Names compare without regard to case.
pub fn validate_name(
    settings: &LayoutSettings,
    renaming: Option<&PresetId>,
    name: &str,
) -> Result<String, NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(NameError::TooLong);
    }
    if is_reserved(name) {
        return Err(NameError::Reserved(name.to_owned()));
    }
    let taken = settings
        .presets
        .iter()
        .filter(|preset| Some(&preset.id) != renaming)
        .any(|preset| same_name(&preset.name, name));
    if taken {
        return Err(NameError::Duplicate(name.to_owned()));
    }
    Ok(name.to_owned())
}

/// Whether the name belongs to a built-in layout.
pub(crate) fn is_reserved(name: &str) -> bool {
    BuiltIn::ALL
        .into_iter()
        .any(|built_in| same_name(built_in.name(), name))
}

pub(crate) fn same_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}
