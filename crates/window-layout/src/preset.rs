//! A layout preset as the user's document would hold it.

use std::{borrow::Cow, fmt};

use nocterm_plugin_model::{ContributionId, GroupId, Side, Zone};

/// The id of a preset. The built-in ids are `default` and `swapped`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PresetId(Cow<'static, str>);

impl PresetId {
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One saved arrangement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutPreset {
    pub id: PresetId,
    /// Shown to the user; never used to find the preset.
    pub name: String,
    pub bar: Bar,
    /// Contributions that stay registered but unplaced, so the default arrangement does not
    /// add them.
    pub tray: Vec<ContributionId>,
    /// Panels opened when the preset is selected.
    pub open: Vec<ContributionId>,
}

/// The three zones of the status bar, each in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    pub left: Vec<BarItem>,
    pub center: Vec<BarItem>,
    pub right: Vec<BarItem>,
}

impl Bar {
    pub fn zone(&self, zone: Zone) -> &[BarItem] {
        match zone {
            Zone::Left => &self.left,
            Zone::Center => &self.center,
            Zone::Right => &self.right,
        }
    }
}

/// One entry of a zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarItem {
    /// A widget.
    Item {
        id: ContributionId,
    },
    Separator,
    /// A panel group, drawn as the toggles of its members.
    Group {
        id: GroupId,
        /// The side of the content the group opens on; `None` follows the zone.
        side: Option<Side>,
        /// Read and kept so a document round-trips, but not used: the catalog decides which
        /// group is primary.
        primary: bool,
        members: Vec<GroupEntry>,
    },
}

/// One entry of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupEntry {
    Panel(ContributionId),
    Separator,
}
