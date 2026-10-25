//! A removal the user was asked to confirm.

use nocterm_plugin_model::PluginId;

/// What a removal takes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RemovalKind {
    /// The plugin; its data stays.
    Keep,
    /// The plugin and its data.
    WithData,
    /// The data; the plugin stays installed.
    DataOnly,
}

impl RemovalKind {
    /// The label of the button that asks, and of the one that confirms.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Keep => "Remove",
            Self::WithData => "Remove with its data",
            Self::DataOnly => "Remove its data",
        }
    }

    /// The question the confirm row asks about the plugin named `name`.
    pub(super) fn question(self, name: &str) -> String {
        match self {
            Self::Keep => format!("Remove {name}? Its data is kept."),
            Self::WithData => format!("Remove {name} and delete its data? This cannot be undone."),
            Self::DataOnly => {
                format!("Delete the data of {name}? The plugin stays. This cannot be undone.")
            }
        }
    }
}

/// A destructive click waiting for its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Removal {
    id: PluginId,
    kind: RemovalKind,
}

impl Removal {
    pub(super) fn new(id: PluginId, kind: RemovalKind) -> Self {
        Self { id, kind }
    }

    /// The plugin the question is about.
    pub(super) fn target(&self) -> &PluginId {
        &self.id
    }

    pub(super) fn kind(&self) -> RemovalKind {
        self.kind
    }

    pub(super) fn into_parts(self) -> (PluginId, RemovalKind) {
        (self.id, self.kind)
    }
}
