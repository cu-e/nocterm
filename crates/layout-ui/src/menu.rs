//! What one item of the bar offers in its menu: move it, group it, separate it, hide it or add
//! it to the bar. The list is plain data, so the rows, the preview and the tray build the same
//! menu and a test reads it without opening one.

use std::rc::Rc;

use gpui_kit::{
    Context, SharedString, Window,
    component::menu::{PopupMenu, PopupMenuItem},
};
use nocterm_plugin_model::{ContributionId, ContributionKind, PluginSet, Zone};
use nocterm_window_layout::{EditOp, Subject};

use crate::{commands::Commands, snapshot::Snapshot};

const ZONES: [(Zone, &str); 3] = [
    (Zone::Left, "Left"),
    (Zone::Center, "Centre"),
    (Zone::Right, "Right"),
];

/// One line of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// Sends `op`; greyed out when `enabled` is false.
    Do {
        label: String,
        op: EditOp,
        enabled: bool,
    },
    Submenu {
        label: String,
        entries: Vec<Entry>,
    },
    Rule,
}

/// The menu of one subject, drawn from the layout as it is now.
pub(crate) struct ItemMenu {
    entries: Vec<Entry>,
    commands: Commands,
}

impl ItemMenu {
    pub(crate) fn new(
        subject: &Subject,
        snapshot: &Snapshot,
        catalog: &PluginSet,
        commands: Commands,
    ) -> Rc<Self> {
        Rc::new(Self {
            entries: entries(subject, snapshot, catalog),
            commands,
        })
    }

    /// Fills a popup with the entries; a click sends the operation to the section.
    pub(crate) fn popup(
        &self,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        fill(menu, &self.entries, &self.commands, window, cx)
    }
}

fn fill(
    mut menu: PopupMenu,
    entries: &[Entry],
    commands: &Commands,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for entry in entries {
        menu = match entry {
            Entry::Rule => menu.separator(),
            Entry::Do { label, op, enabled } => {
                let (commands, op) = (commands.clone(), op.clone());
                menu.item(
                    PopupMenuItem::new(SharedString::from(label.clone()))
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| {
                            let op = op.clone();
                            commands.run(window, cx, move |section, _, cx| section.edit(op, cx));
                        }),
                )
            }
            Entry::Submenu { label, entries } => {
                let (commands, entries) = (commands.clone(), Rc::new(entries.clone()));
                menu.submenu(label.clone(), window, cx, move |submenu, window, cx| {
                    fill(submenu, &entries, &commands, window, cx)
                })
            }
        };
    }
    menu
}

/// The entries for `subject`: the ones for an item in the bar, or for one waiting in the tray.
pub(crate) fn entries(subject: &Subject, snapshot: &Snapshot, catalog: &PluginSet) -> Vec<Entry> {
    let editable = snapshot.editable();
    let run = |label: &str, op: EditOp, enabled: bool| Entry::Do {
        label: label.to_owned(),
        op,
        enabled: enabled && editable,
    };
    let Some(zone) = snapshot.zone_of(subject) else {
        return unplaced(subject, snapshot, catalog, &run);
    };
    let panel = match subject {
        Subject::Contribution(id) => is_panel(id, catalog).then_some(id),
        Subject::Group(_) => None,
    };
    let mut entries = Vec::new();
    // A panel changes zone with its group.
    let carried = match panel {
        Some(id) => snapshot
            .group_of(id)
            .map(|group| Subject::Group(group.id().clone())),
        None => Some(subject.clone()),
    };
    if let Some(carried) = carried {
        let noun = if panel.is_some() { "group " } else { "" };
        for (to, name) in ZONES.into_iter().filter(|(to, _)| *to != zone) {
            let op = EditOp::ToZone {
                subject: carried.clone(),
                zone: to,
            };
            entries.push(run(&format!("Move {noun}to {name}"), op, true));
        }
    }
    if let Some(entry) = panel.and_then(|id| move_into_group(id, snapshot, editable)) {
        entries.push(entry);
    }
    entries.push(Entry::Rule);
    for (label, by) in [("Move up", -1), ("Move down", 1)] {
        let op = EditOp::Move {
            subject: subject.clone(),
            by,
        };
        entries.push(run(label, op, snapshot.can_shift(subject, by)));
    }
    entries.push(Entry::Rule);
    let places = [
        ("Add separator before", snapshot.before(subject)),
        ("Add separator after", snapshot.after(subject)),
    ];
    for (label, at) in places {
        if let Some((place, index)) = at {
            entries.push(run(label, EditOp::InsertSeparator { place, index }, true));
        }
    }
    if let Subject::Contribution(id) = subject
        && catalog.contribution(id).is_some_and(|spec| !spec.pinned())
    {
        entries.push(Entry::Rule);
        let op = EditOp::RemoveToTray { id: id.clone() };
        entries.push(run("Hide from the bar", op, true));
    }
    entries
}

fn is_panel(id: &ContributionId, catalog: &PluginSet) -> bool {
    catalog
        .contribution(id)
        .is_some_and(|spec| spec.kind() == ContributionKind::Panel)
}

/// "Move into group", with the groups other than the panel's own.
fn move_into_group(panel: &ContributionId, snapshot: &Snapshot, editable: bool) -> Option<Entry> {
    let current = snapshot.group_of(panel).map(|group| group.id().clone());
    let entries = snapshot
        .groups()
        .iter()
        .filter(|group| Some(&group.id) != current.as_ref())
        .map(|group| Entry::Do {
            label: group.title.to_owned(),
            op: EditOp::ToGroup {
                panel: panel.clone(),
                group: group.id.clone(),
            },
            enabled: editable,
        })
        .collect::<Vec<_>>();
    (!entries.is_empty()).then(|| Entry::Submenu {
        label: "Move into group".to_owned(),
        entries,
    })
}

/// The entries for a contribution that is not in the bar.
///
/// A widget can be added to any zone. A panel is added to the group its hints name; when that
/// group is not in the bar either, the group comes with it into any zone.
fn unplaced(
    subject: &Subject,
    snapshot: &Snapshot,
    catalog: &PluginSet,
    run: &impl Fn(&str, EditOp, bool) -> Entry,
) -> Vec<Entry> {
    let Subject::Contribution(id) = subject else {
        return Vec::new();
    };
    let host = snapshot
        .layout()
        .host_of(id)
        .filter(|_| is_panel(id, catalog));
    if let Some(group) = host.filter(|group| snapshot.zone_of_group(group).is_some()) {
        let title = snapshot
            .groups()
            .iter()
            .find(|spec| spec.id == *group)
            .map_or("its group", |spec| spec.title);
        let op = EditOp::Restore { id: id.clone() };
        return vec![run(&format!("Add to {title}"), op, true)];
    }
    let zones = ZONES
        .into_iter()
        .map(|(zone, name)| {
            let op = EditOp::PlaceIn {
                id: id.clone(),
                zone,
            };
            run(name, op, true)
        })
        .collect();
    vec![Entry::Submenu {
        label: "Add to bar".to_owned(),
        entries: zones,
    }]
}
