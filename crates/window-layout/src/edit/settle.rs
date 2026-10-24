//! Writing a layout out as a preset: the explicit form every entry operation works on.

use std::collections::BTreeSet;

use nocterm_plugin_model::{ContributionId, PluginCatalog, Zone};

use crate::{
    Bar, BarItem, BuiltIn, GroupEntry, LayoutPreset, ResolvedItem, ResolvedLayout, ResolvedMember,
    resolve,
    resolve::{ZONES, default_side},
};

/// A built-in as a preset with every entry written out. The id and name are left empty.
///
/// Every plugin counts as enabled, so the entries of disabled plugins sit where their hints put
/// them and come back with their plugin.
pub(super) fn written_out(built_in: BuiltIn, catalog: &PluginCatalog) -> LayoutPreset {
    explicit(
        &built_in.resolve(catalog, &BTreeSet::new()),
        Vec::new(),
        LayoutPreset::default(),
    )
}

/// The preset as it resolves with every plugin enabled, written out.
///
/// The id, name and contents are the same when the preset was already in this form. Ids the
/// catalog does not know leave the bar for the tray, in bar order after the ones already there.
pub(super) fn settle(preset: &LayoutPreset, catalog: &PluginCatalog) -> LayoutPreset {
    let layout = resolve(preset, catalog, &BTreeSet::new());
    let mut tray: Vec<ContributionId> = Vec::new();
    let held = preset.tray.iter().filter(|id| !layout.is_placed(id));
    let unknown = ZONES
        .iter()
        .flat_map(|zone| preset.bar.zone(*zone))
        .flat_map(ids_of)
        .filter(|id| catalog.contribution(id).is_none());
    for id in held.cloned().chain(unknown) {
        if !tray.contains(&id) {
            tray.push(id);
        }
    }
    explicit(&layout, tray, preset.clone())
}

/// The contribution ids a bar entry names.
fn ids_of(item: &BarItem) -> Vec<ContributionId> {
    match item {
        BarItem::Item { id } => vec![id.clone()],
        BarItem::Separator => Vec::new(),
        BarItem::Group { members, .. } => members
            .iter()
            .filter_map(|member| match member {
                GroupEntry::Panel(id) => Some(id.clone()),
                GroupEntry::Separator => None,
            })
            .collect(),
    }
}

/// The preset that resolves to this layout (with nothing disabled), keeping the fields of
/// `base` that a layout does not hold.
fn explicit(
    layout: &ResolvedLayout,
    tray: Vec<ContributionId>,
    base: LayoutPreset,
) -> LayoutPreset {
    let [left, center, right] = ZONES.map(|zone| zone_of(layout, zone));
    LayoutPreset {
        bar: Bar {
            left,
            center,
            right,
        },
        tray,
        open: layout.open().to_vec(),
        ..base
    }
}

fn zone_of(layout: &ResolvedLayout, zone: Zone) -> Vec<BarItem> {
    layout
        .zone(zone)
        .iter()
        .map(|item| match item {
            ResolvedItem::Widget(id) => BarItem::Item { id: id.clone() },
            ResolvedItem::Separator => BarItem::Separator,
            ResolvedItem::Group(group) => BarItem::Group {
                id: group.id().clone(),
                side: (group.side() != default_side(zone)).then_some(group.side()),
                primary: group.is_primary(),
                members: group
                    .members()
                    .iter()
                    .map(|member| match member {
                        ResolvedMember::Panel(id) => GroupEntry::Panel(id.clone()),
                        ResolvedMember::Separator => GroupEntry::Separator,
                    })
                    .collect(),
            },
        })
        .collect()
}
