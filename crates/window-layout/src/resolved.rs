//! The arrangement the shell draws: a preset checked against the catalog.

use nocterm_plugin_model::{ContributionId, GroupId, Side, Zone};

/// A preset resolved against the catalog.
///
/// Every contribution is placed at most once. Separators never lead, trail or double in a zone
/// or a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLayout {
    pub(crate) left: Vec<ResolvedItem>,
    pub(crate) center: Vec<ResolvedItem>,
    pub(crate) right: Vec<ResolvedItem>,
    pub(crate) open: Vec<ContributionId>,
    /// The panels the bar leaves out, each with the group it opens in.
    pub(crate) tray: Vec<(ContributionId, GroupId)>,
}

/// One entry of a resolved zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedItem {
    Widget(ContributionId),
    Separator,
    Group(ResolvedGroup),
}

/// A panel group in the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedGroup {
    pub(crate) id: GroupId,
    pub(crate) side: Side,
    pub(crate) primary: bool,
    pub(crate) members: Vec<ResolvedMember>,
}

/// One entry of a resolved group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedMember {
    Panel(ContributionId),
    Separator,
}

impl ResolvedLayout {
    pub fn zone(&self, zone: Zone) -> &[ResolvedItem] {
        match zone {
            Zone::Left => &self.left,
            Zone::Center => &self.center,
            Zone::Right => &self.right,
        }
    }

    /// The groups in bar order: the left zone, then the centre, then the right zone.
    pub fn groups(&self) -> impl Iterator<Item = &ResolvedGroup> + '_ {
        [Zone::Left, Zone::Center, Zone::Right]
            .into_iter()
            .flat_map(|zone| self.zone(zone))
            .filter_map(|item| match item {
                ResolvedItem::Group(group) => Some(group),
                _ => None,
            })
    }

    /// The group that holds the sidebar toggle. There is always exactly one.
    pub fn primary_group(&self) -> Option<&ResolvedGroup> {
        self.groups().find(|group| group.primary)
    }

    /// The group a placed panel belongs to.
    pub fn group_of(&self, panel: &ContributionId) -> Option<&ResolvedGroup> {
        self.groups().find(|group| group.holds(panel))
    }

    /// The group a panel opens in: the group that places it, or for a panel the bar leaves out
    /// the group its placement hints name. `None` for an id that is no panel of the catalog.
    pub fn host_of(&self, panel: &ContributionId) -> Option<&GroupId> {
        self.group_of(panel).map(ResolvedGroup::id).or_else(|| {
            self.tray
                .iter()
                .find_map(|(id, group)| (id == panel).then_some(group))
        })
    }

    /// Whether the bar places the contribution, as a widget or a group member.
    pub fn is_placed(&self, id: &ContributionId) -> bool {
        [Zone::Left, Zone::Center, Zone::Right]
            .into_iter()
            .flat_map(|zone| self.zone(zone))
            .any(|item| match item {
                ResolvedItem::Widget(widget) => widget == id,
                ResolvedItem::Group(group) => group.holds(id),
                ResolvedItem::Separator => false,
            })
    }

    /// The panels to open with the layout, at most one per group.
    pub fn open(&self) -> &[ContributionId] {
        &self.open
    }

    /// The same arrangement with every group on the other side of the content.
    pub fn swapped(&self) -> Self {
        let flip = |items: &[ResolvedItem]| {
            items
                .iter()
                .map(|item| match item {
                    ResolvedItem::Group(group) => ResolvedItem::Group(ResolvedGroup {
                        side: group.side.swapped_if(true),
                        ..group.clone()
                    }),
                    other => other.clone(),
                })
                .collect()
        };
        Self {
            left: flip(&self.left),
            center: flip(&self.center),
            right: flip(&self.right),
            open: self.open.clone(),
            tray: self.tray.clone(),
        }
    }
}

impl ResolvedGroup {
    pub fn id(&self) -> &GroupId {
        &self.id
    }

    /// The side of the content the group opens on.
    pub fn side(&self) -> Side {
        self.side
    }

    pub fn is_primary(&self) -> bool {
        self.primary
    }

    pub fn members(&self) -> &[ResolvedMember] {
        &self.members
    }

    /// The member panels in order, without separators.
    pub fn panels(&self) -> impl Iterator<Item = &ContributionId> + '_ {
        self.members.iter().filter_map(|member| match member {
            ResolvedMember::Panel(id) => Some(id),
            ResolvedMember::Separator => None,
        })
    }

    pub fn holds(&self, panel: &ContributionId) -> bool {
        self.panels().any(|member| member == panel)
    }
}
