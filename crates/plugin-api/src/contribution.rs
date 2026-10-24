//! The author-facing words for where and what a contribution is.

use nocterm_plugin_model::{ContributionKind, Zone as ModelZone};
use serde::{Deserialize, Serialize};

/// A zone of the status bar, as an author names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Zone {
    Left,
    Center,
    Right,
}

impl Zone {
    /// Every zone, in declaration order.
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }

    pub const fn to_model(self) -> ModelZone {
        match self {
            Self::Left => ModelZone::Left,
            Self::Center => ModelZone::Center,
            Self::Right => ModelZone::Right,
        }
    }
}

/// What a contribution is, as an author names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Widget,
    Panel,
}

impl Kind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 2] = [Self::Widget, Self::Panel];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Widget => "widget",
            Self::Panel => "panel",
        }
    }

    pub const fn to_model(self) -> ContributionKind {
        match self {
            Self::Widget => ContributionKind::Widget,
            Self::Panel => ContributionKind::Panel,
        }
    }
}

/// Where a contribution is drawn, which a snapshot tells the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContributionContext {
    pub kind: Kind,
    pub zone: Zone,
}
