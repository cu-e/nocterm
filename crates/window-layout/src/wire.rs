//! The document shape of bar items and group entries.
//!
//! The in-memory enums name their variants for the code that edits them; the document is
//! shorter: `{ item = "id" }`, `{ separator = true }` and `{ group = "id", ... }`, and a plain
//! string for a panel in a group. These mirror types carry that shape and its schema, so the
//! plugin model needs no `schemars` dependency.

use nocterm_plugin_model::{ContributionId, GroupId, Side};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BarItem, GroupEntry};

/// A side of the content, as the document spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SideName {
    Left,
    Right,
}

impl From<SideName> for Side {
    fn from(side: SideName) -> Self {
        match side {
            SideName::Left => Self::Left,
            SideName::Right => Self::Right,
        }
    }
}

impl From<Side> for SideName {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Left,
            Side::Right => Self::Right,
        }
    }
}

/// An entry of a zone in the document.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum BarItemWire {
    /// A widget.
    Item {
        #[schemars(with = "String")]
        item: ContributionId,
    },
    /// A divider; the value must be `true`.
    Separator { separator: bool },
    /// A panel group.
    Group {
        #[schemars(with = "String")]
        group: GroupId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<SideName>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        primary: bool,
        #[serde(default)]
        members: Vec<GroupEntry>,
    },
}

/// An entry of a group in the document.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum GroupEntryWire {
    /// A panel, by contribution id.
    Panel(#[schemars(with = "String")] ContributionId),
    /// A divider; the value must be `true`.
    Separator { separator: bool },
}

impl From<BarItem> for BarItemWire {
    fn from(item: BarItem) -> Self {
        match item {
            BarItem::Item { id } => Self::Item { item: id },
            BarItem::Separator => Self::Separator { separator: true },
            BarItem::Group {
                id,
                side,
                primary,
                members,
            } => Self::Group {
                group: id,
                side: side.map(SideName::from),
                primary,
                members,
            },
        }
    }
}

impl TryFrom<BarItemWire> for BarItem {
    type Error = &'static str;

    fn try_from(wire: BarItemWire) -> Result<Self, Self::Error> {
        match wire {
            BarItemWire::Item { item } => Ok(Self::Item { id: item }),
            BarItemWire::Separator { separator: true } => Ok(Self::Separator),
            BarItemWire::Separator { separator: false } => {
                Err("a separator is written `{ separator = true }`")
            }
            BarItemWire::Group {
                group,
                side,
                primary,
                members,
            } => Ok(Self::Group {
                id: group,
                side: side.map(Side::from),
                primary,
                members,
            }),
        }
    }
}

impl From<GroupEntry> for GroupEntryWire {
    fn from(entry: GroupEntry) -> Self {
        match entry {
            GroupEntry::Panel(id) => Self::Panel(id),
            GroupEntry::Separator => Self::Separator { separator: true },
        }
    }
}

impl TryFrom<GroupEntryWire> for GroupEntry {
    type Error = &'static str;

    fn try_from(wire: GroupEntryWire) -> Result<Self, Self::Error> {
        match wire {
            GroupEntryWire::Panel(id) => Ok(Self::Panel(id)),
            GroupEntryWire::Separator { separator: true } => Ok(Self::Separator),
            GroupEntryWire::Separator { separator: false } => {
                Err("a separator is written `{ separator = true }`")
            }
        }
    }
}
