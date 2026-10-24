//! What the status bar draws, resolved against the window's state.

use gpui_kit::{AnyView, App, SharedString};
use nocterm_plugin_model::{ContributionId, PluginCatalog, Zone};
use nocterm_window_layout::{ResolvedGroup, ResolvedItem, ResolvedLayout, ResolvedMember};

use super::StatusBar;
use crate::{
    ContextSelectionReceipt,
    plugin::{LOCAL_TERMINAL, NOTIFICATIONS},
    workspace::{
        chrome::{Regions, toggle_look::ToggleLook},
        panel_groups::{GroupKey, PanelGroups},
    },
};

/// The toggle of one panel.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct PanelToggle {
    id: ContributionId,
    group: GroupKey,
    ix: usize,
    look: ToggleLook,
    badge: Option<SharedString>,
    selected: bool,
}

impl PanelToggle {
    pub(in crate::workspace) fn id(&self) -> &ContributionId {
        &self.id
    }
    pub(in crate::workspace) fn group(&self) -> GroupKey {
        self.group
    }
    /// The panel's position in its group, which names the toggle's element. A group lists the
    /// panels the layout places in layout order and the tray after them, so the placed toggles
    /// are numbered from 0 without gaps.
    pub(in crate::workspace) fn ix(&self) -> usize {
        self.ix
    }
    pub(in crate::workspace) fn look(&self) -> &ToggleLook {
        &self.look
    }
    pub(in crate::workspace) fn badge(&self) -> Option<&SharedString> {
        self.badge.as_ref()
    }
    pub(in crate::workspace) fn is_selected(&self) -> bool {
        self.selected
    }
}

/// One thing in a zone of the bar.
#[derive(Clone)]
pub(in crate::workspace) enum BarEntry {
    Panel(PanelToggle),
    /// The notice bar, which the shell draws itself.
    Notices,
    /// The toggle of the local terminal dock, which the shell draws itself.
    LocalTerminal(ToggleLook),
    /// A registered widget's view.
    Widget(AnyView),
    Separator,
}

/// The entries of the three zones, with the state the entries need.
pub(in crate::workspace) struct BarInputs {
    left: Vec<BarEntry>,
    center: Vec<BarEntry>,
    right: Vec<BarEntry>,
    local_terminal_open: bool,
    selection: ContextSelectionReceipt,
}

impl BarInputs {
    /// Resolves the arrangement in force for the window's panels and widgets.
    ///
    /// - A panel's toggle is drawn while the host offers the panel.
    /// - A panel that the arrangement does not place is drawn at the inner end of the right zone
    ///   while it asks for the user.
    pub(in crate::workspace) fn new(regions: &Regions<'_>, status: &StatusBar, cx: &App) -> Self {
        let layout = regions.shell.current(cx);
        let resolver = Resolver {
            layout,
            groups: regions.groups,
            status,
            catalog: regions.catalog,
            cx,
        };
        let mut right = resolver.zone(Zone::Right);
        let unplaced = resolver.unplaced_asking();
        right.splice(0..0, unplaced);
        Self {
            left: tidy(resolver.zone(Zone::Left)),
            center: tidy(resolver.zone(Zone::Center)),
            right: tidy(right),
            local_terminal_open: regions.tabs.local_visible(),
            selection: regions.selection,
        }
    }

    pub(in crate::workspace) fn left(&self) -> &[BarEntry] {
        &self.left
    }
    pub(in crate::workspace) fn center(&self) -> &[BarEntry] {
        &self.center
    }
    pub(in crate::workspace) fn right(&self) -> &[BarEntry] {
        &self.right
    }
    pub(in crate::workspace) fn local_terminal_open(&self) -> bool {
        self.local_terminal_open
    }
    pub(in crate::workspace) fn selection(&self) -> ContextSelectionReceipt {
        self.selection
    }

