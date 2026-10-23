//! The window's side panels, held as groups of panels that exclude each other.
//!
//! Every placed panel belongs to exactly one group; opening a member closes its sibling in
//! the group, and groups are independent of each other. Today's two groups are the
//! navigation sidebar (several panels, one shown at a time) and the assistant (one
//! expandable side panel). [`PanelGroups`] owns their state and every transition between
//! states, and returns a [`PanelTransition`] for the workspace to apply to focus and events.

use gpui_kit::{App, EntityId, Window};

use crate::Panel;

mod group;
mod hosted;
mod transition;

use group::PanelGroup;
pub(super) use hosted::{HostedPanel, PanelChrome, Presentation};
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
    /// Stable name used for the group's column.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Navigation => "navigation",
            Self::Assistant => "assistant",
        }
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
        self == Self::Navigation
    }
}

/// Which side of the content a group's column sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Side {
    Left,
    Right,
}

impl Side {
    /// The opposite side when `swapped`.
    pub(super) fn swapped_if(self, swapped: bool) -> Self {
        match (self, swapped) {
            (side, false) => side,
            (Self::Left, true) => Self::Right,
            (Self::Right, true) => Self::Left,
        }
    }
}

/// Groups whose columns share one remembered width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum WidthClass {
    Navigation,
    Assistant,
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
    panel: HostedPanel,
    available: bool,
}

pub(super) struct PanelGroups {
    groups: Vec<PanelGroup>,
    slots: Vec<PanelSlot>,
}

