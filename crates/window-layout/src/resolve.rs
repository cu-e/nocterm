//! Resolution of a preset against the plugin catalog.

use std::collections::{BTreeSet, HashSet};

use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, ContributionRef, GroupId, GroupSpec, PluginId,
    PluginSet, Side, WidthClass, Zone,
};

use crate::{
    BarItem, GroupEntry, LayoutPreset, ResolvedGroup, ResolvedItem, ResolvedLayout, ResolvedMember,
};

pub(crate) const ZONES: [Zone; 3] = [Zone::Left, Zone::Center, Zone::Right];

/// Arranges the catalog's contributions as `preset` asks.
///
/// - Ids the catalog does not know, widgets named as panels and panels named as widgets are not
///   placed; the preset itself is never changed. A user contribution is placed only once the
///   user approved placing its package (`placeable`).
/// - A contribution is placed once: the first mention wins.
/// - Contributions that appear neither in the bar nor in the tray are placed where their hints
///   say, if they ask to be shown, in `(zone, order)` order. Panels join their hinted group, or
///   else the group of their width class, and a group joins the zone of its first panel.
/// - A pinned widget that is still unplaced goes to the end of the right zone. Pinning has no
///   effect on a panel: a panel the bar leaves out stays in the tray and opens in its hinted
///   group.
/// - The contributions of the plugins in `disabled` stay in the preset but are not placed, not
///   added by their hints, not left in the tray and not opened. The ones the bar names are
///   listed by [`ResolvedLayout::disabled`].
/// - Exactly one group is primary, and it is in the bar even without members.
/// - Separators never lead, trail or double in a zone or a group, and a group other than the
///   primary one needs a panel to stay.
pub fn resolve(
    preset: &LayoutPreset,
    catalog: &PluginSet,
    disabled: &BTreeSet<PluginId>,
) -> ResolvedLayout {
    let mut arrangement = Arrangement::new(catalog, disabled);
    for zone in ZONES {
        arrangement.place_bar(zone, preset.bar.zone(zone));
    }
    arrangement.place_by_hints(&preset.tray);
    arrangement.place_pinned();
    arrangement.ensure_primary_group();
    arrangement.finish(&preset.open)
}

/// The zones being filled and the contributions placed so far.
struct Arrangement<'a> {
    catalog: &'a PluginSet,
    disabled: &'a BTreeSet<PluginId>,
    zones: [Vec<ResolvedItem>; 3],
    placed: HashSet<ContributionId>,
    /// The disabled contributions the bar names, in bar order.
    skipped: Vec<ContributionId>,
}

impl<'a> Arrangement<'a> {
    fn new(catalog: &'a PluginSet, disabled: &'a BTreeSet<PluginId>) -> Self {
        Self {
            catalog,
            disabled,
            zones: Default::default(),
            placed: HashSet::new(),
            skipped: Vec::new(),
        }
    }

    /// Whether the plugin that owns the contribution is disabled. Core contributions have no
    /// plugin and are never disabled.
    fn is_disabled(&self, id: &ContributionId) -> bool {
        self.catalog
            .plugin_of(id)
            .is_some_and(|plugin| self.disabled.contains(plugin.id()))
    }

    fn items(&mut self, zone: Zone) -> &mut Vec<ResolvedItem> {
        &mut self.zones[ZONES
            .iter()
            .position(|candidate| *candidate == zone)
            .unwrap_or(0)]
    }

    fn place_bar(&mut self, zone: Zone, bar: &[BarItem]) {
        for item in bar {
            match item {
                BarItem::Separator => self.items(zone).push(ResolvedItem::Separator),
                BarItem::Item { id } => {
                    if self.claim(id, ContributionKind::Widget) {
                        self.items(zone).push(ResolvedItem::Widget(id.clone()));
                    }
                }
                BarItem::Group {
                    id, side, members, ..
                } => {
                    let Some(spec) = self.catalog.group(id.as_str()) else {
                        continue;
                    };
                    if self.group(&spec.id).is_some() {
                        continue;
                    }
                    let members = self.place_members(members);
                    self.items(zone).push(ResolvedItem::Group(ResolvedGroup {
                        id: spec.id.clone(),
                        side: side.unwrap_or(default_side(zone)),
                        primary: spec.primary,
                        members,
                    }));
                }
            }
        }
    }

