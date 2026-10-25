//! What the editor shows of one layout, derived from the settings on each draw.

use std::collections::{BTreeMap, BTreeSet};

use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{ContributionId, GroupId, GroupSpec, PluginId, PluginSet, Side, Zone};
use nocterm_window_layout::{
    BarItem, BuiltIn, LayoutSettings, PresetId, ResolvedGroup, ResolvedItem, ResolvedLayout,
    ResolvedMember, Slot, Subject, select,
};

/// A layout the picker offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    id: PresetId,
    name: String,
    built_in: bool,
    /// The layout the windows show.
    in_use: bool,
}

impl Choice {
    pub(crate) fn id(&self) -> &PresetId {
        &self.id
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn in_use(&self) -> bool {
        self.in_use
    }
}

/// The layout the editor shows, resolved with every plugin enabled so that nothing the user
/// can edit is hidden, and the plugins that are in fact disabled.
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    shown: PresetId,
    name: String,
    built_in: bool,
    in_use: bool,
    choices: Vec<Choice>,
    layout: ResolvedLayout,
    /// Contributions the layout leaves out of the bar, in catalog order.
    tray: Vec<ContributionId>,
    /// Ids in the tray that this application does not know.
    unknown: Vec<ContributionId>,
    disabled: BTreeSet<PluginId>,
    groups: Vec<GroupSpec>,
    /// The side a group was told to open on; a group that follows its zone is not listed.
    sides: BTreeMap<GroupId, Side>,
}

