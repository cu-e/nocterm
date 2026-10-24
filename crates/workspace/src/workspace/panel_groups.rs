//! The window's side panels, held as groups of panels that exclude each other.
//!
//! Every placed panel belongs to exactly one group; opening a member closes its sibling in
//! the group, and groups are independent of each other. Today's two groups are the
//! navigation sidebar (several panels, one shown at a time) and the assistant (one
//! expandable side panel). [`PanelGroups`] owns their state and every transition between
//! states, and returns a [`PanelTransition`] for the workspace to apply to focus and events.

use std::collections::HashMap;

use gpui_kit::{App, EntityId, Window};
use nocterm_plugin_model::{ContributionId, GroupSpec, Side, WidthClass};
use nocterm_window_layout::ResolvedLayout;

use crate::plugin::GROUPS;

mod group;
mod hosted;
mod transition;

use group::PanelGroup;
pub(super) use hosted::{ExpandableHandle, HostedPanel, Presentation};
pub(super) use transition::{PanelFocus, PanelTransition};

/// The groups of today's shell. Declaration order is the order of `PanelGroups::groups`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum GroupKey {
    /// The sidebar: the primary group, shown even while it has no panel.
    Navigation,
    /// The independent side panel, hidden until its panel is available and opened.
    Assistant,
}

impl GroupKey {
    const ALL: [Self; 2] = [Self::Navigation, Self::Assistant];

    fn index(self) -> usize {
        match self {
            Self::Navigation => 0,
            Self::Assistant => 1,
        }
    }
    /// The shell's declaration of the group; `GROUPS` lists the groups in `ALL` order.
    fn spec(self) -> &'static GroupSpec {
        &GROUPS[self.index()]
    }
    /// Stable name used for the group's column.
    pub(super) fn id(self) -> &'static str {
        self.spec().id.as_str()
    }
    /// The group the shell declares under this id.
    pub(super) fn named(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.id() == id)
    }
    /// The side of the content the group opens on before the user swaps sides.
    pub(super) fn side(self) -> Side {
        match self {
            Self::Navigation => Side::Left,
            Self::Assistant => Side::Right,
        }
    }
    pub(super) fn width_class(self) -> WidthClass {
        match self {
            Self::Navigation => WidthClass::Navigation,
            Self::Assistant => WidthClass::Assistant,
        }
    }
    /// The primary group is visible from the start and never needs a member to be shown.
    pub(super) fn primary(self) -> bool {
        self.spec().primary
    }
}

/// A permanent index into the panel slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PanelKey(u32);

impl PanelKey {
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// A hosted panel and whether the host currently offers it.
struct PanelSlot {
    /// `None` while the panel of a registered contribution is not created.
    panel: Option<HostedPanel>,
    available: bool,
    /// The group the resolved layout hosts the panel in; `None` until the layout is adopted.
    home: Option<GroupKey>,
    /// Whether the resolved layout places the panel in the bar. A panel it leaves out opens by
    /// id but is not part of the sidebar's cycle.
    placed: bool,
}

/// The panels of the window and the groups that hold them.
///
/// The resolved layout is the one source of which group holds a panel and in what order:
/// [`PanelGroups::adopt`] is the only place that sets either.
pub(super) struct PanelGroups {
    groups: Vec<PanelGroup>,
    slots: Vec<PanelSlot>,
    /// The slots of registered panels, by the id of their contribution.
    ids: HashMap<ContributionId, PanelKey>,
}

impl PanelGroups {
    pub(super) fn new() -> Self {
        Self {
            groups: GroupKey::ALL.into_iter().map(PanelGroup::new).collect(),
            slots: Vec::new(),
            ids: HashMap::new(),
        }
    }

    // `groups` holds one entry per `GroupKey`, in declaration order, from `new`.
    fn group(&self, key: GroupKey) -> &PanelGroup {
        &self.groups[key.index()]
    }
    fn group_mut(&mut self, key: GroupKey) -> &mut PanelGroup {
        &mut self.groups[key.index()]
    }
    fn slot(&self, key: Option<PanelKey>) -> Option<&PanelSlot> {
        self.slots.get(key?.index())
    }
    fn selected_slot(&self, key: GroupKey) -> Option<&PanelSlot> {
        self.slot(self.group(key).selected())
    }

    // ── Registration ─────────────────────────────────────────────────────────