    fn place_members(&mut self, entries: &[GroupEntry]) -> Vec<ResolvedMember> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                GroupEntry::Separator => Some(ResolvedMember::Separator),
                GroupEntry::Panel(id) => self
                    .claim(id, ContributionKind::Panel)
                    .then(|| ResolvedMember::Panel(id.clone())),
            })
            .collect()
    }

    /// Takes the contribution for the bar if it exists, has the kind, is enabled and is not
    /// placed yet. A disabled one is noted instead, once.
    fn claim(&mut self, id: &ContributionId, kind: ContributionKind) -> bool {
        let fits = self.catalog.is_placeable(id)
            && self
                .catalog
                .contribution(id)
                .is_some_and(|contribution| contribution.kind() == kind);
        if fits && self.is_disabled(id) {
            if !self.skipped.contains(id) {
                self.skipped.push(id.clone());
            }
            return false;
        }
        fits && self.placed.insert(id.clone())
    }

    fn group(&mut self, id: &GroupId) -> Option<&mut ResolvedGroup> {
        self.zones.iter_mut().flatten().find_map(|item| match item {
            ResolvedItem::Group(group) if group.id == *id => Some(group),
            _ => None,
        })
    }

    fn place_by_hints(&mut self, tray: &[ContributionId]) {
        for contribution in self.catalog.entries() {
            let id = contribution.id();
            let left_out = self.placed.contains(id)
                || tray.contains(id)
                || self.is_disabled(id)
                || !self.catalog.is_placeable(id);
            if contribution.placement().auto == AutoPlace::Show && !left_out {
                self.place_by_hint(contribution);
            }
        }
    }

    fn place_by_hint(&mut self, contribution: ContributionRef<'_>) {
        let zone = contribution.placement().zone;
        self.placed.insert(contribution.id().clone());
        if contribution.kind() == ContributionKind::Widget {
            self.items(zone)
                .push(ResolvedItem::Widget(contribution.id().clone()));
            return;
        }
        let Some(spec) = hinted_group(self.catalog, contribution) else {
            return;
        };
        let member = ResolvedMember::Panel(contribution.id().clone());
        if let Some(group) = self.group(&spec.id) {
            group.members.push(member);
            return;
        }
        self.items(zone).push(ResolvedItem::Group(ResolvedGroup {
            id: spec.id.clone(),
            side: default_side(zone),
            primary: spec.primary,
            members: vec![member],
        }));
    }

    fn place_pinned(&mut self) {
        for contribution in self.catalog.entries() {
            if contribution.pinned()
                && contribution.kind() == ContributionKind::Widget
                && !self.is_disabled(contribution.id())
                && self.placed.insert(contribution.id().clone())
            {
                self.items(Zone::Right)
                    .push(ResolvedItem::Widget(contribution.id().clone()));
            }
        }
    }

    /// Leaves exactly one primary group: the catalog's, which is added to the start of the left
    /// zone if the bar has none. A `primary` flag in the document is not read, because the
    /// panel groups and the actions that act on "the sidebar" take the catalog's.
    fn ensure_primary_group(&mut self) {
        let Some(spec) = self.catalog.groups().iter().find(|spec| spec.primary) else {
            return;
        };
        let keep = spec.id.clone();
        for item in self.zones.iter_mut().flatten() {
            if let ResolvedItem::Group(group) = item {
                group.primary = group.id == keep;
            }
        }
        if self.group(&keep).is_none() {
            self.items(Zone::Left).insert(
                0,
                ResolvedItem::Group(ResolvedGroup {
                    id: spec.id.clone(),
                    side: default_side(Zone::Left),
                    primary: true,
                    members: Vec::new(),
                }),
            );
        }
    }

    fn finish(self, open: &[ContributionId]) -> ResolvedLayout {
        let catalog = self.catalog;
        let disabled = self.disabled;
        let [left, center, right] = self.zones.map(tidy_zone);
        let mut layout = ResolvedLayout {
            left,
            center,
            right,
            open: Vec::new(),
            tray: Vec::new(),
            disabled: self.skipped,
        };
        layout.tray = catalog
            .entries()
            .filter(|contribution| {
                contribution.kind() == ContributionKind::Panel
                    && catalog.is_placeable(contribution.id())
                    && !layout.is_placed(contribution.id())
                    && !catalog
                        .plugin_of(contribution.id())
                        .is_some_and(|plugin| disabled.contains(plugin.id()))
            })
            .filter_map(|contribution| {
                let spec = hinted_group(catalog, contribution)?;
                Some((contribution.id().clone(), spec.id.clone()))
            })
            .collect();
        let mut opened_groups = HashSet::new();
        for id in open {
            let group = layout.group_of(id).map(|group| group.id.clone());
            let wanted = match group {
                Some(group) => opened_groups.insert(group),
                None => layout.is_placed(id),
            };
            if wanted && !layout.open.contains(id) {
                layout.open.push(id.clone());
            }
        }
        layout
    }
}

/// The group a panel opens in: the one it names, else the primary group for a navigation panel
/// and the first other group for an assistant.
pub(crate) fn hinted_group<'a>(
    catalog: &'a PluginSet,
    contribution: ContributionRef<'_>,
) -> Option<&'a GroupSpec> {
    let groups = catalog.groups();
    let named = contribution
        .placement()
        .group
        .and_then(|hint| catalog.group(hint));
    let by_width = || {
        let assistant = contribution
            .panel()
            .is_some_and(|traits| traits.width == WidthClass::Assistant);
        groups
            .iter()
            .find(|spec| spec.primary != assistant)
            .or_else(|| groups.iter().find(|spec| spec.primary))
    };
    named.or_else(by_width)
}

pub(crate) fn default_side(zone: Zone) -> Side {
    match zone {
        Zone::Left | Zone::Center => Side::Left,
        Zone::Right => Side::Right,
    }
}

/// Collapses separators in the zone and its groups, and drops groups that have nothing to
/// toggle.
fn tidy_zone(items: Vec<ResolvedItem>) -> Vec<ResolvedItem> {
    let items = items
        .into_iter()
        .filter_map(|item| match item {
            ResolvedItem::Group(mut group) => {
                group.members = collapse(group.members, |member| {
                    matches!(member, ResolvedMember::Separator)
                });
                let keeps = group.primary || group.panels().next().is_some();
                keeps.then_some(ResolvedItem::Group(group))
            }
            other => Some(other),
        })
        .collect();
    collapse(items, |item| matches!(item, ResolvedItem::Separator))
}

/// Removes separators at the start and the end and all but the first of each run.
fn collapse<T>(items: Vec<T>, is_separator: impl Fn(&T) -> bool) -> Vec<T> {
    let mut kept: Vec<T> = Vec::with_capacity(items.len());
    for item in items {
        let doubled = is_separator(&item) && kept.last().is_none_or(&is_separator);
        if !doubled {
            kept.push(item);
        }
    }
    while kept.last().is_some_and(&is_separator) {
        kept.pop();
    }
    kept
}
