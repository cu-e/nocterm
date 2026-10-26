//! The entry operations: moving, grouping, separating, removing and restoring what the bar
//! holds, on a preset in its written-out form.

use nocterm_plugin_model::{
    ContributionId, ContributionKind, ContributionRef, GroupId, PluginSet, Side, Zone,
};

use super::{EditError, EditOp, Limit, Slot, Subject, settle::settle};
use crate::{
    BarItem, GroupEntry, LayoutPreset, LayoutSettings, MAX_GROUP_MEMBERS, MAX_NAME_CHARS,
    MAX_ZONE_ITEMS, NameError, PresetId,
    name::same_name,
    resolve::{ZONES, hinted_group},
    solo_group_id,
};

/// Applies an entry operation to the stored preset `id`.
///
/// The preset is written out first and tidied after, so what the operation sees is what the
/// editor shows, and entries it leaves invalid (a leading separator, a group with no panel, two
/// open panels in a group) are dropped.
pub(super) fn apply(
    next: &mut LayoutSettings,
    id: &PresetId,
    catalog: &PluginSet,
    op: EditOp,
) -> Result<(), EditError> {
    let Some(stored) = next.presets.iter_mut().find(|preset| preset.id == *id) else {
        return Err(EditError::PresetGone(id.clone()));
    };
    let mut preset = settle(stored, catalog);
    Editing {
        catalog,
        preset: &mut preset,
    }
    .run(op)?;
    *stored = settle(&preset, catalog);
    Ok(())
}

/// Where an entry sits in the bar: a zone and an index, and for a panel the group's index in
/// the zone and the panel's in the group.
#[derive(Clone, Copy)]
enum Spot {
    Zone(Zone, usize),
    Member(Zone, usize, usize),
}

struct Editing<'a> {
    catalog: &'a PluginSet,
    preset: &'a mut LayoutPreset,
}

impl<'a> Editing<'a> {
    fn run(&mut self, op: EditOp) -> Result<(), EditError> {
        match op {
            EditOp::Move { subject, by } => self.shift(&subject, by),
            EditOp::ToZone { subject, zone } => self.put_in_zone(&subject, zone),
            EditOp::ToGroup { panel, group } => self.put_in_group(&panel, &group),
            EditOp::MoveTo {
                subject,
                place,
                index,
            } => self.move_to(&subject, &place, index),
            EditOp::InsertSeparator { place, index } => self.insert_separator(&place, index),
            EditOp::RemoveSeparator { place, index } => self.remove_separator(&place, index),
            EditOp::RemoveToTray { id } => self.remove_to_tray(&id),
            EditOp::Restore { id } => self.restore(&id),
            EditOp::PlaceIn { id, zone } => self.place_in(&id, zone),
            EditOp::Forget { id } => self.forget(&id),
            EditOp::SetOpen { panel, open } => self.set_open(&panel, open),
            EditOp::SetSide { group, side } => self.set_side(&group, side),
            EditOp::GroupWith { panel, target } => self.group_with(&panel, &target),
            EditOp::NewGroup { panel } => self.new_group(&panel),
            EditOp::Ungroup { group } => self.ungroup(&group),
            EditOp::RenameGroup { group, name } => self.rename_group(&group, &name),
            // Handled before the entry operations are reached.
            EditOp::Rename { .. }
            | EditOp::Duplicate
            | EditOp::Delete
            | EditOp::Reset
            | EditOp::RestorePresets(_) => Ok(()),
        }
    }

    fn zone(&mut self, zone: Zone) -> &mut Vec<BarItem> {
        match zone {
            Zone::Left => &mut self.preset.bar.left,
            Zone::Center => &mut self.preset.bar.center,
            Zone::Right => &mut self.preset.bar.right,
        }
    }

    fn find(&self, subject: &Subject) -> Option<Spot> {
        ZONES.iter().find_map(|zone| {
            self.preset
                .bar
                .zone(*zone)
                .iter()
                .enumerate()
                .find_map(|(at, item)| spot_in(*zone, at, item, subject))
        })
    }

    fn group_at(&self, id: &GroupId) -> Option<(Zone, usize)> {
        ZONES.iter().find_map(|zone| {
            let at =
                self.preset.bar.zone(*zone).iter().position(
                    |item| matches!(item, BarItem::Group { id: group, .. } if group == id),
                )?;
            Some((*zone, at))
        })
    }

