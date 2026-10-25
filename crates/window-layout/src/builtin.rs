//! The layouts that ship with the application and are never stored.

use std::collections::BTreeSet;

use nocterm_plugin_model::{PluginId, PluginSet};

use crate::{Bar, LayoutPreset, PresetId, ResolvedLayout, resolve};

/// A built-in layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltIn {
    /// Every contribution where its hints put it.
    Default,
    /// The default with every group on the other side of the content; the status bar is the
    /// same.
    Swapped,
}

impl BuiltIn {
    pub const ALL: [Self; 2] = [Self::Default, Self::Swapped];

    pub const fn id(self) -> PresetId {
        match self {
            Self::Default => PresetId::new("default"),
            Self::Swapped => PresetId::new("swapped"),
        }
    }

    /// The name the user sees.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Swapped => "Swapped sides",
        }
    }

    /// The built-in with this id.
    pub fn of(id: &PresetId) -> Option<Self> {
        Self::ALL.into_iter().find(|built_in| built_in.id() == *id)
    }

    /// The default's preset: empty, so everything comes from the hints. The swapped layout is a
    /// resolution of it, not a preset of its own.
    pub fn preset() -> LayoutPreset {
        LayoutPreset {
            id: Self::Default.id(),
            name: Self::Default.name().to_owned(),
            bar: Bar::default(),
            tray: Vec::new(),
            open: Vec::new(),
        }
    }

    /// The arrangement of this built-in against the catalog, without the disabled plugins.
    pub fn resolve(self, catalog: &PluginSet, disabled: &BTreeSet<PluginId>) -> ResolvedLayout {
        let default = resolve(&Self::preset(), catalog, disabled);
        match self {
            Self::Default => default,
            Self::Swapped => default.swapped(),
        }
    }
}