    /// Registers a panel of the catalog, available, in the group and at the position the
    /// layout gives it. The panel may be missing; [`PanelGroups::fill`] supplies it. A
    /// registered id is registered once; the caller checks.
    pub(super) fn register(
        &mut self,
        id: ContributionId,
        panel: Option<HostedPanel>,
        layout: &ResolvedLayout,
    ) {
        let key = PanelKey(self.slots.len() as u32);
        self.slots.push(PanelSlot {
            panel,
            available: true,
            home: None,
            placed: false,
        });
        self.ids.insert(id, key);
        self.adopt(layout);
    }

    /// Takes membership, member order and placement of every registered panel from the layout.
    ///
    /// A group holds the panels the layout places in it, in its order, then the panels the bar
    /// leaves out that its hints host there, in registration order. A group keeps its selected
    /// member while the layout still holds it, and otherwise selects its first.
    pub(super) fn adopt(&mut self, layout: &ResolvedLayout) {
        for (id, key) in &self.ids {
            let home = layout
                .host_of(id)
                .and_then(|group| GroupKey::named(group.as_str()));
            if let Some(slot) = self.slots.get_mut(key.index()) {
                slot.home = home;
                slot.placed = layout.group_of(id).is_some();
            }
        }
        for key in GroupKey::ALL {
            let members = self.members_in_layout_order(key, layout);
            self.group_mut(key).arrange(members);
        }
    }

    fn members_in_layout_order(&self, key: GroupKey, layout: &ResolvedLayout) -> Vec<PanelKey> {
        let placed = layout
            .groups()
            .filter(|group| group.id().as_str() == key.id())
            .flat_map(|group| group.panels())
            .filter_map(|id| self.ids.get(id).copied());
        let mut left_out: Vec<PanelKey> = self
            .ids
            .values()
            .copied()
            .filter(|panel| {
                self.slots
                    .get(panel.index())
                    .is_some_and(|slot| slot.home == Some(key) && !slot.placed)
            })
            .collect();
        left_out.sort_by_key(|panel| panel.0);
        placed.chain(left_out).collect()
    }

    /// Supplies the panel of a registered id whose panel is missing. Whether it was.
    pub(super) fn fill(&mut self, id: &ContributionId, panel: HostedPanel) -> bool {
        let Some(key) = self.ids.get(id).copied() else {
            return false;
        };
        match self.slots.get_mut(key.index()) {
            Some(slot) if slot.panel.is_none() => {
                slot.panel = Some(panel);
                true
            }
            _ => false,
        }
    }

    /// Records a changed title, icon or badge. Whether the panel's presentation changed.
    pub(super) fn presentation_changed(
        &mut self,
        panel: EntityId,
        presentation: Presentation,
    ) -> bool {
        self.slots
            .iter_mut()
            .filter_map(|slot| slot.panel.as_mut())
            .find(|hosted| hosted.panel_id() == panel)
            .is_some_and(|hosted| hosted.presentation_changed(presentation))
    }

    /// Records whether a panel asks for the user. Whether that changed.
    pub(super) fn attention_changed(&mut self, panel: EntityId, attention: bool) -> bool {
        self.slots
            .iter_mut()
            .filter_map(|slot| slot.panel.as_mut())
            .find(|hosted| hosted.panel_id() == panel)
            .is_some_and(|hosted| hosted.set_attention(attention))
    }

    // ── Observations ─────────────────────────────────────────────────────────

    /// The group that holds the panel with this entity id.
    pub(super) fn group_holding(&self, panel: EntityId) -> Option<GroupKey> {
        self.slots
            .iter()
            .find(|slot| {
                slot.panel
                    .as_ref()
                    .is_some_and(|hosted| hosted.panel_id() == panel)
            })
            .and_then(|slot| slot.home)
    }

    pub(super) fn is_shown(&self, key: GroupKey) -> bool {
        self.group(key).is_shown()
    }
    pub(super) fn is_expanded(&self, key: GroupKey) -> bool {
        self.group(key).is_expanded()
    }
    /// The shown groups in bar order.
    pub(super) fn shown(&self) -> impl Iterator<Item = GroupKey> + '_ {
        self.groups
            .iter()
            .filter(|group| group.is_shown())
            .map(PanelGroup::key)
    }
    /// The group that fills the body, if any.
    pub(super) fn expanded(&self) -> Option<GroupKey> {
        self.groups
            .iter()
            .find(|group| group.is_expanded())
            .map(PanelGroup::key)
    }
    /// The shown (or last shown) panel of the group.
    pub(super) fn selected(&self, key: GroupKey) -> Option<&HostedPanel> {
        self.selected_slot(key)?.panel.as_ref()
    }
    /// The member `NextPanel` goes to: the one after the selection while the group is shown
    /// (wrapping), else the selection itself, which the action reveals. Only offered panels
    /// that the layout places in the bar take part.
    pub(super) fn next(&self, key: GroupKey) -> Option<usize> {
        let group = self.group(key);
        let members = group.members();
        let start = group.selected_position()?;
        let skip = usize::from(group.is_shown());
        (skip..members.len() + skip)
            .map(|step| (start + step) % members.len())
            .find(|ix| {
                self.slot(members.get(*ix).copied())
                    .is_some_and(|slot| slot.offered() && slot.placed)
            })
    }

