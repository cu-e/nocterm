//! The user tier: plugins installed from a package, and the clamp that bounds what they declare.
//!
//! A user plugin is plain owned data built by the host after it validated a package. Its
//! contribution descriptor is a separate type from [`SystemContribution`], and the only way to
//! build one is [`UserContribution::clamp`], so a manifest cannot express a built-in privilege.

use alloc::string::String;
use alloc::vec::Vec;
use thiserror::Error;

use crate::{
    AutoPlace, ContributionId, ContributionKind, Icon, IdError, PanelChrome, PanelTraits,
    Placement, PluginId, SchemaError, SettingsSchema, WidthClass, Zone, sanitize_text,
};

/// Longest name or title, in characters.
const MAX_NAME: usize = 48;
/// Longest description, in characters.
const MAX_DESCRIPTION: usize = 160;

/// Which kind of owner a plugin has. It decides privileges, storage and execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluginTier {
    /// Compiled in, with the authority of its feature crate.
    System,
    /// Installed from a package, sandboxed and capability-gated.
    User,
}

/// A user plugin or contribution the host refuses to build.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum UserPluginError {
    #[error("invalid id")]
    Id(#[from] IdError),
    #[error("`{id}` is not in the user plugin space, which starts with `user-`")]
    NotUserSpace { id: PluginId },
    #[error("contribution `{id}` must be named in the namespace `{expected}`")]
    Namespace {
        id: ContributionId,
        expected: PluginId,
    },
    #[error("contribution `{0}` is declared twice")]
    Duplicate(ContributionId),
    #[error("the {field} is empty once unsafe characters are removed")]
    Empty { field: &'static str },
    #[error("the settings schema is invalid")]
    Schema(#[from] SchemaError),
}

/// A panel or widget of a user plugin, as the shell may place it.
///
/// Fields are private: the clamp is the only constructor. A user contribution always waits in
/// the tray until the user places it, has no hint into a built-in panel group, is not pinned,
/// draws no header of its own, cannot be expanded and is never required.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserContribution {
    id: ContributionId,
    kind: ContributionKind,
    title: String,
    icon: Icon,
    zone: Zone,
    order: u16,
}

impl UserContribution {
    /// Builds the descriptor from what a manifest asked for, dropping every privilege.
    ///
    /// The title is sanitized and cut to 48 characters, and an icon a user plugin may not show
    /// becomes the plug icon. `zone` and `order` only place a widget or panel toggle when the user
    /// puts it on the bar; they never place it by themselves.
    pub fn clamp(
        id: ContributionId,
        kind: ContributionKind,
        title: &str,
        icon: Icon,
        zone: Zone,
        order: u16,
    ) -> Result<Self, UserPluginError> {
        id.check()?;
        if !id.namespace().starts_with(crate::USER_PREFIX) {
            return Err(UserPluginError::NotUserSpace {
                id: PluginId::owned(id.namespace()),
            });
        }
        Ok(Self {
            id,
            kind,
            title: shortened(title, MAX_NAME, "title")?,
            icon: if icon.user_allowed() {
                icon
            } else {
                Icon::Plug
            },
            zone,
            order,
        })
    }

    pub fn id(&self) -> &ContributionId {
        &self.id
    }

    pub fn kind(&self) -> ContributionKind {
        self.kind
    }

    /// The sanitized name beside the toggle.
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn icon(&self) -> Icon {
        self.icon
    }

    /// Where the user's arrangement may place it: always the tray by default, never in a
    /// built-in group.
    pub fn placement(&self) -> Placement {
        Placement {
            zone: self.zone,
            order: self.order,
            group: None,
            auto: AutoPlace::Tray,
        }
    }

    /// How a panel is framed. User panels never draw their own header.
    pub fn chrome(&self) -> Option<PanelChrome> {
        (self.kind == ContributionKind::Panel).then_some(PanelChrome::Titled)
    }

    /// What a user panel needs from its column: the navigation width and a shell-drawn title.
    pub fn panel(&self) -> Option<PanelTraits> {
        (self.kind == ContributionKind::Panel).then_some(PanelTraits {
            width: WidthClass::Navigation,
            chrome: PanelChrome::Titled,
        })
    }

    /// User contributions are never pinned.
    pub fn pinned(&self) -> bool {
        false
    }

    /// User panels are never expandable.
    pub fn expandable(&self) -> bool {
        false
    }
}

/// A plugin installed from a package: its id, text, contributions and settings schema.
///
/// The enabled flag, grants and data live in the host's records, never here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserPlugin {
    id: PluginId,
    name: String,
    description: String,
    contributions: Vec<UserContribution>,
    settings: SettingsSchema,
}

impl UserPlugin {
    /// Checks and joins a plugin.
    ///
    /// Refuses an id outside the `user-` space, a contribution outside the plugin's namespace or
    /// declared twice, an empty name, and a settings schema that breaks its limits. Name and
    /// description are sanitized.
    pub fn new(
        id: PluginId,
        name: &str,
        description: &str,
        contributions: Vec<UserContribution>,
        settings: SettingsSchema,
    ) -> Result<Self, UserPluginError> {
        id.check()?;
        if !id.is_user() {
            return Err(UserPluginError::NotUserSpace { id });
        }
        let mut own: Vec<ContributionId> = Vec::with_capacity(contributions.len());
        for contribution in &contributions {
            if contribution.id.namespace() != id.as_str() {
                return Err(UserPluginError::Namespace {
                    id: contribution.id.clone(),
                    expected: id,
                });
            }
            if own.contains(&contribution.id) {
                return Err(UserPluginError::Duplicate(contribution.id.clone()));
            }
            own.push(contribution.id.clone());
        }
        settings.validate(&own)?;
        Ok(Self {
            name: shortened(name, MAX_NAME, "name")?,
            description: sanitize_text(description)
                .chars()
                .take(MAX_DESCRIPTION)
                .collect(),
            id,
            contributions,
            settings,
        })
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn contributions(&self) -> &[UserContribution] {
        &self.contributions
    }

    pub fn settings(&self) -> &SettingsSchema {
        &self.settings
    }
}

/// What menus, the palette and toggles call a user plugin's contribution: the plugin's name
/// before the title, so it cannot pass for one of the shell's own.
pub fn user_label(plugin: &str, title: &str) -> String {
    alloc::format!("Plugin \u{b7} {plugin} \u{2014} {title}")
}

/// [`user_label`] in a developer session: the name also ends in "DEV", so a contribution that
/// is not installed cannot pass for one that is.
pub fn developer_label(plugin: &str, title: &str) -> String {
    alloc::format!("{} \u{b7} DEV", user_label(plugin, title))
}

fn shortened(text: &str, max: usize, field: &'static str) -> Result<String, UserPluginError> {
    let text: String = sanitize_text(text).trim().chars().take(max).collect();
    if text.is_empty() {
        Err(UserPluginError::Empty { field })
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests;
