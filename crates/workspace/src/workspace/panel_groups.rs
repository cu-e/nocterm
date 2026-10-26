//! The window's side panels, held as groups of panels that exclude each other.
//!
//! Every placed panel belongs to exactly one group; opening a member closes its sibling in
//! the group, and groups are independent of each other. The catalog declares the navigation
//! sidebar (several panels, one shown at a time) and the assistant (one expandable side
//! panel); the layout adds the groups the user made and a group of one for each panel it
//! places on its own, so such a panel opens beside the others. [`PanelGroups`] owns their
//! state and every transition between states, and returns a [`PanelTransition`] for the
//! workspace to apply to focus and events.

use std::collections::HashMap;

use gpui_kit::{App, EntityId, Window};
use nocterm_plugin_model::{ContributionId, GroupId, Side, WidthClass};
use nocterm_window_layout::ResolvedLayout;

use crate::plugin::GROUPS;

mod group;
mod hosted;
mod key;
mod transition;

use group::PanelGroup;
pub(super) use hosted::{ExpandableHandle, HostedPanel, Presentation};
pub(super) use key::GroupKey;
use key::GroupPlace;
pub(super) use transition::{PanelFocus, PanelTransition};

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
            groups: GROUPS
                .iter()
                .enumerate()
                .map(|(ix, spec)| PanelGroup::new(GroupKey::at(ix), GroupPlace::of_catalog(spec)))
                .collect(),
            slots: Vec::new(),
            ids: HashMap::new(),
        }
    }

    // A key indexes `groups`, which never shrinks: `new` and `adopt` make every key.
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
        self.place_groups(layout);
        for (id, key) in &self.ids {
            let home = layout.host_of(id).and_then(|group| self.named(group));
            if let Some(slot) = self.slots.get_mut(key.index()) {
                slot.home = home;
                slot.placed = layout.group_of(id).is_some();
            }
        }
        for key in self.keys() {
            let members = self.members_in_layout_order(key, layout);
            self.group_mut(key).arrange(members);
        }
    }

    /// Takes where each group goes from the layout, making a group for each id it has not seen.
    /// A catalog group the bar leaves out returns to its own side.
    fn place_groups(&mut self, layout: &ResolvedLayout) {
        let mut placed = vec![false; self.groups.len()];
        for (order, resolved) in layout.groups().enumerate() {
            let place = GroupPlace::placed(resolved, order);
            let key = match self.named(resolved.id()) {
                Some(key) => {
                    self.group_mut(key).set_place(place);
                    key
                }
                None => {
                    let key = GroupKey::at(self.groups.len());
                    self.groups.push(PanelGroup::new(key, place));
                    placed.push(false);
                    key
                }
            };
            placed[key.index()] = true;
        }
        for (group, placed) in self.groups.iter_mut().zip(placed) {
            if placed {
                continue;
            }
            let place = GROUPS
                .get(group.key().index())
                .map_or_else(|| group.place().unplaced(), GroupPlace::of_catalog);
            group.set_place(place);
        }
    }

    fn keys(&self) -> impl Iterator<Item = GroupKey> + use<> {
        (0..self.groups.len()).map(GroupKey::at)
    }
    /// The group with this id.
    fn named(&self, id: &GroupId) -> Option<GroupKey> {
        self.groups
            .iter()
            .find(|group| group.place().id() == id)
            .map(PanelGroup::key)
    }

    fn members_in_layout_order(&self, key: GroupKey, layout: &ResolvedLayout) -> Vec<PanelKey> {
        let placed = layout
            .groups()
            .filter(|group| group.id() == self.group(key).place().id())
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

    /// Whether the id is registered and its panel does not exist: the plugin that owns it is
    /// disabled, or the factory has not run yet.
    pub(super) fn lacks_panel(&self, id: &ContributionId) -> bool {
        self.ids
            .get(id)
            .and_then(|key| self.slots.get(key.index()))
            .is_some_and(|slot| slot.panel.is_none())
    }

    /// Takes the panel out of a registered id's slot, because its plugin was disabled. A panel
    /// that shows closes first, by the rule for hiding a group: focus moves only if the panel
    /// held it. The slot stays, so enabling the plugin again supplies a new panel with
    /// [`PanelGroups::fill`]. `None` when the slot has no panel.
    ///
    /// The caller drops the returned panel after applying the transition.
    pub(super) fn release(
        &mut self,
        id: &ContributionId,
        window: &Window,
        cx: &mut App,
    ) -> Option<(Option<PanelTransition>, HostedPanel)> {
        let key = self.ids.get(id).copied()?;
        self.slots.get(key.index())?.panel.as_ref()?;
        let transition = if self.panel_is_open(id) {
            self.toggle_panel(id, window, cx)
        } else {
            None
        };
        let panel = self.slots.get_mut(key.index())?.panel.take()?;
        Some((transition, panel))
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
    /// The group's id, which also names its column.
    pub(super) fn id(&self, key: GroupKey) -> &GroupId {
        self.group(key).place().id()
    }
    /// Whether the group is the sidebar: visible from the start and never in need of a member
    /// to be shown.
    pub(super) fn is_primary(&self, key: GroupKey) -> bool {
        self.group(key).place().primary()
    }
    /// The columns of the shown groups that have a member, in bar order, with their sides and
    /// classes of width. Groups the bar does not place come last.
    pub(super) fn columns(&self) -> Vec<(GroupKey, Side, WidthClass)> {
        let mut shown: Vec<&PanelGroup> = self
            .groups
            .iter()
            .filter(|group| group.is_shown() && !group.members().is_empty())
            .collect();
        shown.sort_by_key(|group| group.place().order().unwrap_or(usize::MAX));
        shown
            .into_iter()
            .map(|group| (group.key(), group.place().side(), group.place().width()))
            .collect()
    }
    /// The group `ToggleRightPanel` acts on: the assistant while it has a member, else the
    /// first group of the assistant's width that has one, as when the user took the
    /// assistant's panel out of its group.
    pub(super) fn assistant(&self) -> GroupKey {
        let holds_assistant = |group: &&PanelGroup| {
            !group.members().is_empty() && group.place().width() == WidthClass::Assistant
        };
        self.groups
            .iter()
            .find(holds_assistant)
            .map_or(GroupKey::ASSISTANT, PanelGroup::key)
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
        if self.is_primary(key) {
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
    pub(super) fn set_docked_left(&self, cx: &mut App) {
        for group in &self.groups {
            let left = group.place().side() == Side::Left;
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
    /// Shows exactly the panels of `open` and hides the groups none of them belongs to,
    /// leaving focus where it is unless a hidden group held it. A group opens at most one
    /// panel: the first of `open` that the host offers.
    pub(super) fn set_open(
        &mut self,
        open: &[ContributionId],
        window: &Window,
        cx: &mut App,
    ) -> Vec<PanelTransition> {
        let mut transitions = Vec::new();
        for key in self.keys() {
            let wanted = open
                .iter()
                .filter_map(|id| self.offered(id).map(|(group, ix, _)| (group, ix)))
                .find(|(group, _)| *group == key)
                .map(|(_, ix)| ix);
            match wanted {
                Some(ix) if self.group(key).showing(ix) => {}
                Some(ix) => {
                    if let Some(member) = self.group(key).members().get(ix).copied() {
                        let revealed = self.group_mut(key).select(member);
                        transitions.push(PanelTransition::new(
                            revealed.then_some(key),
                            PanelFocus::Preserve,
                        ));
                    }
                }
                None if self.group(key).is_shown() => {
                    transitions.push(self.close(key, window, cx));
                }
                None => {}
            }
        }
        transitions
    }
    /// Follows a new arrangement: membership and order come from the layout, a switch to a
    /// saved layout shows exactly the panels it names (`open`), and the panels the layout no
    /// longer places close.
    pub(super) fn follow(
        &mut self,
        layout: &ResolvedLayout,
        open: Option<&[ContributionId]>,
        window: &Window,
        cx: &mut App,
    ) -> Vec<PanelTransition> {
        self.adopt(layout);
        let mut transitions = open
            .map(|open| self.set_open(open, window, cx))
            .unwrap_or_default();
        let stray: Vec<_> = self
            .registered_ids()
            .filter(|id| self.panel_is_open(id) && !layout.is_placed(id))
            .cloned()
            .collect();
        for id in stray {
            transitions.extend(self.toggle_panel(&id, window, cx));
        }
        transitions
    }
    /// Shows one panel without taking focus; used while a window composes its panels.
    pub(super) fn reveal_panel(&mut self, id: &ContributionId) -> Option<PanelTransition> {
        let (key, panel, _) = self.position_of(id)?;
        self.slot(Some(panel)).filter(|slot| slot.offered())?;
        let revealed = self.group_mut(key).select(panel);
        Some(PanelTransition::new(
            revealed.then_some(key),
            PanelFocus::Preserve,
        ))
    }
    pub(super) fn set_panel_available(
        &mut self,
        id: &ContributionId,
        available: bool,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        let Some((key, panel)) = self.registered(id) else {
            // A panel the layout does not host, as while its plugin is disabled, only keeps
            // what its owner last said, for when it returns.
            let key = self.ids.get(id).copied()?;
            self.slots.get_mut(key.index())?.available = available;
            return None;
        };
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

/// Why a plugin may not show its panel now.
#[cfg(feature = "user-plugins")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpeningRefused {
    /// Another member of the panel's group shows, and showing this one would replace it.
    WouldReplace,
    /// A panel asks for the user, so nothing else may take the screen.
    UserIsBeingAsked,
}

/// What a user plugin may do to the panels, under the rules that protect what the user is doing.
#[cfg(feature = "user-plugins")]
impl PanelGroups {
    /// Shows the registered panel without taking focus. It refuses when that would replace a
    /// panel that shows (the group is shown with another member selected) or while any panel
    /// asks for the user; a panel that shows already is not a replacement. `Ok(None)` when
    /// nothing changes: an unknown id or a panel that is not offered.
    pub(super) fn open_for_plugin(
        &mut self,
        id: &ContributionId,
    ) -> Result<Option<PanelTransition>, OpeningRefused> {
        if let Some((key, _, ix)) = self.position_of(id) {
            let group = self.group(key);
            if group.is_shown() && !group.showing(ix) {
                return Err(OpeningRefused::WouldReplace);
            }
        }
        if self
            .slots
            .iter()
            .filter_map(|slot| slot.panel.as_ref())
            .any(HostedPanel::attention)
        {
            return Err(OpeningRefused::UserIsBeingAsked);
        }
        Ok(self.reveal_panel(id))
    }

    /// Closes the registered panel's group if the panel is the one showing, leaving focus alone
    /// unless the panel held it. `None` when it does not show.
    pub(super) fn close_for_plugin(
        &mut self,
        id: &ContributionId,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        let (key, _, ix) = self.position_of(id)?;
        self.group(key)
            .showing(ix)
            .then(|| self.close(key, window, cx))
    }

    /// Removes a registration whose plugin was uninstalled or replaced. A panel that shows closes
    /// first, as when its plugin is disabled. Returns the transition and the panel for the
    /// caller to apply and drop; the id can register again.
    pub(super) fn forget(
        &mut self,
        id: &ContributionId,
        window: &Window,
        cx: &mut App,
    ) -> (Option<PanelTransition>, Option<HostedPanel>) {
        let released = self.release(id, window, cx);
        self.ids.remove(id);
        match released {
            Some((transition, panel)) => (transition, Some(panel)),
            None => (None, None),
        }
    }
}

#[cfg(test)]
mod expanded_tests;
#[cfg(test)]
mod tests;
