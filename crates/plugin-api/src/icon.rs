//! The icons a plugin view may draw.

use nocterm_plugin_model::Icon;
use serde::{Deserialize, Serialize};

/// An icon a plugin names. Names are frozen and the list is append-only.
///
/// Each is an icon that no system plugin or core contribution draws, so a plugin toggle cannot
/// imitate a host function (`Icon::user_allowed`; the app checks the catalog against this set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PluginIcon {
    Box,
    Gem,
    Hexagon,
    Plug,
    Puzzle,
    Shapes,
}

impl PluginIcon {
    /// Every icon, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Box,
        Self::Gem,
        Self::Hexagon,
        Self::Plug,
        Self::Puzzle,
        Self::Shapes,
    ];

    /// The frozen kebab-case name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Gem => "gem",
            Self::Hexagon => "hexagon",
            Self::Plug => "plug",
            Self::Puzzle => "puzzle",
            Self::Shapes => "shapes",
        }
    }

    /// The host icon this one draws as.
    pub const fn to_icon(self) -> Icon {
        match self {
            Self::Box => Icon::Box,
            Self::Gem => Icon::Gem,
            Self::Hexagon => Icon::Hexagon,
            Self::Plug => Icon::Plug,
            Self::Puzzle => Icon::Puzzle,
            Self::Shapes => Icon::Shapes,
        }
    }
}