impl PanelGroups {
    pub(super) fn new() -> Self {
        Self {
            groups: GroupKey::ALL.into_iter().map(PanelGroup::new).collect(),
            slots: Vec::new(),
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
    fn selected_slot_mut(&mut self, key: GroupKey) -> Option<&mut PanelSlot> {
        let selected = self.group(key).selected()?;
        self.slots.get_mut(selected.index())
    }

    // ── Registration ─────────────────────────────────────────────────────────

    /// Adds a panel at the end of the group. Panels added to a group start available.
    pub(super) fn add(&mut self, key: GroupKey, panel: HostedPanel) {
        let panel_key = PanelKey(self.slots.len() as u32);
        self.slots.push(PanelSlot {
            panel,
            available: true,
        });
        self.group_mut(key).add(panel_key);
    }

    /// Hosts the one panel of a group. A second panel replaces the first and inherits its
    /// attention; the replaced panel's availability stays. A new panel starts unavailable
    /// until the host offers it.
    pub(super) fn install(&mut self, key: GroupKey, mut panel: HostedPanel) {
        if let Some(slot) = self.selected_slot_mut(key) {
            panel.set_attention(slot.panel.attention());
            slot.panel = panel;
            return;
        }
        self.slots.push(PanelSlot {
            panel,
            available: false,
        });
        let panel_key = PanelKey((self.slots.len() - 1) as u32);
        self.group_mut(key).add(panel_key);
    }

    /// Records a changed title, icon or badge. Whether the panel's presentation changed.
    pub(super) fn presentation_changed(
        &mut self,
        panel: EntityId,
        presentation: Presentation,
    ) -> bool {
        self.slots
            .iter_mut()
            .find(|slot| slot.panel.panel_id() == panel)
            .is_some_and(|slot| slot.panel.presentation_changed(presentation))
    }

    // ── Observations ─────────────────────────────────────────────────────────

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
    /// Whether the group has a panel that the host offers.
    pub(super) fn is_available(&self, key: GroupKey) -> bool {
        self.selected_slot(key).is_some_and(|slot| slot.available)
    }
    pub(super) fn attention(&self, key: GroupKey) -> bool {
        self.selected_slot(key)
            .is_some_and(|slot| slot.panel.attention())
    }
    /// The shown (or last shown) panel of the group.
    pub(super) fn selected(&self, key: GroupKey) -> Option<&HostedPanel> {
        self.selected_slot(key).map(|slot| &slot.panel)
    }
    /// The group's panels in switcher order.
    pub(super) fn members(&self, key: GroupKey) -> impl Iterator<Item = &HostedPanel> {
        self.group(key)
            .members()
            .iter()
            .filter_map(|member| self.slot(Some(*member)))
            .map(|slot| &slot.panel)
    }
    /// Whether the member at `ix` is selected while the group is shown.
    pub(super) fn is_showing(&self, key: GroupKey, ix: usize) -> bool {
        self.group(key).showing(ix)
    }
    pub(super) fn index_of<T: Panel>(&self, key: GroupKey) -> Option<usize> {
        self.members(key).position(|panel| panel.is::<T>())
    }
    /// The member `NextPanel` goes to.
    pub(super) fn next(&self, key: GroupKey) -> Option<usize> {
        self.group(key).next()
    }

    // ── Transitions ──────────────────────────────────────────────────────────

    /// Selects the member at `ix`, shows its group and focuses it. `None` for an unknown
    /// member.
    pub(super) fn activate(
        &mut self,
        key: GroupKey,
        ix: usize,
        cx: &App,
    ) -> Option<PanelTransition> {
        let member = *self.group(key).members().get(ix)?;
        let focus = self.slot(Some(member))?.panel.focus_handle(cx);
        let revealed = self.group_mut(key).select(member);
        Some(PanelTransition::new(
            revealed.then_some(key),
            PanelFocus::Panel(focus),
        ))
    }

    /// Shows a hidden group or hides a shown one.
    ///
    /// The primary group toggles whether or not it has panels, and hiding it moves focus to
    /// the expanded group, else to the active tab, wherever focus was. Any other group
    /// needs an available panel; opening it focuses the panel, and closing it refocuses the
    /// tab only if the panel held focus. `None` when there is nothing to open.
    pub(super) fn toggle(
        &mut self,
        key: GroupKey,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        if key.primary() {
            let shown = self.group_mut(key).toggle();
            let focus = if shown {
                PanelFocus::Preserve
            } else {
                self.focus_after_hiding_primary(cx)
            };
            return Some(PanelTransition::new(Some(key), focus));
        }
        let slot = self.selected_slot(key).filter(|slot| slot.available)?;
        if self.group(key).is_shown() {
            return Some(self.close(key, window, cx));
        }
        let focus = slot.panel.focus_handle(cx);
        self.group_mut(key).show();
        Some(PanelTransition::new(Some(key), PanelFocus::Panel(focus)))
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
        let had_focus = self
            .selected(key)
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx));
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

    /// Offers or withdraws the group's panel. Withdrawing closes the group first. `None`
    /// when nothing changes, including while no panel is installed.
    pub(super) fn set_available(
        &mut self,
        key: GroupKey,
        available: bool,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        if self.selected_slot(key)?.available == available {
            return None;
        }
        if available {
            self.selected_slot_mut(key)?.available = true;
            return Some(PanelTransition::new(None, PanelFocus::Preserve));
        }
        let transition = self.close(key, window, cx);
        self.selected_slot_mut(key)?.available = false;
        Some(transition)
    }

    /// Expands the shown panel over the content, or restores it. Whether anything changed.
    /// Only an available, expandable panel of a shown group expands.
    pub(super) fn set_expanded(&mut self, key: GroupKey, expanded: bool, cx: &mut App) -> bool {
        let Some(slot) = self.selected_slot(key) else {
            return false;
        };
        if !slot.available || !slot.panel.is_expandable() {
            return false;
        }
        if !self.group_mut(key).set_expanded(expanded) {
            return false;
        }
        if let Some(slot) = self.selected_slot(key) {
            slot.panel.set_expanded(expanded, cx);
        }
        true
    }

    /// Whether the attention flag of the group's panel changed.
    pub(super) fn set_attention(&mut self, key: GroupKey, attention: bool) -> bool {
        self.selected_slot_mut(key)
            .is_some_and(|slot| slot.panel.set_attention(attention))
    }

    /// Tells every expandable panel which side of the content it is on.
    pub(super) fn set_docked_left(&self, left: bool, cx: &mut App) {
        for slot in &self.slots {
            slot.panel.set_docked_left(left, cx);
        }
    }
}

#[cfg(test)]
mod tests;
