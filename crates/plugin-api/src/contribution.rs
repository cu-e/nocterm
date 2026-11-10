//! The author-facing words for where and what a contribution is.

use nocterm_plugin_model::{ContributionKind, Edge as ModelEdge, ToolbarTabs, Zone as ModelZone};
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
    /// A strip along an edge of the tab area.
    Toolbar,
}

impl Kind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 3] = [Self::Widget, Self::Panel, Self::Toolbar];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Widget => "widget",
            Self::Panel => "panel",
            Self::Toolbar => "toolbar",
        }
    }

    pub const fn to_model(self) -> ContributionKind {
        match self {
            Self::Widget => ContributionKind::Widget,
            Self::Panel => ContributionKind::Panel,
            Self::Toolbar => ContributionKind::Toolbar,
        }
    }

    /// The author's word for a kind of the shell.
    pub const fn of_model(kind: ContributionKind) -> Self {
        match kind {
            ContributionKind::Widget => Self::Widget,
            ContributionKind::Panel => Self::Panel,
            ContributionKind::Toolbar => Self::Toolbar,
        }
    }
}

/// The edge of the tab area a toolbar suggests, as an author names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Edge {
    /// Under the tab bar.
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    /// Every edge, in declaration order.
    pub const ALL: [Self; 4] = [Self::Top, Self::Bottom, Self::Left, Self::Right];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub const fn to_model(self) -> ModelEdge {
        match self {
            Self::Top => ModelEdge::Top,
            Self::Bottom => ModelEdge::Bottom,
            Self::Left => ModelEdge::Left,
            Self::Right => ModelEdge::Right,
        }
    }
}

/// Which tabs a toolbar runs along, as an author names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tabs {
    /// Only the tabs the plugin opened, inside each of them.
    #[default]
    Own,
    /// Every tab, along the window's tab area.
    All,
}

impl Tabs {
    /// Every choice, in declaration order.
    pub const ALL: [Self; 2] = [Self::Own, Self::All];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::All => "all",
        }
    }

    pub const fn to_model(self) -> ToolbarTabs {
        match self {
            Self::Own => ToolbarTabs::Own,
            Self::All => ToolbarTabs::All,
        }
    }
}

/// Where a contribution is drawn, which a snapshot tells the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContributionContext {
    pub kind: Kind,
    /// The status bar zone of a widget or panel toggle. For a toolbar, its axis: `center`
    /// across the top or the bottom, `left` or `right` along that side, as a column.
    pub zone: Zone,
}
