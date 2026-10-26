//! Ids of plugins, contributions and panel groups.
//!
//! Each id wraps a `Cow<'static, str>` so owners declare them in `const` items with
//! [`PluginId::new`] and friends. Construction does not check syntax, because `const`
//! construction cannot report errors: the catalog checks every id it admits.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::{String, ToString};
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::IdError;

/// Longest plugin id, and longest namespace of a contribution id.
const MAX_LEN: usize = 32;

/// The prefix every user plugin id starts with. System plugin ids and the namespaces of core
/// contributions must not use it.
pub const USER_PREFIX: &str = "user-";

/// The id of a plugin: `[a-z0-9-]+`, at most 32 characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(Cow<'static, str>);

impl PluginId {
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// An id built at run time. It is not checked until [`check`](Self::check).
    pub fn owned(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }

    /// The id of a user plugin, `user-<publisher>-<name>`, built by the host from the manifest.
    ///
    /// `publisher` and `name` are each `[a-z0-9]+`, and the whole id is at most 32 characters.
    pub fn user(publisher: &str, name: &str) -> Result<Self, IdError> {
        let id = Self::owned(format!("{USER_PREFIX}{publisher}-{name}"));
        for part in [publisher, name] {
            if part.is_empty() {
                return Err(IdError::Empty {
                    id: id.0.to_string(),
                });
            }
            if !part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                return Err(IdError::Syntax {
                    id: id.0.to_string(),
                });
            }
        }
        id.check()?;
        Ok(id)
    }

    /// Whether the id is in the user plugin space.
    pub fn is_user(&self) -> bool {
        self.0.starts_with(USER_PREFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the id is well formed.
    pub fn check(&self) -> Result<(), IdError> {
        segment(&self.0, MAX_LEN)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The id of a contribution: `<plugin id>.<name>`, each part `[a-z0-9-]+`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContributionId(Cow<'static, str>);

impl ContributionId {
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// An id built at run time. It is not checked until [`check`](Self::check).
    pub fn owned(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }

    /// The id `<plugin>.<local>`, checked.
    pub fn in_plugin(plugin: &PluginId, local: &str) -> Result<Self, IdError> {
        let id = Self::owned(format!("{plugin}.{local}"));
        id.check()?;
        Ok(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the dot, which names the owner. The whole id when it has no dot.
    pub fn namespace(&self) -> &str {
        self.0
            .split_once('.')
            .map_or(&self.0, |(namespace, _)| namespace)
    }

    /// Whether the id is well formed.
    pub fn check(&self) -> Result<(), IdError> {
        let Some((namespace, name)) = self.0.split_once('.') else {
            return Err(IdError::Syntax {
                id: self.0.to_string(),
            });
        };
        segment(namespace, MAX_LEN).map_err(|error| error.within(&self.0))?;
        segment(name, MAX_LEN).map_err(|error| error.within(&self.0))
    }
}

impl fmt::Display for ContributionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The id of a panel group.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(Cow<'static, str>);

impl GroupId {
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// An id built at run time, such as a group the user made. It is not checked until
    /// [`check`](Self::check).
    pub fn owned(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the id is well formed.
    pub fn check(&self) -> Result<(), IdError> {
        segment(&self.0, MAX_LEN)
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn segment(text: &str, max: usize) -> Result<(), IdError> {
    if text.is_empty() {
        return Err(IdError::Empty {
            id: text.to_string(),
        });
    }
    if text.len() > max {
        return Err(IdError::TooLong {
            id: text.to_string(),
        });
    }
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(IdError::Syntax {
            id: text.to_string(),
        });
    }
    Ok(())
}
