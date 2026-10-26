//! Bar entries of plugins the run cannot show: a run that holds user plugins knows none, but its
//! layouts may name them, and an edit leaves them where they are.

use std::collections::HashSet;

use nocterm_plugin_model::{ContributionId, PluginSet, Zone};

use crate::{Bar, BarItem, GroupEntry, resolve::ZONES};

/// Appends to `settled` the entries of `stored` that `catalog` keeps as unavailable.
///
/// A widget returns to its zone and a panel to its group, in their old order, after the entries
/// that resolved. They go last so that the position an editor passes for a visible entry is
/// still its position in the preset. A group that only such panels held is written back with
/// just them. Each id is kept once, the first mention.
pub(super) fn keep_unavailable(settled: &mut Bar, stored: &Bar, catalog: &PluginSet) {
    let mut seen: HashSet<&ContributionId> = HashSet::new();
    for zone in ZONES {
        for item in stored.zone(zone) {
            match item {
                BarItem::Item { id } if catalog.keeps_unavailable(id) && seen.insert(id) => {
                    zone_mut(settled, zone).push(BarItem::Item { id: id.clone() });
                }
                BarItem::Group {
                    id,
                    side,
                    primary,
                    members,
                } => {
                    let kept: Vec<GroupEntry> = members
                        .iter()
                        .filter_map(|member| match member {
                            GroupEntry::Panel(panel)
                                if catalog.keeps_unavailable(panel) && seen.insert(panel) =>
                            {
                                Some(GroupEntry::Panel(panel.clone()))
                            }
                            _ => None,
                        })
                        .collect();
                    if !kept.is_empty() {
                        rejoin(settled, zone, id, (*side, *primary), kept);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Puts `kept` at the end of the group `id` wherever it is in `bar`, or writes the group back
/// into `zone` when nothing else kept it.
fn rejoin(
    bar: &mut Bar,
    zone: Zone,
    id: &nocterm_plugin_model::GroupId,
    look: (Option<nocterm_plugin_model::Side>, bool),
    kept: Vec<GroupEntry>,
) {
    for at in ZONES {
        for item in zone_mut(bar, at) {
            if let BarItem::Group {
                id: group, members, ..
            } = item
                && group == id
            {
                members.extend(kept);
                return;
            }
        }
    }
    zone_mut(bar, zone).push(BarItem::Group {
        id: id.clone(),
        side: look.0,
        primary: look.1,
        members: kept,
    });
}

fn zone_mut(bar: &mut Bar, zone: Zone) -> &mut Vec<BarItem> {
    match zone {
        Zone::Left => &mut bar.left,
        Zone::Center => &mut bar.center,
        Zone::Right => &mut bar.right,
    }
}
