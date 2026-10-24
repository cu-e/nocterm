//! What the user allowed one exact package to use.
//!
//! A grant is bound to `(PluginId, sha256)`: a package with a new hash has no grant. The
//! capabilities a package lists as required are granted all together at consent; optional ones
//! only when the user accepts them there. An optional capability that was not granted answers
//! `unavailable`, which is how a plugin detects the feature.

use alloc::{collections::BTreeSet, string::String, vec::Vec};

use nocterm_plugin_model::PluginId;
use thiserror::Error;

use crate::{ContentHash, HostError, Manifest, Package};

/// A grant the host refuses to make.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum GrantError {
    #[error("`{0}` is not an optional capability of the package")]
    NotOptional(String),
}

/// The capabilities the user allowed for one exact package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    plugin: PluginId,
    hash: ContentHash,
    required: BTreeSet<String>,
    declared_optional: BTreeSet<String>,
    granted_optional: BTreeSet<String>,
}

impl Grant {
    /// The grant of a consent to `package`: every required capability, and the optional ones the
    /// user accepted. Accepting an optional capability the package does not list is refused.
    ///
    /// The manifest and the hash come from one [`Package`], so a grant cannot join the
    /// capabilities of one version with the hash of another.
    pub fn consent(package: &Package, accepted: &[&str]) -> Result<Self, GrantError> {
        let manifest = package.manifest();
        let declared_optional: BTreeSet<String> =
            manifest.optional().iter().map(|c| c.id().into()).collect();
        let mut granted_optional = BTreeSet::new();
        for id in accepted {
            let known = manifest.optional().iter().any(|optional| {
                optional.id() == *id && matches!(optional, crate::OptionalCapability::Known(_))
            });
            if !known {
                return Err(GrantError::NotOptional((*id).into()));
            }
            granted_optional.insert((*id).into());
        }
        Ok(Self {
            plugin: manifest.plugin().id().clone(),
            hash: *package.hash(),
            required: manifest.required().iter().map(|e| e.id.into()).collect(),
            declared_optional,
            granted_optional,
        })
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn hash(&self) -> &ContentHash {
        &self.hash
    }

    /// Whether the grant was made for exactly this package.
    pub fn covers(&self, plugin: &PluginId, hash: &ContentHash) -> bool {
        self.plugin == *plugin && self.hash == *hash
    }

    /// Whether `capability` may be used by the package with `hash`.
    ///
    /// A different package, or a capability the package never declared, is `NotGranted`. An
    /// optional capability the user did not accept is `Unavailable`.
    pub fn check(
        &self,
        plugin: &PluginId,
        hash: &ContentHash,
        capability: &str,
    ) -> Result<(), HostError> {
        if !self.covers(plugin, hash) {
            return Err(HostError::NotGranted);
        }
        if self.required.contains(capability) || self.granted_optional.contains(capability) {
            Ok(())
        } else if self.declared_optional.contains(capability) {
            Err(HostError::Unavailable)
        } else {
            Err(HostError::NotGranted)
        }
    }

    /// Every capability the grant allows.
    pub fn granted(&self) -> impl Iterator<Item = &str> {
        self.required
            .iter()
            .chain(&self.granted_optional)
            .map(String::as_str)
    }
}

/// The capabilities and settings keys `next` adds over `previous`, for the update dialog to
/// highlight. An optional capability that `next` requires counts as added: the user may have
/// declined it before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Additions {
    pub capabilities: Vec<String>,
    pub settings: Vec<String>,
}

impl Additions {
    pub fn between(previous: &Manifest, next: &Manifest) -> Self {
        let was_required = |id: &str| previous.required().iter().any(|e| e.id == id);
        let was_listed =
            |id: &str| was_required(id) || previous.optional().iter().any(|c| c.id() == id);
        let required = next
            .required()
            .iter()
            .map(|entry| entry.id)
            .filter(|id| !was_required(id));
        let optional = next
            .optional()
            .iter()
            .map(|capability| capability.id())
            .filter(|id| !was_listed(id));
        let capabilities = required.chain(optional).map(Into::into).collect();
        let settings = next
            .plugin()
            .settings()
            .fields()
            .filter(|field| previous.plugin().settings().field(&field.key).is_none())
            .map(|field| field.key.as_ref().into())
            .collect();
        Self {
            capabilities,
            settings,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty() && self.settings.is_empty()
    }
}