    // ── Transitions ──────────────────────────────────────────────────────────

    /// Selects the member at `ix`, shows its group and focuses it. `None` for an unknown
    /// member and for one the host does not offer.
    pub(super) fn activate(
        &mut self,
        key: GroupKey,
        ix: usize,
        cx: &App,
    ) -> Option<PanelTransition> {
        let member = *self.group(key).members().get(ix)?;
        let slot = self.slot(Some(member)).filter(|slot| slot.offered())?;
        let focus = slot.panel.as_ref()?.focus_handle(cx);
        let revealed = self.group_mut(key).select(member);
        Some(PanelTransition::new(
            revealed.then_some(key),
            PanelFocus::Panel(focus),
        ))
    }

    /// Shows a hidden group or hides a shown one.
    ///
    /// The primary group toggles whether or not it has panels. Hiding it moves focus only if
    /// the group held it: to the expanded group, else to the active tab. Any other group
    /// needs an available panel; opening it focuses the panel, and closing it refocuses the
    /// tab only if the panel held focus. `None` when there is nothing to open.
    pub(super) fn toggle(
        &mut self,
        key: GroupKey,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        if key.primary() {
            let held_focus = self.holds_focus(key, window, cx);
            let shown = self.group_mut(key).toggle();
            let focus = if shown || !held_focus {
                PanelFocus::Preserve
            } else {
                self.focus_after_hiding_primary(cx)
            };
            return Some(PanelTransition::new(Some(key), focus));
        }
        let slot = self.selected_slot(key).filter(|slot| slot.offered())?;
        if self.group(key).is_shown() {
            return Some(self.close(key, window, cx));
        }
        let focus = slot.panel.as_ref()?.focus_handle(cx);
        self.group_mut(key).show();
        Some(PanelTransition::new(Some(key), PanelFocus::Panel(focus)))
    }

    /// Whether focus is inside the group's selected panel.
    fn holds_focus(&self, key: GroupKey, window: &Window, cx: &App) -> bool {
        self.selected(key)
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx))
    }

    fn focus_after_hiding_primary(&self, cx: &App) -> PanelFocus {
        let expanded = self.expanded().and_then(|key| self.selected(key));
        match expanded {
            Some(panel) => PanelFocus::Panel(panel.focus_handle(cx)),
            None => PanelFocus::ActiveItem,
        }
    }

    /// Hides the group after ending its expansion. The tab takes focus if the panel held it.
    pub(super) fn close(
        &mut self,
        key: GroupKey,
        window: &Window,
        cx: &mut App,
    ) -> PanelTransition {
        let had_focus = self.holds_focus(key, window, cx);
        self.set_expanded(key, false, cx);
        let hidden = self.group(key).is_shown();
        if hidden {
            self.group_mut(key).hide();
        }
        PanelTransition::new(
            hidden.then_some(key),
            if had_focus {
                PanelFocus::Central
            } else {
                PanelFocus::Preserve
            },
        )
    }

    /// Withdrawing the selected panel closes its group first; another member's withdrawal
    /// leaves the group as it is.
    fn set_slot_available(
        &mut self,
        key: GroupKey,
        panel: PanelKey,
        available: bool,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        if self.slots.get(panel.index())?.available == available {
            return None;
        }
        if available {
            self.slots.get_mut(panel.index())?.available = true;
            return Some(PanelTransition::new(None, PanelFocus::Preserve));
        }
        let transition = if self.group(key).selected() == Some(panel) {
            self.close(key, window, cx)
        } else {
            PanelTransition::new(None, PanelFocus::Preserve)
        };
        self.slots.get_mut(panel.index())?.available = false;
        Some(transition)
    }

    /// Expands the shown panel over the content, or restores it. Whether anything changed.
    /// Only an available, expandable panel of a shown group expands.
    pub(super) fn set_expanded(&mut self, key: GroupKey, expanded: bool, cx: &mut App) -> bool {
        let can_expand = self
            .selected_slot(key)
            .is_some_and(|slot| slot.offered() && slot.is_expandable());
        if !can_expand || !self.group_mut(key).set_expanded(expanded) {
            return false;
        }
        if let Some(panel) = self.selected(key) {
            panel.set_expanded(expanded, cx);
        }
        true
    }

    /// Tells every expandable panel which side of the content its group is on.
    pub(super) fn set_docked_left(&self, side_of: impl Fn(GroupKey) -> Side, cx: &mut App) {
        for group in &self.groups {
            let left = side_of(group.key()) == Side::Left;
            for member in group.members() {
                if let Some(panel) = self.slot(Some(*member)).and_then(|s| s.panel.as_ref()) {
                    panel.set_docked_left(left, cx);
                }
            }
        }
    }
}