impl Snapshot {
    /// `shown` is the layout to show; `None` follows the one in use.
    pub(crate) fn new(
        settings: &LayoutSettings,
        enablement: &PluginEnablement,
        legacy_swapped: bool,
        shown: Option<&PresetId>,
        catalog: &PluginSet,
    ) -> Self {
        let in_use_id = select(settings, legacy_swapped);
        let shown = shown
            .filter(|id| BuiltIn::of(id).is_some() || settings.exists(id))
            .cloned()
            .unwrap_or_else(|| in_use_id.clone());
        let built_in = BuiltIn::of(&shown);
        let name = match built_in {
            Some(built_in) => built_in.name().to_owned(),
            None => settings
                .preset(&shown)
                .map(|preset| preset.name.clone())
                .unwrap_or_default(),
        };
        let layout = settings.resolve(&shown, catalog, &BTreeSet::new());
        let tray = catalog
            .entries()
            .filter(|contribution| !layout.is_placed(contribution.id()))
            .map(|contribution| contribution.id().clone())
            .collect();
        let unknown = settings
            .preset(&shown)
            .map(|preset| {
                preset
                    .tray
                    .iter()
                    .filter(|id| catalog.contribution(id).is_none())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let sides = settings
            .preset(&shown)
            .into_iter()
            .flat_map(|preset| {
                [Zone::Left, Zone::Center, Zone::Right]
                    .into_iter()
                    .flat_map(|zone| preset.bar.zone(zone))
            })
            .filter_map(|item| match item {
                BarItem::Group {
                    id,
                    side: Some(side),
                    ..
                } => Some((id.clone(), *side)),
                _ => None,
            })
            .collect();
        let choices = BuiltIn::ALL
            .into_iter()
            .map(|built_in| (built_in.id(), built_in.name().to_owned(), true))
            .chain(
                settings
                    .presets
                    .iter()
                    .map(|preset| (preset.id.clone(), preset.name.clone(), false)),
            )
            .map(|(id, name, built_in)| Choice {
                in_use: id == in_use_id,
                id,
                name,
                built_in,
            })
            .collect();
        Self {
            in_use: shown == in_use_id,
            shown,
            name,
            built_in: built_in.is_some(),
            choices,
            layout,
            tray,
            unknown,
            disabled: enablement.effective_disabled(catalog.catalog()),
            groups: catalog.groups().to_vec(),
            sides,
        }
    }

    pub(crate) fn shown(&self) -> &PresetId {
        &self.shown
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn built_in(&self) -> bool {
        self.built_in
    }

    pub(crate) fn choices(&self) -> &[Choice] {
        &self.choices
    }

    pub(crate) fn layout(&self) -> &ResolvedLayout {
        &self.layout
    }

    pub(crate) fn tray(&self) -> &[ContributionId] {
        &self.tray
    }

    pub(crate) fn unknown(&self) -> &[ContributionId] {
        &self.unknown
    }

    pub(crate) fn disabled(&self) -> &BTreeSet<PluginId> {
        &self.disabled
    }

    pub(crate) fn groups(&self) -> &[GroupSpec] {
        &self.groups
    }

    /// The side a group was told to open on, or `None` while it follows its zone.
    pub(crate) fn side_override(&self, group: &GroupId) -> Option<Side> {
        self.sides.get(group).copied()
    }

    /// Whether the controls act on this layout. A built-in can be edited only while it is the
    /// layout in use, because the first edit copies it and the copy takes its place.
    pub(crate) fn editable(&self) -> bool {
        !self.built_in || self.in_use
    }

    pub(crate) fn is_disabled(&self, id: &ContributionId, catalog: &PluginSet) -> bool {
        catalog
            .plugin_of(id)
            .is_some_and(|plugin| self.disabled.contains(plugin.id()))
    }

    /// The zone a placed widget, group or panel is in.
    pub(crate) fn zone_of(&self, subject: &Subject) -> Option<Zone> {
        self.find(subject).map(|found| found.zone)
    }

    /// Where a separator goes to follow `subject`: the zone for a widget or a group, the group
    /// for a panel.
    pub(crate) fn after(&self, subject: &Subject) -> Option<(Slot, usize)> {
        let found = self.find(subject)?;
        Some(match found.member {
            Some((group, member)) => (Slot::Group(group), member + 1),
            None => (Slot::Zone(found.zone), found.index + 1),
        })
    }

    /// Where a separator goes to precede `subject`: its own place in the list.
    pub(crate) fn before(&self, subject: &Subject) -> Option<(Slot, usize)> {
        let found = self.find(subject)?;
        Some(match found.member {
            Some((group, member)) => (Slot::Group(group), member),
            None => (Slot::Zone(found.zone), found.index),
        })
    }

    /// Whether `subject` has an entry before it (`by` is -1) or after it (+1) in its list, so
    /// that moving it by one changes something.
    pub(crate) fn can_shift(&self, subject: &Subject, by: i32) -> bool {
        let Some(found) = self.find(subject) else {
            return false;
        };
        let (index, len) = match &found.member {
            Some((_, member)) => (*member, found.members),
            None => (found.index, found.len),
        };
        if by < 0 { index > 0 } else { index + 1 < len }
    }

    fn find(&self, subject: &Subject) -> Option<Found> {
        for zone in [Zone::Left, Zone::Center, Zone::Right] {
            let len = self.layout.zone(zone).len();
            for (index, item) in self.layout.zone(zone).iter().enumerate() {
                match (item, subject) {
                    (ResolvedItem::Widget(id), Subject::Contribution(wanted)) if id == wanted => {
                        return Some(Found {
                            zone,
                            index,
                            len,
                            members: 0,
                            member: None,
                        });
                    }
                    (ResolvedItem::Group(group), Subject::Group(wanted))
                        if group.id() == wanted =>
                    {
                        return Some(Found {
                            zone,
                            index,
                            len,
                            members: 0,
                            member: None,
                        });
                    }
                    (ResolvedItem::Group(group), Subject::Contribution(wanted)) => {
                        let member = group.members().iter().position(
                            |member| matches!(member, ResolvedMember::Panel(id) if id == wanted),
                        );
                        if let Some(member) = member {
                            return Some(Found {
                                zone,
                                index,
                                len,
                                members: group.members().len(),
                                member: Some((group.id().clone(), member)),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    pub(crate) fn group_of(&self, panel: &ContributionId) -> Option<&ResolvedGroup> {
        self.layout.group_of(panel)
    }

    pub(crate) fn zone_of_group(&self, group: &GroupId) -> Option<Zone> {
        [Zone::Left, Zone::Center, Zone::Right]
            .into_iter()
            .find(|zone| {
                self.layout
                    .zone(*zone)
                    .iter()
                    .any(|item| matches!(item, ResolvedItem::Group(placed) if placed.id() == group))
            })
    }
}

/// Where `find` met a subject.
struct Found {
    zone: Zone,
    index: usize,
    /// How many entries the zone holds.
    len: usize,
    /// For a panel, how many entries its group holds.
    members: usize,
    /// For a panel, its group and its place among the members.
    member: Option<(GroupId, usize)>,
}
