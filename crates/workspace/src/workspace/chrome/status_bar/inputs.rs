//! What the status bar draws, resolved against the window's state.

use gpui_kit::{AnyView, App, ElementId, SharedString};
use nocterm_plugin_model::{ContributionId, GroupId, PluginSet, Zone};
use nocterm_window_layout::{ResolvedGroup, ResolvedItem, ResolvedLayout, ResolvedMember};

use super::{BarOverflow, Pointed, StatusBar};
use crate::{
    ContextSelectionReceipt,
    plugin::{LOCAL_TERMINAL, NOTIFICATIONS},
    workspace::{
        chrome::{Regions, toggle_look::ToggleLook},
        panel_groups::PanelGroups,
    },
};

/// The toggle of one panel.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct PanelToggle {
    id: ContributionId,
    /// Names the toggle's element: the sidebar's toggles by position, any other by its group's
    /// id and position.
    element: ElementId,
    /// Whether the panel is in the sidebar, which tests tell apart from other groups.
    #[cfg(any(test, feature = "test-support"))]
    primary: bool,
    look: ToggleLook,
    badge: Option<SharedString>,
    selected: bool,
    attention: bool,
    user: bool,
}

impl PanelToggle {
    pub(in crate::workspace) fn id(&self) -> &ContributionId {
        &self.id
    }
    /// The toggle's element. A group lists the panels the layout places in layout order and
    /// the tray after them, so the placed toggles of a group are numbered from 0 without gaps.
    pub(in crate::workspace) fn element(&self) -> &ElementId {
        &self.element
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
    /// Whether a user plugin owns the panel, so its toggle is the first to move into the
    /// overflow menu.
    pub(in crate::workspace) fn is_user(&self) -> bool {
        self.user
    }
    /// Whether the panel asks for the user, which keeps its toggle in the bar.
    pub(in crate::workspace) fn has_attention(&self) -> bool {
        self.attention
    }
}

/// One thing in a zone of the bar.
#[derive(Clone)]
pub(in crate::workspace) enum BarEntry {
    Panel(PanelToggle),
    /// The notice bar, which the shell draws itself, in the zone that places it.
    Notices(Zone),
    /// The toggle of the local terminal dock, which the shell draws itself.
    LocalTerminal(ToggleLook),
    /// A registered widget's view.
    Widget(ContributionId, AnyView),
    /// The view of a user plugin's widget, which the shell holds to a narrow, clipped width.
    UserWidget(ContributionId, AnyView),
    Separator,
}

/// How an entry gives way when the bar is too narrow for its zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Shrink {
    /// Never shrinks: it is drawn whole or clipped with its zone.
    Fixed,
    /// Truncates its own content, before any zone clips.
    Elastic,
    /// A user plugin's widget: at most [`USER_WIDGET_MAX_WIDTH`] wide, clipped, and the first to
    /// give way.
    UserPlugin,
}

/// The widest a user plugin's widget is drawn, in pixels.
pub(in crate::workspace) const USER_WIDGET_MAX_WIDTH: f32 = 160.;

impl BarEntry {
    /// The contribution the entry draws; a separator has none.
    pub(in crate::workspace) fn id(&self) -> Option<ContributionId> {
        match self {
            Self::Panel(toggle) => Some(toggle.id().clone()),
            Self::Notices(_) => Some(NOTIFICATIONS.id.clone()),
            Self::LocalTerminal(_) => Some(LOCAL_TERMINAL.id.clone()),
            Self::Widget(id, _) | Self::UserWidget(id, _) => Some(id.clone()),
            Self::Separator => None,
        }
    }

    /// Toggles, widgets and separators keep their size: a bar without room for them moves them
    /// into the overflow menu. The notice line truncates; the bell beside it is not part of what
    /// truncates.
    pub(in crate::workspace) fn shrink(&self) -> Shrink {
        match self {
            Self::Panel(_) | Self::LocalTerminal(_) | Self::Widget(..) | Self::Separator => {
                Shrink::Fixed
            }
            Self::Notices(_) => Shrink::Elastic,
            Self::UserWidget(..) => Shrink::UserPlugin,
        }
    }
}

/// The entries of the three zones, with the state the entries need.
#[derive(Clone)]
pub(in crate::workspace) struct BarInputs {
    left: Vec<BarEntry>,
    center: Vec<BarEntry>,
    right: Vec<BarEntry>,
    /// The entries the bar has no room for, in bar order.
    hidden: Vec<BarEntry>,
    /// Whether the arrangement has a centre, which stays three slots even if the centre's
    /// entries are all hidden.
    centred: bool,
    local_terminal_open: bool,
    selection: ContextSelectionReceipt,
    pointed: Pointed,
    overflow: BarOverflow,
}

