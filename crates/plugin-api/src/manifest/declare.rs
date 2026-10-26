//! The capabilities a package declares, resolved against the table.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use thiserror::Error;

use super::MAX_REASON;
use crate::{
    CapabilityEntry,
    capability::{Family, capability},
    sanitize_display,
};

/// Most capabilities a package may list, required and optional together.
pub const MAX_DECLARED: usize = 16;

/// An optional capability: one this API knows, or an id from a newer API.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionalCapability {
    Known(&'static CapabilityEntry),
    /// Never granted; every use is `unavailable`.
    Unknown(String),
}

impl OptionalCapability {
    pub fn id(&self) -> &str {
        match self {
            Self::Known(entry) => entry.id,
            Self::Unknown(id) => id,
        }
    }
}

/// A capability list the host refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DeclareError {
    #[error("`{0}` is not a capability of this API; the package may need a newer Nocterm")]
    UnknownRequired(String),
    #[error("`{0}` is declared twice, or as both required and optional")]
    Duplicate(String),
    #[error("a package declares at most {MAX_DECLARED} capabilities")]
    TooMany,
    #[error("`{0}` is not a kebab-case `domain.name` id")]
    Syntax(String),
    #[error("a reason names `{0}`, which the package does not declare")]
    ReasonFor(String),
    #[error("`{0}` acts on the host, and a declarative package has no code that could use it")]
    EffectInDeclarative(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Declaration {
    pub(super) required: Vec<&'static CapabilityEntry>,
    pub(super) optional: Vec<OptionalCapability>,
    pub(super) reasons: BTreeMap<String, String>,
}

impl Declaration {
    pub(super) fn resolve(
        required: &[String],
        optional: &[String],
        reasons: &BTreeMap<String, String>,
    ) -> Result<Self, DeclareError> {
        if required.len() + optional.len() > MAX_DECLARED {
            return Err(DeclareError::TooMany);
        }
        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(required.len());
        for id in required {
            check_syntax(id)?;
            if !seen.insert(id.as_str()) {
                return Err(DeclareError::Duplicate(id.clone()));
            }
            entries.push(capability(id).ok_or_else(|| DeclareError::UnknownRequired(id.clone()))?);
        }
        let mut optionals = Vec::with_capacity(optional.len());
        for id in optional {
            check_syntax(id)?;
            if !seen.insert(id.as_str()) {
                return Err(DeclareError::Duplicate(id.clone()));
            }
            optionals.push(match capability(id) {
                Some(entry) => OptionalCapability::Known(entry),
                None => OptionalCapability::Unknown(id.clone()),
            });
        }
        let mut kept = BTreeMap::new();
        for (id, reason) in reasons {
            if !seen.contains(id.as_str()) {
                return Err(DeclareError::ReasonFor(id.clone()));
            }
            kept.insert(id.clone(), sanitize_display(reason, MAX_REASON));
        }
        Ok(Self {
            required: entries,
            optional: optionals,
            reasons: kept,
        })
    }

    /// Refuses effect capabilities. The declarative runtime produces no effect, so asking for
    /// one would only put an unusable permission in front of the user at consent.
    pub(super) fn observation_only(&self) -> Result<(), DeclareError> {
        let optional = self
            .optional
            .iter()
            .filter_map(|capability| match capability {
                OptionalCapability::Known(entry) => Some(*entry),
                OptionalCapability::Unknown(_) => None,
            });
        match self
            .required
            .iter()
            .copied()
            .chain(optional)
            .find(|entry| entry.family == Family::Effect)
        {
            Some(entry) => Err(DeclareError::EffectInDeclarative(entry.id.to_string())),
            None => Ok(()),
        }
    }

    /// The observing capability ids a template may name: required and known optional ones.
    pub(super) fn observing(&self) -> Vec<&'static str> {
        let optional = self
            .optional
            .iter()
            .filter_map(|capability| match capability {
                OptionalCapability::Known(entry) => Some(*entry),
                OptionalCapability::Unknown(_) => None,
            });
        self.required
            .iter()
            .copied()
            .chain(optional)
            .map(|entry| entry.id)
            .collect()
    }
}

fn check_syntax(id: &str) -> Result<(), DeclareError> {
    let ok = id.split_once('.').is_some_and(|(domain, name)| {
        [domain, name].iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
    });
    if ok && id.len() <= 48 {
        Ok(())
    } else {
        Err(DeclareError::Syntax(
            id.chars().take(48).collect::<String>().to_string(),
        ))
    }
}
