//! Errors the host reports to a plugin, and the limits of plugin data.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why the host did not give a plugin what it asked for.
///
/// `Faulted` is a host state and never reaches a guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "kebab-case")]
pub enum HostError {
    /// The plugin never declared the capability, or the user did not grant it for this package.
    #[error("not granted")]
    NotGranted,
    /// The host refused this use of the capability.
    #[error("denied")]
    Denied,
    /// The capability was used too often.
    #[error("rate limited, retry after {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u32 },
    /// The data is not available now, or the optional capability was not granted.
    #[error("unavailable")]
    Unavailable,
}

/// A value of plugin data outside its limits.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LimitError {
    #[error("`{field}` is longer than {max} characters")]
    TooLong { field: &'static str, max: usize },
    #[error("`{field}` is empty")]
    Empty { field: &'static str },
    #[error("`{field}` does not follow its syntax")]
    Syntax { field: &'static str },
    #[error("`{field}` must be at most {max}")]
    Range { field: &'static str, max: u32 },
    #[error("`{field}` has more than {max} entries")]
    TooMany { field: &'static str, max: usize },
    #[error("`{field}` holds text that sanitizing would change")]
    Unsanitized { field: &'static str },
}

/// Longest local or storage key kept in a message.
pub(crate) fn key_syntax(text: &str, max: usize, field: &'static str) -> Result<(), LimitError> {
    if text.is_empty() {
        return Err(LimitError::Empty { field });
    }
    if text.len() > max {
        return Err(LimitError::TooLong { field, max });
    }
    let plain = text
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._".contains(&byte));
    if plain {
        Ok(())
    } else {
        Err(LimitError::Syntax { field })
    }
}
