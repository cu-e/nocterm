//! `notifications.own` and `notifications.post`.

use alloc::{string::String, string::ToString, vec::Vec};
use serde::{Deserialize, Serialize};

use super::{Capability, Fields, Value, ValueType, sealed};
use crate::{LimitError, error::key_syntax, sanitize_display};

/// Longest notice text.
pub const MAX_NOTICE_TEXT: usize = 200;
/// Most live notices one plugin has.
pub const MAX_LIVE_NOTICES: usize = 3;
const MAX_KEY: usize = 32;

/// How a notice a plugin posts is drawn. There is no error level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoticeTone {
    Info,
    Caution,
}

impl NoticeTone {
    /// Every tone, in declaration order.
    pub const ALL: [Self; 2] = [Self::Info, Self::Caution];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Caution => "caution",
        }
    }
}

/// A notice the plugin posted and the host still shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct OwnNotice {
    pub tone: NoticeTone,
    pub text: String,
}

impl Fields for OwnNotice {
    const FIELDS: &'static [(&'static str, ValueType)] =
        &[("text", ValueType::Text), ("tone", ValueType::Text)];

    fn field(&self, name: &str) -> Option<Value> {
        match name {
            "text" => Some(Value::Text(self.text.clone())),
            "tone" => Some(Value::Text(self.tone.name().to_string())),
            _ => None,
        }
    }
}

/// The plugin's own live notices, never the host's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OwnNotices(pub Vec<OwnNotice>);

impl OwnNotices {
    pub fn check(&self) -> Result<(), LimitError> {
        if self.0.len() > MAX_LIVE_NOTICES {
            return Err(LimitError::TooMany {
                field: "notices",
                max: MAX_LIVE_NOTICES,
            });
        }
        Ok(())
    }
}

/// Marker of `notifications.own`.
pub struct Own;

impl sealed::Sealed for Own {}

impl Capability for Own {
    const ID: &'static str = "notifications.own";
    type Value = OwnNotices;
}

/// A notice to post, with the key that replaces an earlier one of the same key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Post {
    pub tone: NoticeTone,
    pub text: String,
    #[serde(default)]
    pub notice_key: Option<String>,
}

impl Post {
    pub fn check(&self) -> Result<(), LimitError> {
        if self.text.is_empty() {
            return Err(LimitError::Empty { field: "text" });
        }
        if self.text.chars().count() > MAX_NOTICE_TEXT {
            return Err(LimitError::TooLong {
                field: "text",
                max: MAX_NOTICE_TEXT,
            });
        }
        if sanitize_display(&self.text, MAX_NOTICE_TEXT) != self.text {
            return Err(LimitError::Unsanitized { field: "text" });
        }
        self.notice_key
            .as_deref()
            .map_or(Ok(()), |key| key_syntax(key, MAX_KEY, "notice_key"))
    }
}

/// Marker of `notifications.post`.
pub struct PostNotice;

impl sealed::Sealed for PostNotice {}

impl Capability for PostNotice {
    const ID: &'static str = "notifications.post";
    type Value = Post;
}