/// Registered panels by the id of their contribution. An unknown id is never open, available or
/// expanded, and no transition applies to it.
impl PanelGroups {
    fn registered(&self, id: &ContributionId) -> Option<(GroupKey, PanelKey)> {
        let panel = self.ids.get(id).copied()?;
        Some((self.slots.get(panel.index())?.home?, panel))
    }
    /// The registered panel's group and its position in the group.
    fn position_of(&self, id: &ContributionId) -> Option<(GroupKey, PanelKey, usize)> {
        let (key, panel) = self.registered(id)?;
        let ix = self
            .group(key)
            .members()
            .iter()
            .position(|member| *member == panel)?;
        Some((key, panel, ix))
    }
    /// The ids of the registered panels, in registration order.
    pub(super) fn registered_ids(&self) -> impl Iterator<Item = &ContributionId> {
        let mut ids: Vec<_> = self.ids.iter().map(|(id, panel)| (*panel, id)).collect();
        ids.sort_by_key(|(panel, _)| panel.0);
        ids.into_iter().map(|(_, id)| id)
    }
    /// The group, position and panel of a registered panel that the host offers.
    pub(super) fn offered(&self, id: &ContributionId) -> Option<(GroupKey, usize, &HostedPanel)> {
        let (key, panel, ix) = self.position_of(id)?;
        let slot = self.slot(Some(panel)).filter(|slot| slot.offered())?;
        Some((key, ix, slot.panel.as_ref()?))
    }
    /// Whether the panel is the shown member of a shown group.
    pub(super) fn panel_is_open(&self, id: &ContributionId) -> bool {
        self.position_of(id)
            .is_some_and(|(key, _, ix)| self.group(key).showing(ix))
    }
    pub(super) fn panel_is_available(&self, id: &ContributionId) -> bool {
        self.registered(id)
            .and_then(|(_, panel)| self.slot(Some(panel)))
            .is_some_and(PanelSlot::offered)
    }
    /// Whether the panel is the member that fills the body.
    pub(super) fn panel_is_expanded(&self, id: &ContributionId) -> bool {
        self.registered(id).is_some_and(|(key, panel)| {
            self.group(key).selected() == Some(panel) && self.group(key).is_expanded()
        })
    }
    /// Hides the panel's group when the panel is showing, else shows the panel. `None` when
    /// the id is unknown or its panel is not offered.
    pub(super) fn toggle_panel(
        &mut self,
        id: &ContributionId,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        let (key, _, ix) = self.position_of(id)?;
        if self.group(key).showing(ix) {
            return self.toggle(key, window, cx);
        }
        self.activate(key, ix, cx)
    }
    /// Shows the panel and focuses it; `None` for an unknown id, an unoffered panel or a panel
    /// that already shows.
    pub(super) fn open_panel(&mut self, id: &ContributionId, cx: &App) -> Option<PanelTransition> {
        let (key, _, ix) = self.position_of(id)?;
        if self.group(key).showing(ix) {
            return None;
        }
        self.activate(key, ix, cx)
    }
    pub(super) fn set_panel_available(
        &mut self,
        id: &ContributionId,
        available: bool,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        let (key, panel) = self.registered(id)?;
        self.set_slot_available(key, panel, available, window, cx)
    }
    /// Expands the panel over the content or restores it. Whether anything changed. Only the
    /// shown member of a group expands.
    pub(super) fn set_panel_expanded(
        &mut self,
        id: &ContributionId,
        expanded: bool,
        cx: &mut App,
    ) -> bool {
        match self.registered(id) {
            Some((key, panel)) if self.group(key).selected() == Some(panel) => {
                self.set_expanded(key, expanded, cx)
            }
            _ => false,
        }
    }
}

impl PanelSlot {
    /// Whether the host offers the slot's panel and the panel exists.
    fn offered(&self) -> bool {
        self.available && self.panel.is_some()
    }
    fn is_expandable(&self) -> bool {
        self.panel.as_ref().is_some_and(HostedPanel::is_expandable)
    }
}

#[cfg(test)]
mod expanded_tests;
#[cfg(test)]
mod tests;
