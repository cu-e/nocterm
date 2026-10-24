//! The plugins of one window session: the system catalog and the installed user plugins.
//!
//! A [`PluginSet`] is an immutable snapshot. The host that manages user plugins builds a new one
//! whenever the installed plugins, their enabled flags or their placement approval change, and
//! raises the revision, so a reader that holds an older snapshot can tell it is stale.

use alloc::{collections::BTreeSet, sync::Arc, vec::Vec};
use thiserror::Error;

use crate::{
    ContributionId, ContributionRef, GroupSpec, Owner, PluginCatalog, PluginId, PluginRef,
    UserPlugin,
};

/// A user plugin with the flags the host keeps for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserEntry {
    pub plugin: UserPlugin,
    /// Whether the user switched the plugin on. A disabled plugin keeps its placement.
    pub enabled: bool,
    /// Whether the user approved placing this exact package. Entries that are not placeable are
    /// skipped by placement and cannot be opened.
    pub placeable: bool,
}

/// A set of user plugins the shell refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PluginSetError {
    #[error("plugin `{0}` is installed twice")]
    Duplicate(PluginId),
}

/// Who offers a contribution, over both tiers.
#[derive(Clone, Copy, Debug)]
pub enum SetOwner<'a> {
    /// A function of the shell itself. It is not a plugin and is never disabled.
    Core,
    Plugin(PluginRef<'a>),
}

/// The system catalog, the installed user plugins and a revision.
#[derive(Clone, Debug)]
pub struct PluginSet {
    catalog: Arc<PluginCatalog>,
    users: Vec<UserEntry>,
    revision: u64,
}

impl PluginSet {
    /// A set of the system catalog alone, at revision 0.
    pub fn system(catalog: Arc<PluginCatalog>) -> Self {
        Self {
            catalog,
            users: Vec::new(),
            revision: 0,
        }
    }

    /// The same system catalog with `users` installed, at `revision`.
    pub fn with_users(&self, users: Vec<UserEntry>, revision: u64) -> Result<Self, PluginSetError> {
        let mut seen = BTreeSet::new();
        for entry in &users {
            if !seen.insert(entry.plugin.id()) {
                return Err(PluginSetError::Duplicate(entry.plugin.id().clone()));
            }
        }
        Ok(Self {
            catalog: self.catalog.clone(),
            users,
            revision,
        })
    }

    /// The checked system catalog.
    pub fn catalog(&self) -> &Arc<PluginCatalog> {
        &self.catalog
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The installed user plugins in installation order.
    pub fn users(&self) -> &[UserEntry] {
        &self.users
    }

    pub fn user(&self, id: &PluginId) -> Option<&UserEntry> {
        self.users.iter().find(|entry| entry.plugin.id() == id)
    }

    /// The declared panel groups; user plugins declare none.
    pub fn groups(&self) -> &[GroupSpec] {
        self.catalog.groups()
    }

    pub fn group(&self, id: &str) -> Option<&GroupSpec> {
        self.catalog.group(id)
    }

    /// Every contribution: the system ones in `(zone, order)` order, then those of each user
    /// plugin in installation and declaration order.
    pub fn entries(&self) -> impl Iterator<Item = ContributionRef<'_>> + '_ {
        let system = self
            .catalog
            .entries()
            .map(|contribution| -> ContributionRef<'_> { ContributionRef::System(contribution) });
        let user = self
            .users
            .iter()
            .flat_map(|entry| entry.plugin.contributions())
            .map(ContributionRef::User);
        system.chain(user)
    }

    /// The system plugins in declaration order, then the user plugins in installation order.
    pub fn plugins(&self) -> impl Iterator<Item = PluginRef<'_>> + '_ {
        let system = self
            .catalog
            .plugins()
            .map(|plugin| -> PluginRef<'_> { PluginRef::System(plugin) });
        let user = self
            .users
            .iter()
            .map(|entry| PluginRef::User(&entry.plugin));
        system.chain(user)
    }

    pub fn contribution(&self, id: &ContributionId) -> Option<ContributionRef<'_>> {
        if let Some(system) = self.catalog.contribution(id) {
            return Some(ContributionRef::System(system));
        }
        self.users
            .iter()
            .flat_map(|entry| entry.plugin.contributions())
            .find(|contribution| contribution.id() == id)
            .map(ContributionRef::User)
    }

    pub fn plugin(&self, id: &PluginId) -> Option<PluginRef<'_>> {
        if let Some(system) = self.catalog.plugin(id) {
            return Some(PluginRef::System(system));
        }
        self.user(id).map(|entry| PluginRef::User(&entry.plugin))
    }

    /// The plugin that offers a contribution; `None` for core and unknown ids.
    pub fn plugin_of(&self, id: &ContributionId) -> Option<PluginRef<'_>> {
        match self.owner(id)? {
            SetOwner::Core => None,
            SetOwner::Plugin(plugin) => Some(plugin),
        }
    }

    pub fn owner(&self, id: &ContributionId) -> Option<SetOwner<'_>> {
        if let Some(owner) = self.catalog.owner(id) {
            return Some(match owner {
                Owner::Core => SetOwner::Core,
                Owner::Plugin(plugin) => SetOwner::Plugin(PluginRef::System(plugin)),
            });
        }
        self.users
            .iter()
            .find(|entry| {
                entry
                    .plugin
                    .contributions()
                    .iter()
                    .any(|contribution| contribution.id() == id)
            })
            .map(|entry| SetOwner::Plugin(PluginRef::User(&entry.plugin)))
    }

    /// Whether the shell may place and open the contribution: always for the system tier, and
    /// for a user contribution once the user approved placing its package.
    pub fn is_placeable(&self, id: &ContributionId) -> bool {
        match self.owner(id) {
            Some(SetOwner::Plugin(PluginRef::User(plugin))) => {
                self.user(plugin.id()).is_some_and(|entry| entry.placeable)
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Adds the user plugins the user switched off to `disabled`.
    pub fn extend_disabled(&self, disabled: &mut BTreeSet<PluginId>) {
        disabled.extend(
            self.users
                .iter()
                .filter(|entry| !entry.enabled)
                .map(|entry| entry.plugin.id().clone()),
        );
    }
}

impl From<Arc<PluginCatalog>> for PluginSet {
    fn from(catalog: Arc<PluginCatalog>) -> Self {
        Self::system(catalog)
    }
}

impl From<PluginCatalog> for PluginSet {
    fn from(catalog: PluginCatalog) -> Self {
        Self::system(Arc::new(catalog))
    }
}

#[cfg(test)]
mod tests;
