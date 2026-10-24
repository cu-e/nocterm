//! What the window shell can host and the hints it places it by.
//!
//! All of these are plain data an owner crate declares in `const` items with struct literals.

use serde::{Deserialize, Serialize};

use crate::{ContributionId, GroupId};

/// What a contribution is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContributionKind {
    /// A view in a side panel group, opened by its toggle.
    Panel,
    /// A view in the status bar.
    Widget,
}

/// A zone of the status bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    Left,
    Center,
    Right,
}

/// Which side of the content a panel group opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// The opposite side when `swapped`.
    pub fn swapped_if(self, swapped: bool) -> Self {
        match (self, swapped) {
            (side, false) => side,
            (Self::Left, true) => Self::Right,
            (Self::Right, true) => Self::Left,
        }
    }
}

/// Whether a contribution takes a place in the default arrangement or waits in the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoPlace {
    Show,
    Tray,
}

/// The icons a contribution may name. The ui layer maps each to a drawable icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Activity,
    Bell,
    Bot,
    Box,
    Clock,
    Container,
    FolderTree,
    Gem,
    Hexagon,
    Layers,
    PanelRight,
    Plug,
    Puzzle,
    ScrollText,
    Server,
    Shapes,
    ShieldCheck,
    SquareTerminal,
    Upload,
}

impl Icon {
    /// Every icon, in declaration order.
    pub const ALL: [Self; 19] = [
        Self::Activity,
        Self::Bell,
        Self::Bot,
        Self::Box,
        Self::Clock,
        Self::Container,
        Self::FolderTree,
        Self::Gem,
        Self::Hexagon,
        Self::Layers,
        Self::PanelRight,
        Self::Plug,
        Self::Puzzle,
        Self::ScrollText,
        Self::Server,
        Self::Shapes,
        Self::ShieldCheck,
        Self::SquareTerminal,
        Self::Upload,
    ];

    /// The icon a user plugin contribution may show.
    ///
    /// Only icons that no system plugin or core contribution draws are allowed, so a plugin
    /// toggle cannot pass for a host function: a footer toggle shows only its icon. A system
    /// contribution must therefore never take one of these; the app checks the catalog against
    /// this set.
    pub const fn user_allowed(self) -> bool {
        match self {
            Self::Box | Self::Gem | Self::Hexagon | Self::Plug | Self::Puzzle | Self::Shapes => {
                true
            }
            Self::Activity
            | Self::Bell
            | Self::Bot
            | Self::Clock
            | Self::Container
            | Self::FolderTree
            | Self::Layers
            | Self::PanelRight
            | Self::ScrollText
            | Self::Server
            | Self::ShieldCheck
            | Self::SquareTerminal
            | Self::Upload => false,
        }
    }
}

/// Panel groups whose columns share one remembered width and resize range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidthClass {
    Navigation,
    Assistant,
}

/// How a panel is framed in its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelChrome {
    /// The shell draws the upper-case title above the view.
    Titled,
    /// The panel draws its own header.
    OwnHeader,
}

/// What a panel needs from its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelTraits {
    pub width: WidthClass,
    pub chrome: PanelChrome,
}

/// A panel group: panels in it exclude each other, and groups are independent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSpec {
    pub id: GroupId,
    /// The name the user sees for the group.
    pub title: &'static str,
    /// The group behind the sidebar toggle and `ToggleSidebar`.
    pub primary: bool,
}

/// Where the default arrangement puts a contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub zone: Zone,
    /// Sorts contributions within a zone; unique within it.
    pub order: u16,
    /// The group a panel opens in. Widgets have none.
    pub group: Option<&'static str>,
    pub auto: AutoPlace,
}

/// A panel or widget a system plugin or a core function offers to the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemContribution {
    pub id: ContributionId,
    pub kind: ContributionKind,
    /// The name beside the toggle (its tooltip) and in the editor.
    pub title: &'static str,
    pub icon: Icon,
    pub placement: Placement,
    /// Present exactly for panels.
    pub panel: Option<PanelTraits>,
    /// Sample text for editor previews.
    pub preview: Option<&'static str>,
    /// A pinned contribution can move but not leave the bar.
    pub pinned: bool,
}
