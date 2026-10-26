//! What one item of the bar offers in its menu: move it, group it, separate it, hide it or add
//! it to the bar. The list is plain data, so the preview and the tray build the same menu and a
//! test reads it without opening one.

use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    App, Context, SharedString, Window,
    component::menu::{PopupMenu, PopupMenuItem},
};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, GroupId, PluginId, PluginSet, SetOwner, Side, Zone,
};
use nocterm_window_layout::{EditOp, ResolvedGroup, Slot, Subject};
use nocterm_workspace::OpenPlugins;

use crate::{commands::Commands, snapshot::Snapshot};

const ZONES: [(Zone, &str); 3] = [
    (Zone::Left, "Left"),
    (Zone::Center, "Centre"),
    (Zone::Right, "Right"),
];

/// One line of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// Sends `op`; greyed out when `enabled` is false, with a check mark when `checked`.
    Do {
        label: String,
        op: EditOp,
        enabled: bool,
        checked: bool,
    },
    /// Asks something that is not an edit of the layout.
    Ask {
        label: String,
        ask: Ask,
        enabled: bool,
    },
    Submenu {
        label: String,
        entries: Vec<Entry>,
    },
    Rule,
}

/// What a menu entry asks for besides an edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Type a new name for the group.
    RenameGroup(GroupId),
    /// Open the plugin on the Plugins page, where its settings are.
    Plugin(PluginId),
}