    fn group_mut(&mut self, id: &GroupId) -> Option<&mut BarItem> {
        let (zone, at) = self.group_at(id)?;
        self.zone(zone).get_mut(at)
    }

    fn members_of(&mut self, id: &GroupId) -> Option<&mut Vec<GroupEntry>> {
        match self.group_mut(id)? {
            BarItem::Group { members, .. } => Some(members),
            _ => None,
        }
    }

    /// Takes the entry out of the bar and returns what it was.
    fn take(&mut self, spot: Spot) -> Option<BarItem> {
        match spot {
            Spot::Zone(zone, at) => Some(self.zone(zone).remove(at)),
            Spot::Member(zone, group, at) => {
                let BarItem::Group { members, .. } = self.zone(zone).get_mut(group)? else {
                    return None;
                };
                match members.remove(at) {
                    GroupEntry::Panel(id) => Some(BarItem::Panel { id, side: None }),
                    GroupEntry::Separator => None,
                }
            }
        }
    }

    /// The contribution the user may place: a user plugin's one only after its package was
    /// approved for placement.
    fn contribution(&self, id: &ContributionId) -> Result<ContributionRef<'a>, EditError> {
        self.catalog
            .contribution(id)
            .filter(|_| self.catalog.is_placeable(id))
            .ok_or_else(|| EditError::Unknown(id.to_string()))
    }

    /// A group of the catalog, or one the user made that is in the bar.
    fn check_group(&self, id: &GroupId) -> Result<(), EditError> {
        if self.catalog.group(id.as_str()).is_some() || self.group_at(id).is_some() {
            Ok(())
        } else {
            Err(EditError::Unknown(id.to_string()))
        }
    }

    /// What a contribution becomes as an entry of a zone: a widget, or a panel on its own that
    /// keeps the side it had when `taken` was already one.
    fn zone_entry(
        &self,
        id: &ContributionId,
        taken: Option<BarItem>,
    ) -> Result<BarItem, EditError> {
        Ok(match self.contribution(id)?.kind() {
            ContributionKind::Widget => BarItem::Item { id: id.clone() },
            ContributionKind::Panel => match taken {
                Some(panel @ BarItem::Panel { .. }) => panel,
                _ => BarItem::Panel {
                    id: id.clone(),
                    side: None,
                },
            },
        })
    }

    fn shift(&mut self, subject: &Subject, by: i32) -> Result<(), EditError> {
        match self.find(subject) {
            Some(Spot::Zone(zone, at)) => shift_in(self.zone(zone), at, by),
            Some(Spot::Member(zone, group, at)) => {
                if let Some(BarItem::Group { members, .. }) = self.zone(zone).get_mut(group) {
                    shift_in(members, at, by);
                }
            }
            None => return Err(EditError::NotPlaced(subject.clone())),
        }
        Ok(())
    }

    fn put_in_zone(&mut self, subject: &Subject, zone: Zone) -> Result<(), EditError> {
        let spot = self.find(subject);
        match subject {
            Subject::Contribution(id) => {
                self.contribution(id)?;
            }
            Subject::Group(id) => {
                self.check_group(id)?;
                if spot.is_none() {
                    return Err(EditError::NotPlaced(subject.clone()));
                }
            }
        }
        let staying = matches!(spot, Some(Spot::Zone(held, _)) if held == zone);
        if self.zone(zone).len() >= MAX_ZONE_ITEMS && !staying {
            return Err(EditError::Full(Limit::ZoneItems));
        }
        let taken = spot.and_then(|spot| self.take(spot));
        let item = match subject {
            Subject::Contribution(id) => {
                self.preset.tray.retain(|held| held != id);
                self.zone_entry(id, taken)?
            }
            Subject::Group(_) => taken.ok_or_else(|| EditError::NotPlaced(subject.clone()))?,
        };
        self.zone(zone).push(item);
        Ok(())
    }

    fn put_in_group(&mut self, panel: &ContributionId, group: &GroupId) -> Result<(), EditError> {
        let spec = self.contribution(panel)?;
        let subject = Subject::Contribution(panel.clone());
        if spec.kind() != ContributionKind::Panel {
            return Err(EditError::WrongKind(subject));
        }
        self.check_group(group)?;
        if let Some(spot) = self.find(&subject) {
            self.take(spot);
        }
        if self.group_at(group).is_none() && self.catalog.group(group.as_str()).is_some() {
            let zone = spec.placement().zone;
            if self.zone(zone).len() >= MAX_ZONE_ITEMS {
                return Err(EditError::Full(Limit::ZoneItems));
            }
            let primary = self
                .catalog
                .group(group.as_str())
                .is_some_and(|g| g.primary);
            self.zone(zone).push(BarItem::Group {
                id: group.clone(),
                name: None,
                side: None,
                primary,
                members: Vec::new(),
            });
        }
        self.preset.tray.retain(|held| held != panel);
        let Some(members) = self.members_of(group) else {
            return Err(EditError::NotPlaced(Subject::Group(group.clone())));
        };
        if members.len() >= MAX_GROUP_MEMBERS {
            return Err(EditError::Full(Limit::GroupMembers));
        }
        members.push(GroupEntry::Panel(panel.clone()));
        Ok(())
    }

    fn move_to(&mut self, subject: &Subject, place: &Slot, index: usize) -> Result<(), EditError> {
        let spot = self.find(subject);
        self.check_landing(subject, place, spot)?;
        let leaves = match (spot, place) {
            (Some(Spot::Zone(held, at)), Slot::Zone(zone)) if held == *zone => Some(at),
            (Some(Spot::Member(zone, group, at)), Slot::Group(wanted))
                if matches!(
                    self.zone(zone).get(group),
                    Some(BarItem::Group { id, .. }) if id == wanted
                ) =>
            {
                Some(at)
            }
            _ => None,
        };
        if leaves.is_none() {
            self.check_room(place)?;
        }
        // The entry's own place disappears before it lands, which shifts what follows it.
        let index = match leaves {
            Some(at) if at < index => index - 1,
            _ => index,
        };
        let taken = spot.and_then(|spot| self.take(spot));
        if let Subject::Contribution(id) = subject {
            self.preset.tray.retain(|held| held != id);
        }
        match (place, subject) {
            (Slot::Zone(zone), Subject::Contribution(id)) => {
                let entry = self.zone_entry(id, taken)?;
                insert_at(self.zone(*zone), index, entry);
            }
            (Slot::Zone(zone), Subject::Group(_)) => {
                let group = taken.ok_or_else(|| EditError::NotPlaced(subject.clone()))?;
                insert_at(self.zone(*zone), index, group);
            }
            (Slot::Group(group), Subject::Contribution(id)) => {
                let members = self
                    .members_of(group)
                    .ok_or_else(|| EditError::NotPlaced(Subject::Group(group.clone())))?;
                insert_at(members, index, GroupEntry::Panel(id.clone()));
            }
            (Slot::Group(_), Subject::Group(_)) => {
                return Err(EditError::WrongKind(subject.clone()));
            }
        }
        Ok(())
    }

    /// Whether `subject` can land in `place`: a widget, a panel or a group in a zone, a panel in
    /// a group of the bar.
    fn check_landing(
        &self,
        subject: &Subject,
        place: &Slot,
        spot: Option<Spot>,
    ) -> Result<(), EditError> {
        match (subject, place) {
            (Subject::Contribution(id), Slot::Zone(_)) => {
                self.contribution(id)?;
            }
            (Subject::Group(id), Slot::Zone(_)) => {
                self.check_group(id)?;
                if spot.is_none() {
                    return Err(EditError::NotPlaced(subject.clone()));
                }
            }
            (Subject::Contribution(id), Slot::Group(group)) => {
                if self.contribution(id)?.kind() != ContributionKind::Panel {
                    return Err(EditError::WrongKind(subject.clone()));
                }
                self.check_group(group)?;
                if self.group_at(group).is_none() {
                    return Err(EditError::NotPlaced(Subject::Group(group.clone())));
                }
            }
            (Subject::Group(_), Slot::Group(_)) => {
                return Err(EditError::WrongKind(subject.clone()));
            }
        }
        Ok(())
    }

    /// Whether the list `place` names has room for one more entry.
    fn check_room(&mut self, place: &Slot) -> Result<(), EditError> {
        match place {
            Slot::Zone(zone) if self.zone(*zone).len() >= MAX_ZONE_ITEMS => {
                Err(EditError::Full(Limit::ZoneItems))
            }
            Slot::Group(group)
                if self
                    .members_of(group)
                    .is_some_and(|members| members.len() >= MAX_GROUP_MEMBERS) =>
            {
                Err(EditError::Full(Limit::GroupMembers))
            }
            _ => Ok(()),
        }
    }

    fn insert_separator(&mut self, place: &Slot, index: usize) -> Result<(), EditError> {
        match place {
            Slot::Zone(zone) => {
                let items = self.zone(*zone);
                if items.len() >= MAX_ZONE_ITEMS {
                    return Err(EditError::Full(Limit::ZoneItems));
                }
                items.insert(index.min(items.len()), BarItem::Separator);
            }
            Slot::Group(group) => {
                let members = self
                    .members_of(group)
                    .ok_or_else(|| EditError::NotPlaced(Subject::Group(group.clone())))?;
                if members.len() >= MAX_GROUP_MEMBERS {
                    return Err(EditError::Full(Limit::GroupMembers));
                }
                members.insert(index.min(members.len()), GroupEntry::Separator);
            }
        }
        Ok(())
    }

    fn remove_separator(&mut self, place: &Slot, index: usize) -> Result<(), EditError> {
        match place {
            Slot::Zone(zone) => {
                let items = self.zone(*zone);
                if !matches!(items.get(index), Some(BarItem::Separator)) {
                    return Err(EditError::NotASeparator);
                }
                items.remove(index);
            }
            Slot::Group(group) => {
                let members = self
                    .members_of(group)
                    .ok_or_else(|| EditError::NotPlaced(Subject::Group(group.clone())))?;
                if !matches!(members.get(index), Some(GroupEntry::Separator)) {
                    return Err(EditError::NotASeparator);
                }
                members.remove(index);
            }
        }
        Ok(())
    }

    fn remove_to_tray(&mut self, id: &ContributionId) -> Result<(), EditError> {
        if self.contribution(id)?.pinned() {
            return Err(EditError::Pinned(id.clone()));
        }
        if let Some(spot) = self.find(&Subject::Contribution(id.clone())) {
            self.take(spot);
        }
        if !self.preset.tray.contains(id) {
            self.preset.tray.push(id.clone());
        }
        self.preset.open.retain(|open| open != id);
        Ok(())
    }

    fn restore(&mut self, id: &ContributionId) -> Result<(), EditError> {
        let spec = self.contribution(id)?;
        let subject = Subject::Contribution(id.clone());
        if self.find(&subject).is_some() {
            return Ok(());
        }
        match spec.kind() {
            ContributionKind::Widget => self.put_in_zone(&subject, spec.placement().zone),
            ContributionKind::Panel => {
                let group = hinted_group(self.catalog, spec)
                    .ok_or(EditError::WrongKind(subject))?
                    .id
                    .clone();
                self.put_in_group(id, &group)
            }
        }
    }

    fn place_in(&mut self, id: &ContributionId, zone: Zone) -> Result<(), EditError> {
        let spec = self.contribution(id)?;
        let subject = Subject::Contribution(id.clone());
        if self.find(&subject).is_some() {
            return Ok(());
        }
        match spec.kind() {
            ContributionKind::Widget => self.put_in_zone(&subject, zone),
            ContributionKind::Panel => {
                let group = hinted_group(self.catalog, spec)
                    .ok_or(EditError::WrongKind(subject))?
                    .id
                    .clone();
                let joined = self.group_at(&group).is_some();
                self.put_in_group(id, &group)?;
                if joined {
                    return Ok(());
                }
                self.put_in_zone(&Subject::Group(group), zone)
            }
        }
    }

    fn forget(&mut self, id: &ContributionId) -> Result<(), EditError> {
        if self.catalog.contribution(id).is_some() {
            return Err(EditError::Installed(id.clone()));
        }
        self.preset.tray.retain(|held| held != id);
        Ok(())
    }

    fn set_open(&mut self, panel: &ContributionId, open: bool) -> Result<(), EditError> {
        let subject = Subject::Contribution(panel.clone());
        let (zone, group) = match self.find(&subject) {
            Some(Spot::Member(zone, group, _)) => (zone, group),
            Some(Spot::Zone(zone, at))
                if matches!(self.zone(zone).get(at), Some(BarItem::Panel { .. })) =>
            {
                (zone, at)
            }
            _ => return Err(EditError::NotPlaced(subject)),
        };
        let siblings: Vec<ContributionId> = match self.zone(zone).get(group) {
            Some(BarItem::Panel { id, .. }) => vec![id.clone()],
            Some(BarItem::Group { members, .. }) => members
                .iter()
                .filter_map(|member| match member {
                    GroupEntry::Panel(id) => Some(id.clone()),
                    GroupEntry::Separator => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        self.preset.open.retain(|held| !siblings.contains(held));
        if open {
            self.preset.open.push(panel.clone());
        }
        Ok(())
    }

    fn set_side(&mut self, group: &GroupId, side: Option<Side>) -> Result<(), EditError> {
        let solo = ZONES.iter().find_map(|zone| {
            let at = self.preset.bar.zone(*zone).iter().position(
                |item| matches!(item, BarItem::Panel { id, .. } if solo_group_id(id) == *group),
            )?;
            Some((*zone, at))
        });
        let entry = match solo {
            Some((zone, at)) => self.zone(zone).get_mut(at),
            None => self.group_mut(group),
        };
        match entry {
            Some(BarItem::Group { side: held, .. } | BarItem::Panel { side: held, .. }) => {
                *held = side;
                Ok(())
            }
            _ => Err(EditError::NotPlaced(Subject::Group(group.clone()))),
        }
    }

    /// The panel `id` of the catalog, which may be placed.
    fn panel(&self, id: &ContributionId) -> Result<(), EditError> {
        if self.contribution(id)?.kind() == ContributionKind::Panel {
            Ok(())
        } else {
            Err(EditError::WrongKind(Subject::Contribution(id.clone())))
        }
    }

    fn group_with(
        &mut self,
        panel: &ContributionId,
        target: &ContributionId,
    ) -> Result<(), EditError> {
        self.panel(panel)?;
        self.panel(target)?;
        if panel == target {
            return Ok(());
        }
        let aim = Subject::Contribution(target.clone());
        match self.find(&aim) {
            Some(Spot::Member(zone, group, at)) => {
                let Some(BarItem::Group { id, .. }) = self.zone(zone).get(group) else {
                    return Err(EditError::NotPlaced(aim));
                };
                let place = Slot::Group(id.clone());
                self.move_to(&Subject::Contribution(panel.clone()), &place, at + 1)
            }
            Some(Spot::Zone(..)) => {
                if let Some(spot) = self.find(&Subject::Contribution(panel.clone())) {
                    self.take(spot);
                }
                self.preset.tray.retain(|held| held != panel);
                let Some(Spot::Zone(zone, at)) = self.find(&aim) else {
                    return Err(EditError::NotPlaced(aim));
                };
                let side = match self.zone(zone).get(at) {
                    Some(BarItem::Panel { side, .. }) => *side,
                    _ => None,
                };
                let group = self.made_group(
                    side,
                    vec![
                        GroupEntry::Panel(target.clone()),
                        GroupEntry::Panel(panel.clone()),
                    ],
                );
                if let Some(entry) = self.zone(zone).get_mut(at) {
                    *entry = group;
                }
                Ok(())
            }
            None => Err(EditError::NotPlaced(aim)),
        }
    }

    fn new_group(&mut self, panel: &ContributionId) -> Result<(), EditError> {
        self.panel(panel)?;
        let subject = Subject::Contribution(panel.clone());
        match self.find(&subject) {
            Some(Spot::Zone(zone, at)) => {
                let side = match self.zone(zone).get(at) {
                    Some(BarItem::Panel { side, .. }) => *side,
                    _ => return Err(EditError::WrongKind(subject)),
                };
                let group = self.made_group(side, vec![GroupEntry::Panel(panel.clone())]);
                if let Some(entry) = self.zone(zone).get_mut(at) {
                    *entry = group;
                }
            }
            Some(spot @ Spot::Member(zone, group, _)) => {
                if self.zone(zone).len() >= MAX_ZONE_ITEMS {
                    return Err(EditError::Full(Limit::ZoneItems));
                }
                self.take(spot);
                let made = self.made_group(None, vec![GroupEntry::Panel(panel.clone())]);
                insert_at(self.zone(zone), group + 1, made);
            }
            None => return Err(EditError::NotPlaced(subject)),
        }
        Ok(())
    }

    fn ungroup(&mut self, group: &GroupId) -> Result<(), EditError> {
        if self
            .catalog
            .group(group.as_str())
            .is_some_and(|spec| spec.primary)
        {
            return Err(EditError::PrimaryGroup);
        }
        let subject = Subject::Group(group.clone());
        let Some((zone, at)) = self.group_at(group) else {
            return Err(EditError::NotPlaced(subject));
        };
        let BarItem::Group { side, members, .. } = self.zone(zone).remove(at) else {
            return Err(EditError::NotPlaced(subject));
        };
        let panels: Vec<BarItem> = members
            .into_iter()
            .filter_map(|member| match member {
                GroupEntry::Panel(id) => Some(BarItem::Panel { id, side }),
                GroupEntry::Separator => None,
            })
            .collect();
        let items = self.zone(zone);
        if items.len() + panels.len() > MAX_ZONE_ITEMS {
            return Err(EditError::Full(Limit::ZoneItems));
        }
        items.splice(at..at, panels);
        Ok(())
    }

    fn rename_group(&mut self, group: &GroupId, name: &str) -> Result<(), EditError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NameError::Empty.into());
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(NameError::TooLong.into());
        }
        match self.group_mut(group) {
            Some(BarItem::Group { name: held, .. }) => {
                *held = Some(name.to_owned());
                Ok(())
            }
            _ => Err(EditError::NotPlaced(Subject::Group(group.clone()))),
        }
    }

    /// A new group the user made, with a fresh id and name, holding `members`.
    fn made_group(&self, side: Option<Side>, members: Vec<GroupEntry>) -> BarItem {
        let taken: Vec<(&GroupId, Option<&str>)> = ZONES
            .iter()
            .flat_map(|zone| self.preset.bar.zone(*zone))
            .filter_map(|item| match item {
                BarItem::Group { id, name, .. } => Some((id, name.as_deref())),
                _ => None,
            })
            .collect();
        let n = (1..)
            .find(|n| {
                let id = format!("group-{n}");
                let name = format!("Group {n}");
                !taken.iter().any(|(held, named)| {
                    held.as_str() == id || named.is_some_and(|named| same_name(named, &name))
                }) && self.catalog.group(&id).is_none()
            })
            .unwrap_or(1);
        BarItem::Group {
            id: GroupId::owned(format!("group-{n}")),
            name: Some(format!("Group {n}")),
            side,
            primary: false,
            members,
        }
    }
}

