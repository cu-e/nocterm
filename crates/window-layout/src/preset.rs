//! A layout preset as the user's document would hold it.

use std::{borrow::Cow, fmt};

use crate::wire::{BarItemWire, GroupEntryWire};

use nocterm_plugin_model::{ContributionId, Edge, GroupId, Side, Zone};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The id of a preset. The built-in ids are `default` and `swapped`.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct PresetId(Cow<'static, str>);

impl PresetId {
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// An id made at run time, such as `p3`.
    pub fn from_owned(id: String) -> Self {
        Self(Cow::Owned(id))
    }
}

impl PresetId {
    /// The id of a preset that a document left without one.
    pub(crate) fn is_unset(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for PresetId {
    fn default() -> Self {
        Self::new("")
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One saved arrangement.
///
/// Every field but the id may be left out of the document. A missing or unusable id is replaced
/// when the section is sanitized.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LayoutPreset {
    #[serde(skip_serializing_if = "PresetId::is_unset")]
    pub id: PresetId,
    /// Shown to the user; never used to find the preset.
    pub name: String,
    pub bar: Bar,
    /// Contributions that stay registered but unplaced, so the default arrangement does not
    /// add them.
    #[schemars(with = "Vec<String>")]
    pub tray: Vec<ContributionId>,
    /// Panels opened when the preset is selected.
    #[schemars(with = "Vec<String>")]
    pub open: Vec<ContributionId>,
    /// The toolbars along the edges of the tab area. A toolbar named neither here nor in the
    /// tray follows its hints.
    #[serde(skip_serializing_if = "Toolbars::is_empty")]
    pub toolbars: Toolbars,
}

/// The toolbars along each edge of the tab area, each edge in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Toolbars {
    #[schemars(with = "Vec<String>")]
    pub top: Vec<ContributionId>,
    #[schemars(with = "Vec<String>")]
    pub bottom: Vec<ContributionId>,
    #[schemars(with = "Vec<String>")]
    pub left: Vec<ContributionId>,
    #[schemars(with = "Vec<String>")]
    pub right: Vec<ContributionId>,
}

impl Toolbars {
    pub fn edge(&self, edge: Edge) -> &[ContributionId] {
        match edge {
            Edge::Top => &self.top,
            Edge::Bottom => &self.bottom,
            Edge::Left => &self.left,
            Edge::Right => &self.right,
        }
    }

    pub(crate) fn edge_mut(&mut self, edge: Edge) -> &mut Vec<ContributionId> {
        match edge {
            Edge::Top => &mut self.top,
            Edge::Bottom => &mut self.bottom,
            Edge::Left => &mut self.left,
            Edge::Right => &mut self.right,
        }
    }

    pub fn is_empty(&self) -> bool {
        Edge::ALL.iter().all(|edge| self.edge(*edge).is_empty())
    }

    /// The edge that names the toolbar first, if any does.
    pub fn edge_of(&self, id: &ContributionId) -> Option<Edge> {
        Edge::ALL
            .into_iter()
            .find(|edge| self.edge(*edge).contains(id))
    }

    /// Takes the toolbar off every edge.
    pub(crate) fn remove(&mut self, id: &ContributionId) {
        for edge in Edge::ALL {
            self.edge_mut(edge).retain(|named| named != id);
        }
    }
}

/// The three zones of the status bar, each in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
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
///
/// In the document an entry is a table with exactly one of the keys `item` (a widget id),
/// `separator = true`, `panel` (a panel id, with an optional `side`) or `group` (a group id,
/// with optional `name`, `side`, `primary` and `members`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "BarItemWire", into = "BarItemWire")]
#[schemars(with = "BarItemWire")]
pub enum BarItem {
    /// A widget.
    Item {
        id: ContributionId,
    },
    /// A panel of its own: its toggle stands in the zone and it opens in a column of its own,
    /// beside the other open panels.
    Panel {
        id: ContributionId,
        /// The side of the content the panel opens on; `None` follows the zone.
        side: Option<Side>,
    },
    Separator,
    /// A panel group, drawn as the toggles of its members.
    ///
    /// The groups of the catalog keep their ids; any other well-formed id is a group the user
    /// made.
    Group {
        id: GroupId,
        /// The name the user gave the group; `None` keeps the catalog's title.
        name: Option<String>,
        /// The side of the content the group opens on; `None` follows the zone.
        side: Option<Side>,
        /// Read and kept so a document round-trips, but not used: the catalog decides which
        /// group is primary.
        primary: bool,
        members: Vec<GroupEntry>,
    },
}

/// One entry of a group: a panel id, or the table `{ separator = true }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "GroupEntryWire", into = "GroupEntryWire")]
#[schemars(with = "GroupEntryWire")]
pub enum GroupEntry {
    Panel(ContributionId),
    Separator,
}
