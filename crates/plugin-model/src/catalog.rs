//! The checked set of every core contribution and system plugin of one application.

use alloc::collections::BTreeSet;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::{
    CatalogError, ContributionId, ContributionKind, GroupSpec, PluginId, SystemContribution,
    SystemPlugin,
};

/// Who offers a contribution.
#[derive(Clone, Copy, Debug)]
pub enum Owner<'a> {
    /// A function of the shell itself. It is not a plugin and is never disabled.
    Core,
    Plugin(&'a SystemPlugin),
}

#[derive(Clone, Copy, Debug)]
enum Source {
    Core,
    Plugin(usize),
}

#[derive(Debug)]
struct Entry {
    contribution: &'static SystemContribution,
    source: Source,
}

/// Panel groups, plugins and contributions that passed every check of [`PluginCatalog::new`].
///
/// Entries are kept in `(zone, order)` order.
#[derive(Debug)]
pub struct PluginCatalog {
    groups: Vec<GroupSpec>,
    plugins: Vec<&'static SystemPlugin>,
    entries: Vec<Entry>,
}

impl PluginCatalog {
    /// Checks and joins the groups, the core contributions and the plugins.
    ///
    /// Refuses: malformed or repeated ids; a plugin contribution outside its plugin's
    /// namespace; a core contribution inside any plugin's namespace; a group hint naming an
    /// undeclared group or a widget naming any group; panel traits on a widget or missing on
    /// a panel; a settings schema outside its limits, or one with fields and no settings
    /// section; a set of groups without exactly one primary group; and two contributions of one
    /// zone with the same order.
    pub fn new(
        groups: &[GroupSpec],
        core: &[&'static [SystemContribution]],
        plugins: &[&'static SystemPlugin],
    ) -> Result<Self, CatalogError> {
        let mut seen_groups = BTreeSet::new();
        for group in groups {
            group.id.check()?;
            if !seen_groups.insert(&group.id) {
                return Err(CatalogError::DuplicateGroup(group.id.clone()));
            }
        }
        let primaries = groups.iter().filter(|group| group.primary).count();
        if primaries != 1 {
            return Err(CatalogError::PrimaryGroups { count: primaries });
        }
        let mut seen_plugins = BTreeSet::new();
        for plugin in plugins {
            plugin.id.check()?;
            if !seen_plugins.insert(&plugin.id) {
                return Err(CatalogError::DuplicatePlugin(plugin.id.clone()));
            }
        }
        let mut entries = Vec::new();
        for contributions in core {
            for contribution in *contributions {
                if let Some(plugin) = plugins
                    .iter()
                    .find(|plugin| plugin.id.as_str() == contribution.id.namespace())
                {
                    return Err(CatalogError::CoreNamespace {
                        id: contribution.id.clone(),
                        plugin: plugin.id.clone(),
                    });
                }
                entries.push(Entry {
                    contribution,
                    source: Source::Core,
                });
            }
        }
        for (index, plugin) in plugins.iter().enumerate() {
            check_settings(plugin)?;
            for contribution in plugin.contributions {
                if contribution.id.namespace() != plugin.id.as_str() {
                    return Err(CatalogError::Namespace {
                        id: contribution.id.clone(),
                        expected: plugin.id.clone(),
                    });
                }
                entries.push(Entry {
                    contribution,
                    source: Source::Plugin(index),
                });
            }
        }
        check_entries(&entries, groups)?;
        entries.sort_by_key(|entry| {
            (
                entry.contribution.placement.zone,
                entry.contribution.placement.order,
            )
        });
        Ok(Self {
            groups: groups.to_vec(),
            plugins: plugins.to_vec(),
            entries,
        })
    }

    /// The contributions in `(zone, order)` order.
    pub fn entries(&self) -> impl Iterator<Item = &'static SystemContribution> + '_ {
        self.entries.iter().map(|entry| entry.contribution)
    }

    /// The plugins in declaration order.
    pub fn plugins(&self) -> impl Iterator<Item = &'static SystemPlugin> + '_ {
        self.plugins.iter().copied()
    }

    /// The declared panel groups, in declaration order.
    pub fn groups(&self) -> &[GroupSpec] {
        &self.groups
    }

    pub fn group(&self, id: &str) -> Option<&GroupSpec> {
        self.groups.iter().find(|group| group.id.as_str() == id)
    }

    pub fn contribution(&self, id: &ContributionId) -> Option<&'static SystemContribution> {
        self.entry(id).map(|entry| entry.contribution)
    }

    pub fn plugin(&self, id: &PluginId) -> Option<&'static SystemPlugin> {
        self.plugins().find(|plugin| plugin.id == *id)
    }

    /// The plugin that offers a contribution; `None` for core and unknown ids.
    pub fn plugin_of(&self, id: &ContributionId) -> Option<&'static SystemPlugin> {
        match self.owner(id)? {
            Owner::Core => None,
            Owner::Plugin(plugin) => Some(plugin),
        }
    }

    pub fn owner(&self, id: &ContributionId) -> Option<Owner<'static>> {
        Some(match self.entry(id)?.source {
            Source::Core => Owner::Core,
            Source::Plugin(index) => Owner::Plugin(self.plugins[index]),
        })
    }

    fn entry(&self, id: &ContributionId) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.contribution.id == *id)
    }
}

fn check_settings(plugin: &SystemPlugin) -> Result<(), CatalogError> {
    let own: Vec<ContributionId> = plugin
        .contributions
        .iter()
        .map(|contribution| contribution.id.clone())
        .collect();
    plugin
        .settings
        .validate(&own)
        .map_err(|source| CatalogError::Schema {
            plugin: plugin.id.clone(),
            source,
        })?;
    if plugin.settings_section.is_none() && !plugin.settings.is_empty() {
        return Err(CatalogError::SettingsSection {
            plugin: plugin.id.clone(),
        });
    }
    Ok(())
}

fn check_entries(entries: &[Entry], groups: &[GroupSpec]) -> Result<(), CatalogError> {
    let mut ids = BTreeSet::new();
    let mut orders = BTreeSet::new();
    for Entry { contribution, .. } in entries {
        contribution.id.check()?;
        if !ids.insert(&contribution.id) {
            return Err(CatalogError::Duplicate(contribution.id.clone()));
        }
        let is_panel = contribution.kind == ContributionKind::Panel;
        if let Some(group) = contribution.placement.group {
            if !is_panel {
                return Err(CatalogError::WidgetGroup {
                    id: contribution.id.clone(),
                    group: group.to_string(),
                });
            }
            if !groups.iter().any(|spec| spec.id.as_str() == group) {
                return Err(CatalogError::UnknownGroup {
                    id: contribution.id.clone(),
                    group: group.to_string(),
                });
            }
        }
        if is_panel != contribution.panel.is_some() {
            return Err(CatalogError::PanelTraits {
                id: contribution.id.clone(),
            });
        }
        let placement = &contribution.placement;
        if !orders.insert((placement.zone, placement.order)) {
            return Err(CatalogError::DuplicateOrder {
                zone: placement.zone,
                order: placement.order,
            });
        }
    }
    Ok(())
}