fn spot_in(zone: Zone, at: usize, item: &BarItem, subject: &Subject) -> Option<Spot> {
    match (item, subject) {
        (BarItem::Item { id } | BarItem::Panel { id, .. }, Subject::Contribution(wanted))
            if id == wanted =>
        {
            Some(Spot::Zone(zone, at))
        }
        (BarItem::Group { id, .. }, Subject::Group(wanted)) if id == wanted => {
            Some(Spot::Zone(zone, at))
        }
        (BarItem::Group { members, .. }, Subject::Contribution(wanted)) => members
            .iter()
            .position(|member| matches!(member, GroupEntry::Panel(id) if id == wanted))
            .map(|member| Spot::Member(zone, at, member)),
        _ => None,
    }
}

/// Puts `entry` before the one at `index`, or last when `index` is past the end.
fn insert_at<T>(list: &mut Vec<T>, index: usize, entry: T) {
    list.insert(index.min(list.len()), entry);
}

/// Moves the entry at `from` by `by` places, stopping at the ends.
fn shift_in<T>(list: &mut Vec<T>, from: usize, by: i32) {
    let last = list.len().saturating_sub(1);
    let step = usize::try_from(by.unsigned_abs()).unwrap_or(usize::MAX);
    let to = if by < 0 {
        from.saturating_sub(step)
    } else {
        from.saturating_add(step).min(last)
    };
    let entry = list.remove(from);
    list.insert(to, entry);
}