impl Ask {
    fn run(&self, commands: &Commands, window: &mut Window, cx: &mut App) {
        match self {
            Self::RenameGroup(group) => {
                let (group, commands) = (group.clone(), commands.clone());
                // After the menu has closed and handed the focus back, so the field keeps it.
                window.defer(cx, move |window, cx| {
                    commands.run(window, cx, move |section, window, cx| {
                        section.begin_group_rename(group, window, cx);
                    });
                });
            }
            Self::Plugin(plugin) => {
                let action = OpenPlugins {
                    plugin: Some(plugin.as_str().to_owned().into()),
                };
                window.dispatch_action(Box::new(action), cx);
            }
        }
    }
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
        Self::of(entries(subject, snapshot, catalog), commands)
    }

    pub(crate) fn of(entries: Vec<Entry>, commands: Commands) -> Rc<Self> {
        Rc::new(Self { entries, commands })
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

/// The menu a right click inside a group opens. Menus do not nest, so the group's menu opens on
/// every right click within it; a panel or separator of the group claims that click on the way
/// down, and its own menu opens instead.
#[derive(Clone, Default)]
pub(crate) struct Claim(Rc<Cell<Option<Rc<ItemMenu>>>>);

impl Claim {
    pub(crate) fn claim(&self, menu: Rc<ItemMenu>) {
        self.0.set(Some(menu));
    }

    /// The menu a member claimed for this click, or the group's `own`.
    pub(crate) fn take_or(&self, own: &Rc<ItemMenu>) -> Rc<ItemMenu> {
        self.0.take().unwrap_or_else(|| own.clone())
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
            Entry::Do {
                label,
                op,
                enabled,
                checked,
            } => {
                let (commands, op) = (commands.clone(), op.clone());
                menu.item(
                    PopupMenuItem::new(SharedString::from(label.clone()))
                        .disabled(!enabled)
                        .checked(*checked)
                        .on_click(move |_, window, cx| {
                            let op = op.clone();
                            commands.run(window, cx, move |section, _, cx| section.edit(op, cx));
                        }),
                )
            }
            Entry::Ask {
                label,
                ask,
                enabled,
            } => {
                let (commands, ask) = (commands.clone(), ask.clone());
                menu.item(
                    PopupMenuItem::new(SharedString::from(label.clone()))
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| ask.run(&commands, window, cx)),
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
    let menu = Menu {
        snapshot,
        catalog,
        editable: snapshot.editable(),
    };
    let Some(zone) = snapshot.zone_of(subject) else {
        return menu.unplaced(subject);
    };
    match subject {
        Subject::Group(group) => menu.group(group, zone),
        Subject::Contribution(id) if is_panel(id, catalog) => menu.panel(id, zone),
        Subject::Contribution(id) => menu.widget(id, zone),
    }
}

/// The entries of the separator at `index` of `place`.
pub(crate) fn separator(place: Slot, index: usize, snapshot: &Snapshot) -> Vec<Entry> {
    vec![Entry::Do {
        label: "Remove separator".to_owned(),
        op: EditOp::RemoveSeparator { place, index },
        enabled: snapshot.editable(),
        checked: false,
    }]
}

struct Menu<'a> {
    snapshot: &'a Snapshot,
    catalog: &'a PluginSet,
    editable: bool,
}

impl Menu<'_> {
    fn run(&self, label: impl Into<String>, op: EditOp, enabled: bool) -> Entry {
        Entry::Do {
            label: label.into(),
            op,
            enabled: enabled && self.editable,
            checked: false,
        }
    }

    fn widget(&self, id: &ContributionId, zone: Zone) -> Vec<Entry> {
        let subject = Subject::Contribution(id.clone());
        let mut entries = self.zones(&subject, zone);
        entries.push(Entry::Rule);
        entries.extend(self.shifts(&subject));
        entries.push(Entry::Rule);
        entries.extend(self.separators(&subject));
        entries.extend(self.tail(id));
        entries
    }

    /// A panel on its own moves between zones like a widget; a panel of a group can leave it.
    fn panel(&self, id: &ContributionId, zone: Zone) -> Vec<Entry> {
        let subject = Subject::Contribution(id.clone());
        let group = self.snapshot.group_of(id);
        let mut entries = Vec::new();
        match group.filter(|group| !group.is_solo()) {
            Some(group) => {
                if let Some((place, index)) =
                    self.snapshot.after(&Subject::Group(group.id().clone()))
                {
                    let op = EditOp::MoveTo {
                        subject: subject.clone(),
                        place,
                        index,
                    };
                    entries.push(self.run("Take out of group", op, true));
                }
            }
            None => entries.extend(self.zones(&subject, zone)),
        }
        entries.extend(self.join_group(id));
        let op = EditOp::NewGroup { panel: id.clone() };
        entries.push(self.run("New group", op, true));
        entries.push(Entry::Rule);
        entries.extend(self.shifts(&subject));
        entries.push(Entry::Rule);
        let open = self.snapshot.opens_with_layout(id);
        entries.push(Entry::Do {
            label: "Open with the layout".to_owned(),
            op: EditOp::SetOpen {
                panel: id.clone(),
                open: !open,
            },
            enabled: self.editable,
            checked: open,
        });
        match group {
            Some(group) if group.is_solo() => entries.extend(self.sides(group, zone)),
            Some(group) => entries.push(Entry::Submenu {
                label: format!("Group \u{201c}{}\u{201d}", self.snapshot.group_title(group)),
                entries: self.group(group.id(), zone),
            }),
            None => {}
        }
        entries.push(Entry::Rule);
        entries.extend(self.separators(&subject));
        entries.extend(self.tail(id));
        entries
    }

    fn group(&self, id: &GroupId, zone: Zone) -> Vec<Entry> {
        let subject = Subject::Group(id.clone());
        let mut entries = self.zones(&subject, zone);
        entries.push(Entry::Rule);
        entries.extend(self.shifts(&subject));
        entries.push(Entry::Rule);
        entries.push(Entry::Ask {
            label: "Rename group\u{2026}".to_owned(),
            ask: Ask::RenameGroup(id.clone()),
            enabled: self.editable,
        });
        let group = self.snapshot.group(id);
        if group.is_some_and(|group| !group.is_primary()) {
            let op = EditOp::Ungroup { group: id.clone() };
            entries.push(self.run("Ungroup", op, true));
        }
        if let Some(group) = group {
            entries.extend(self.sides(group, zone));
        }
        entries.push(Entry::Rule);
        entries.extend(self.separators(&subject));
        entries
    }

    /// "Move to" each other zone.
    fn zones(&self, subject: &Subject, zone: Zone) -> Vec<Entry> {
        ZONES
            .into_iter()
            .filter(|(to, _)| *to != zone)
            .map(|(to, name)| {
                let op = EditOp::ToZone {
                    subject: subject.clone(),
                    zone: to,
                };
                self.run(format!("Move to {name}"), op, true)
            })
            .collect()
    }

    fn shifts(&self, subject: &Subject) -> Vec<Entry> {
        [("Move left", -1), ("Move right", 1)]
            .into_iter()
            .map(|(label, by)| {
                let op = EditOp::Move {
                    subject: subject.clone(),
                    by,
                };
                self.run(label, op, self.snapshot.can_shift(subject, by))
            })
            .collect()
    }

    fn separators(&self, subject: &Subject) -> Vec<Entry> {
        [
            ("Add separator before", self.snapshot.before(subject)),
            ("Add separator after", self.snapshot.after(subject)),
        ]
        .into_iter()
        .filter_map(|(label, at)| {
            let (place, index) = at?;
            Some(self.run(label, EditOp::InsertSeparator { place, index }, true))
        })
        .collect()
    }

    /// Where a group in the centre opens; a group in a side zone opens on that side.
    fn sides(&self, group: &ResolvedGroup, zone: Zone) -> Vec<Entry> {
        if zone != Zone::Center {
            return Vec::new();
        }
        let entries = [(Side::Left, "Left"), (Side::Right, "Right")]
            .into_iter()
            .map(|(side, label)| Entry::Do {
                label: label.to_owned(),
                op: EditOp::SetSide {
                    group: group.id().clone(),
                    side: Some(side),
                },
                enabled: self.editable,
                checked: group.side() == side,
            })
            .collect();
        vec![Entry::Submenu {
            label: "Opens on".to_owned(),
            entries,
        }]
    }

    /// "Move into group", with the groups other than the panel's own.
    fn join_group(&self, panel: &ContributionId) -> Option<Entry> {
        let current = self
            .snapshot
            .group_of(panel)
            .map(|group| group.id().clone());
        let entries = self
            .snapshot
            .group_choices()
            .into_iter()
            .filter(|(group, _)| Some(group) != current.as_ref())
            .map(|(group, title)| {
                let op = EditOp::ToGroup {
                    panel: panel.clone(),
                    group,
                };
                self.run(title, op, true)
            })
            .collect::<Vec<_>>();
        (!entries.is_empty()).then(|| Entry::Submenu {
            label: "Move into group".to_owned(),
            entries,
        })
    }

    /// The plugin's settings and hiding, the end of the menu of a contribution.
    fn tail(&self, id: &ContributionId) -> Vec<Entry> {
        let mut entries = self.settings(id);
        if self
            .catalog
            .contribution(id)
            .is_some_and(|spec| !spec.pinned())
        {
            entries.push(Entry::Rule);
            let op = EditOp::RemoveToTray { id: id.clone() };
            entries.push(self.run("Hide from the bar", op, true));
        }
        entries
    }

    /// The entries for a contribution that is not in the bar.
    ///
    /// A widget can be added to any zone. A panel is added to the group its hints name; when
    /// that group is not in the bar either, the group comes with it into any zone.
    fn unplaced(&self, subject: &Subject) -> Vec<Entry> {
        let Subject::Contribution(id) = subject else {
            return Vec::new();
        };
        let host = self
            .snapshot
            .layout()
            .host_of(id)
            .filter(|_| is_panel(id, self.catalog))
            .filter(|group| self.snapshot.zone_of_group(group).is_some());
        let mut entries = match host.and_then(|group| self.snapshot.group(group)) {
            Some(group) => {
                let title = self.snapshot.group_title(group);
                let op = EditOp::Restore { id: id.clone() };
                vec![self.run(format!("Add to {title}"), op, true)]
            }
            None => {
                let zones = ZONES
                    .into_iter()
                    .map(|(zone, name)| {
                        let op = EditOp::PlaceIn {
                            id: id.clone(),
                            zone,
                        };
                        self.run(name, op, true)
                    })
                    .collect();
                vec![Entry::Submenu {
                    label: "Add to bar".to_owned(),
                    entries: zones,
                }]
            }
        };
        entries.extend(self.settings(id));
        entries
    }

    /// The way to the settings of the plugin `id` belongs to, which the Plugins page shows.
    fn settings(&self, id: &ContributionId) -> Vec<Entry> {
        let Some(SetOwner::Plugin(plugin)) = self.catalog.owner(id) else {
            return Vec::new();
        };
        vec![
            Entry::Rule,
            Entry::Ask {
                label: format!("{} settings\u{2026}", plugin.name()),
                ask: Ask::Plugin(plugin.id().clone()),
                enabled: true,
            },
        ]
    }
}

fn is_panel(id: &ContributionId, catalog: &PluginSet) -> bool {
    catalog
        .contribution(id)
        .is_some_and(|spec| spec.kind() == ContributionKind::Panel)
}
