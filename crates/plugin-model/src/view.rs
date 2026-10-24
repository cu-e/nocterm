//! Borrowed read views over both tiers.
//!
//! Readers that list or place plugins (the layout resolver, the editor, the Plugins page) take
//! these instead of a descriptor type, so they handle system and user entries alike without
//! being able to tell which privileges an entry may hold.

use crate::{
    ContributionId, ContributionKind, Icon, PanelTraits, Placement, PluginId, PluginTier,
    SettingsSchema, SystemContribution, SystemPlugin, UserContribution, UserPlugin,
};

/// A plugin of either tier.
#[derive(Clone, Copy, Debug)]
pub enum PluginRef<'a> {
    System(&'a SystemPlugin),
    User(&'a UserPlugin),
}

impl<'a> From<&'a SystemPlugin> for PluginRef<'a> {
    fn from(plugin: &'a SystemPlugin) -> Self {
        Self::System(plugin)
    }
}

impl<'a> From<&'a UserPlugin> for PluginRef<'a> {
    fn from(plugin: &'a UserPlugin) -> Self {
        Self::User(plugin)
    }
}

impl<'a> PluginRef<'a> {
    pub fn tier(self) -> PluginTier {
        match self {
            Self::System(_) => PluginTier::System,
            Self::User(_) => PluginTier::User,
        }
    }

    pub fn id(self) -> &'a PluginId {
        match self {
            Self::System(plugin) => &plugin.id,
            Self::User(plugin) => plugin.id(),
        }
    }

    pub fn name(self) -> &'a str {
        match self {
            Self::System(plugin) => plugin.name,
            Self::User(plugin) => plugin.name(),
        }
    }

    pub fn description(self) -> &'a str {
        match self {
            Self::System(plugin) => plugin.description,
            Self::User(plugin) => plugin.description(),
        }
    }

    /// Only system plugins can be required.
    pub fn required(self) -> bool {
        match self {
            Self::System(plugin) => plugin.required,
            Self::User(_) => false,
        }
    }

    pub fn settings(self) -> &'a SettingsSchema {
        match self {
            Self::System(plugin) => &plugin.settings,
            Self::User(plugin) => plugin.settings(),
        }
    }

    /// The plugin's contributions in declaration order.
    pub fn contributions(self) -> impl Iterator<Item = ContributionRef<'a>> {
        let (system, user) = match self {
            Self::System(plugin) => (plugin.contributions, &[][..]),
            Self::User(plugin) => (&[][..], plugin.contributions()),
        };
        system
            .iter()
            .map(ContributionRef::System)
            .chain(user.iter().map(ContributionRef::User))
    }
}

/// A contribution of either tier.
#[derive(Clone, Copy, Debug)]
pub enum ContributionRef<'a> {
    System(&'a SystemContribution),
    User(&'a UserContribution),
}

impl<'a> From<&'a SystemContribution> for ContributionRef<'a> {
    fn from(contribution: &'a SystemContribution) -> Self {
        Self::System(contribution)
    }
}

impl<'a> From<&'a UserContribution> for ContributionRef<'a> {
    fn from(contribution: &'a UserContribution) -> Self {
        Self::User(contribution)
    }
}

impl<'a> ContributionRef<'a> {
    pub fn tier(self) -> PluginTier {
        match self {
            Self::System(_) => PluginTier::System,
            Self::User(_) => PluginTier::User,
        }
    }

    pub fn id(self) -> &'a ContributionId {
        match self {
            Self::System(contribution) => &contribution.id,
            Self::User(contribution) => contribution.id(),
        }
    }

    pub fn kind(self) -> ContributionKind {
        match self {
            Self::System(contribution) => contribution.kind,
            Self::User(contribution) => contribution.kind(),
        }
    }

    pub fn title(self) -> &'a str {
        match self {
            Self::System(contribution) => contribution.title,
            Self::User(contribution) => contribution.title(),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::System(contribution) => contribution.icon,
            Self::User(contribution) => contribution.icon(),
        }
    }

    pub fn placement(self) -> Placement {
        match self {
            Self::System(contribution) => contribution.placement,
            Self::User(contribution) => contribution.placement(),
        }
    }

    pub fn pinned(self) -> bool {
        match self {
            Self::System(contribution) => contribution.pinned,
            Self::User(contribution) => contribution.pinned(),
        }
    }

    /// What a panel needs from its column; `None` for widgets.
    pub fn panel(self) -> Option<PanelTraits> {
        match self {
            Self::System(contribution) => contribution.panel,
            Self::User(contribution) => contribution.panel(),
        }
    }

    /// Sample text for editor previews.
    pub fn preview(self) -> Option<&'a str> {
        match self {
            Self::System(contribution) => contribution.preview,
            Self::User(_) => None,
        }
    }

    /// Whether a user plugin offers it.
    pub fn is_user(self) -> bool {
        matches!(self, Self::User(_))
    }
}
