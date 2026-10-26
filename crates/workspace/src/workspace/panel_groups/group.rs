//! One group of panels that exclude each other: which member is selected, whether the group
//! is shown and whether it fills the body.

use super::{GroupKey, PanelKey, key::GroupPlace};

/// The state of one group. It knows nothing about the panels themselves; the owner checks
/// availability and expandability before it asks for a transition.
///
/// Invariants: `expanded` implies `shown` and a selected member; `selected` is a member.
pub(super) struct PanelGroup {
    key: GroupKey,
    place: GroupPlace,
    members: Vec<PanelKey>,
    shown: bool,
    selected: Option<PanelKey>,
    expanded: bool,
}

impl PanelGroup {
    /// A primary group starts shown, even while it has no member.
    pub(super) fn new(key: GroupKey, place: GroupPlace) -> Self {
        Self {
            key,
            shown: place.primary(),
            place,
            members: Vec::new(),
            selected: None,
            expanded: false,
        }
    }
    pub(super) fn key(&self) -> GroupKey {
        self.key
    }
    pub(super) fn place(&self) -> &GroupPlace {
        &self.place
    }
    pub(super) fn set_place(&mut self, place: GroupPlace) {
        self.place = place;
    }
    pub(super) fn members(&self) -> &[PanelKey] {
        &self.members
    }
    pub(super) fn selected(&self) -> Option<PanelKey> {
        self.selected
    }
    pub(super) fn is_shown(&self) -> bool {
        self.shown
    }
    pub(super) fn is_expanded(&self) -> bool {
        self.expanded
    }
    /// Replaces the members. The selection stays while it is still a member, and otherwise
    /// moves to the first member, which ends an expansion; a group left without members
    /// closes unless it is the primary group.
    pub(super) fn arrange(&mut self, members: Vec<PanelKey>) {
        self.members = members;
        if self
            .selected
            .is_some_and(|panel| self.members.contains(&panel))
        {
            return;
        }
        self.selected = self.members.first().copied();
        self.expanded = false;
        if self.selected.is_none() && !self.place.primary() {
            self.shown = false;
        }
    }
    /// Selects a member and shows the group; the previous selection is no longer shown.
    /// Whether the group was hidden before.
    pub(super) fn select(&mut self, panel: PanelKey) -> bool {
        self.selected = Some(panel);
        !std::mem::replace(&mut self.shown, true)
    }
    /// Flips visibility and returns whether the group is now shown.
    pub(super) fn toggle(&mut self) -> bool {
        if self.shown {
            self.hide();
        } else {
            self.shown = true;
        }
        self.shown
    }
    pub(super) fn show(&mut self) {
        self.shown = true;
    }
    /// Hides the group, which also ends its expansion.
    pub(super) fn hide(&mut self) {
        self.shown = false;
        self.expanded = false;
    }
    /// Whether the expansion changed. Only a shown group with a selected member expands.
    pub(super) fn set_expanded(&mut self, expanded: bool) -> bool {
        if !self.shown || self.selected.is_none() || self.expanded == expanded {
            return false;
        }
        self.expanded = expanded;
        true
    }
    /// Whether the member at `ix` is selected and the group is shown.
    pub(super) fn showing(&self, ix: usize) -> bool {
        self.shown && self.selected_position() == Some(ix)
    }
    /// The position of the selected member.
    pub(super) fn selected_position(&self) -> Option<usize> {
        let selected = self.selected?;
        self.members.iter().position(|member| *member == selected)
    }
}