impl BarInputs {
    /// Resolves the arrangement in force for the window's panels and widgets.
    ///
    /// - A panel's toggle is drawn while the host offers the panel.
    /// - A panel that the arrangement does not place is drawn at the inner end of the right zone
    ///   while it asks for the user.
    pub(in crate::workspace) fn new(regions: &Regions<'_>, status: &StatusBar, cx: &App) -> Self {
        let layout = regions.shell.current();
        let resolver = Resolver {
            layout,
            groups: regions.groups,
            status,
            set: regions.shell.set(),
            cx,
        };
        let mut right = resolver.zone(Zone::Right);
        let unplaced = resolver.unplaced_asking();
        right.splice(0..0, unplaced);
        let center = tidy(resolver.zone(Zone::Center));
        Self {
            left: tidy(resolver.zone(Zone::Left)),
            centred: !center.is_empty(),
            center,
            right: tidy(right),
            hidden: Vec::new(),
            local_terminal_open: regions.tabs.local_visible(),
            selection: regions.selection,
            pointed: status.pointed().clone(),
            overflow: status.overflow().clone(),
        }
    }

    /// The bar without the entries flagged in `left`, `centre` and `right` (a zone with a
    /// shorter list, or none, hides nothing beyond it), which are kept for the overflow menu.
    pub(super) fn collapsed(mut self, left: &[bool], centre: &[bool], right: &[bool]) -> Self {
        let mut hidden = Vec::new();
        let mut keep = |entries: Vec<BarEntry>, flags: &[bool]| {
            let mut kept = Vec::with_capacity(entries.len());
            for (ix, entry) in entries.into_iter().enumerate() {
                let gone = flags.get(ix).copied().unwrap_or(false);
                match (gone, &entry) {
                    (false, _) => kept.push(entry),
                    (true, BarEntry::Separator) => {}
                    (true, _) => hidden.push(entry),
                }
            }
            tidy(kept)
        };
        self.left = keep(std::mem::take(&mut self.left), left);
        self.center = keep(std::mem::take(&mut self.center), centre);
        self.right = keep(std::mem::take(&mut self.right), right);
        self.hidden = hidden;
        self
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
    /// Whether the bar is drawn as three slots.
    pub(super) fn is_centred(&self) -> bool {
        self.centred
    }
    /// The entries the bar had no room for.
    pub(super) fn hidden(&self) -> &[BarEntry] {
        &self.hidden
    }
    pub(super) fn overflow(&self) -> &BarOverflow {
        &self.overflow
    }
    pub(in crate::workspace) fn local_terminal_open(&self) -> bool {
        self.local_terminal_open
    }
    pub(in crate::workspace) fn pointed(&self) -> &Pointed {
        &self.pointed
    }
    /// Every entry of the three zones.
    pub(in crate::workspace) fn entries(&self) -> impl Iterator<Item = &BarEntry> {
        self.left.iter().chain(&self.center).chain(&self.right)
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
                .filter(|toggle| toggle.primary)
                .map(|toggle| toggle.look.clone())
                .collect(),
            local_terminal: entries().find_map(|entry| match entry {
                BarEntry::LocalTerminal(look) => Some(look.clone()),
                _ => None,
            }),
            side_panel: panels()
                .find(|toggle| !toggle.primary)
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
    set: &'a PluginSet,
    cx: &'a App,
}

impl Resolver<'_> {
    fn zone(&self, zone: Zone) -> Vec<BarEntry> {
        let mut entries = Vec::new();
        for item in self.layout.zone(zone) {
            match item {
                ResolvedItem::Separator => entries.push(BarEntry::Separator),
                ResolvedItem::Widget(id) => entries.extend(self.widget(id, zone)),
                ResolvedItem::Group(group) => self.group(group, &mut entries),
            }
        }
        entries
    }

    fn widget(&self, id: &ContributionId, zone: Zone) -> Option<BarEntry> {
        if *id == NOTIFICATIONS.id {
            return Some(BarEntry::Notices(zone));
        }
        if *id == LOCAL_TERMINAL.id {
            return Some(BarEntry::LocalTerminal(ToggleLook::of_system(
                &LOCAL_TERMINAL,
                false,
            )));
        }
        let view = self.status.view(id).cloned()?;
        let user = self.set.contribution(id).is_some_and(|c| c.is_user());
        Some(if user {
            BarEntry::UserWidget(id.clone(), view)
        } else {
            BarEntry::Widget(id.clone(), view)
        })
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
        let contribution = self.set.contribution(id)?;
        let primary = self.groups.is_primary(group);
        Some(BarEntry::Panel(PanelToggle {
            id: id.clone(),
            element: toggle_id(primary, self.groups.id(group), ix),
            #[cfg(any(test, feature = "test-support"))]
            primary,
            look: match self.set.plugin_of(id).filter(|_| contribution.is_user()) {
                Some(plugin) => {
                    let label = self.set.user_label(plugin.name(), contribution.title());
                    ToggleLook::of_user(contribution, label, panel.attention())
                }
                None => ToggleLook::of(contribution, panel.attention()),
            },
            badge: panel.badge(self.cx),
            selected: self.groups.panel_is_open(id),
            attention: panel.attention(),
            user: contribution.is_user(),
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

/// The sidebar's toggles keep their position as id; any other group names its own.
fn toggle_id(primary: bool, group: &GroupId, ix: usize) -> ElementId {
    if primary {
        ("sidebar-panel", ix).into()
    } else {
        (
            ElementId::from("panel-group-toggle"),
            format!("{group}:{ix}"),
        )
            .into()
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