    /// The looks of the toggles the bar draws.
    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::workspace) fn looks(&self) -> ToggleLooks {
        let entries = || {
            [&self.left, &self.center, &self.right]
                .into_iter()
                .flatten()
        };
        let panels = || {
            entries().filter_map(|entry| match entry {
                BarEntry::Panel(toggle) => Some(toggle),
                _ => None,
            })
        };
        ToggleLooks {
            navigation: panels()
                .filter(|toggle| toggle.group.primary())
                .map(|toggle| toggle.look.clone())
                .collect(),
            local_terminal: entries().find_map(|entry| match entry {
                BarEntry::LocalTerminal(look) => Some(look.clone()),
                _ => None,
            }),
            side_panel: panels()
                .find(|toggle| !toggle.group.primary())
                .map(|toggle| toggle.look.clone()),
        }
    }
}

/// The presentation of the toggles the bar draws, for tests: the sidebar's, the local
/// terminal's and the first of any other group.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToggleLooks {
    pub(crate) navigation: Vec<ToggleLook>,
    pub(crate) local_terminal: Option<ToggleLook>,
    pub(crate) side_panel: Option<ToggleLook>,
}

struct Resolver<'a> {
    layout: &'a ResolvedLayout,
    groups: &'a PanelGroups,
    status: &'a StatusBar,
    catalog: &'a PluginCatalog,
    cx: &'a App,
}

impl Resolver<'_> {
    fn zone(&self, zone: Zone) -> Vec<BarEntry> {
        let mut entries = Vec::new();
        for item in self.layout.zone(zone) {
            match item {
                ResolvedItem::Separator => entries.push(BarEntry::Separator),
                ResolvedItem::Widget(id) => entries.extend(self.widget(id)),
                ResolvedItem::Group(group) => self.group(group, &mut entries),
            }
        }
        entries
    }

    fn widget(&self, id: &ContributionId) -> Option<BarEntry> {
        if *id == NOTIFICATIONS.id {
            return Some(BarEntry::Notices);
        }
        if *id == LOCAL_TERMINAL.id {
            return Some(BarEntry::LocalTerminal(ToggleLook::of(
                &LOCAL_TERMINAL,
                false,
            )));
        }
        self.status.view(id).cloned().map(BarEntry::Widget)
    }

    fn group(&self, group: &ResolvedGroup, entries: &mut Vec<BarEntry>) {
        for member in group.members() {
            match member {
                ResolvedMember::Separator => entries.push(BarEntry::Separator),
                ResolvedMember::Panel(id) => entries.extend(self.toggle(id)),
            }
        }
    }

    fn toggle(&self, id: &ContributionId) -> Option<BarEntry> {
        let (group, ix, panel) = self.groups.offered(id)?;
        let contribution = self.catalog.contribution(id)?;
        Some(BarEntry::Panel(PanelToggle {
            id: id.clone(),
            group,
            ix,
            look: ToggleLook::of(contribution, panel.attention()),
            badge: panel.badge(self.cx),
            selected: self.groups.panel_is_open(id),
        }))
    }

    /// The toggles of offered panels that ask for the user and that the arrangement leaves out.
    fn unplaced_asking(&self) -> Vec<BarEntry> {
        self.groups
            .registered_ids()
            .filter(|id| !self.layout.is_placed(id))
            .filter(|id| {
                self.groups
                    .offered(id)
                    .is_some_and(|(_, _, panel)| panel.attention())
            })
            .filter_map(|id| self.toggle(id))
            .collect()
    }
}

/// Removes separators at the start and the end of the zone and all but the first of a run, as
/// entries that were not drawn can leave them adjacent.
fn tidy(entries: Vec<BarEntry>) -> Vec<BarEntry> {
    let is_separator = |entry: &BarEntry| matches!(entry, BarEntry::Separator);
    let mut kept: Vec<BarEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        if !(is_separator(&entry) && kept.last().is_none_or(is_separator)) {
            kept.push(entry);
        }
    }
    while kept.last().is_some_and(is_separator) {
        kept.pop();
    }
    kept
}
