//! Typed errors of id syntax and catalog validation.

use alloc::{string::String, string::ToString};
use thiserror::Error;

use crate::{ContributionId, GroupId, PluginId, SchemaError, Zone};

/// An id that does not follow the id syntax.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum IdError {
    #[error("`{id}` has an empty part")]
    Empty { id: String },
    #[error("`{id}` is not a lowercase letters, digits and hyphens id")]
    Syntax { id: String },
    #[error("`{id}` is longer than 32 characters")]
    TooLong { id: String },
}

impl IdError {
    /// The same refusal, reporting the whole id when only a part of it was checked.
    pub(crate) fn within(self, whole: &str) -> Self {
        match self {
            Self::Empty { .. } => Self::Empty {
                id: whole.to_string(),
            },
            Self::Syntax { .. } => Self::Syntax {
                id: whole.to_string(),
            },
            Self::TooLong { .. } => Self::TooLong {
                id: whole.to_string(),
            },
        }
    }
}

/// A set of plugins, core contributions and groups that the catalog refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CatalogError {
    #[error("plugin `{0}` is declared twice")]
    DuplicatePlugin(PluginId),
    #[error("contribution `{0}` is declared twice")]
    Duplicate(ContributionId),
    #[error("group `{0}` is declared twice")]
    DuplicateGroup(GroupId),
    #[error("invalid id")]
    InvalidId(#[from] IdError),
    #[error("contribution `{id}` of a plugin must be named in the namespace `{expected}`")]
    Namespace {
        id: ContributionId,
        expected: PluginId,
    },
    #[error("core contribution `{id}` is named in the namespace of plugin `{plugin}`")]
    CoreNamespace {
        id: ContributionId,
        plugin: PluginId,
    },
    #[error("contribution `{id}` names the undeclared group `{group}`")]
    UnknownGroup { id: ContributionId, group: String },
    #[error("widget `{id}` names the group `{group}`, and only panels belong to groups")]
    WidgetGroup { id: ContributionId, group: String },
    #[error("contribution `{id}` must have panel traits exactly when it is a panel")]
    PanelTraits { id: ContributionId },
    #[error("exactly one group must be primary, but {count} are")]
    PrimaryGroups { count: usize },
    #[error("the settings schema of plugin `{plugin}` is invalid")]
    Schema {
        plugin: PluginId,
        #[source]
        source: SchemaError,
    },
    #[error("plugin `{plugin}` declares settings fields but names no settings section")]
    SettingsSection { plugin: PluginId },
    #[error("two contributions of the {zone:?} zone share the order {order}")]
    DuplicateOrder { zone: Zone, order: u16 },
}
