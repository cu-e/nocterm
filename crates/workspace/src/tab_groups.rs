//! Tabs kept together.
//!
//! A group is a set of tabs that sit side by side in one tab bar and move as
//! one. The dock knows nothing of it: it moves a single tab wherever the user
//! drags it. Afterwards [`TabGroups::gather`] compares the tab bars with the
//! layout it last saw, finds the member that was moved and brings the rest of
//! its group back beside it, in the order they had. So dragging any member
//! drags the group, a tab dropped into the middle of a group is moved out of
//! it, and reordering inside a group just reorders it.
//!
//! Generic over the dock's panel and node ids, so the rules are tested
//! without a window.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

/// Names a group of tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GroupId(u64);

/// One step towards a gathered group, with the dock's own move semantics:
/// `panel` leaves its place, then is inserted at `ix` of `node`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Move<P, N> {
    pub panel: P,
    pub node: N,
    pub ix: usize,
}

/// The tab bars, each a node and the panels in it from left to right.
pub(crate) type Bars<P, N> = Vec<(N, Vec<P>)>;

pub(crate) struct TabGroups<P, N> {
    next: u64,
    /// Each group and the color it is drawn in, an index into a palette.
    colors: BTreeMap<GroupId, usize>,
    member: HashMap<P, GroupId>,
    /// Where each member was when the groups were last gathered. A member
    /// without a place has just joined, so it follows rather than leads.
    last: HashMap<P, (N, usize)>,
}

impl<P, N> Default for TabGroups<P, N> {
    fn default() -> Self {
        Self {
            next: 0,
            colors: BTreeMap::new(),
            member: HashMap::new(),
            last: HashMap::new(),
        }
    }
}

impl<P: Copy + Eq + Hash, N: Copy + Eq> TabGroups<P, N> {
    pub(crate) fn group_of(&self, panel: P) -> Option<GroupId> {
        self.member.get(&panel).copied()
    }

    /// The palette index `group` is drawn in.
    pub(crate) fn color(&self, group: GroupId) -> Option<usize> {
        self.colors.get(&group).copied()
    }

    pub(crate) fn groups(&self) -> impl Iterator<Item = GroupId> + '_ {
        self.colors.keys().copied()
    }

    pub(crate) fn members(&self, group: GroupId) -> impl Iterator<Item = P> + '_ {
        self.member
            .iter()
            .filter(move |(_, of)| **of == group)
            .map(|(panel, _)| *panel)
    }

    /// Starts a group with `panel` in it, taking it out of any other, in the
    /// first of `palette` colors no other group is drawn in.
    pub(crate) fn create(&mut self, panel: P, palette: usize) -> GroupId {
        let group = GroupId(self.next);
        self.next += 1;
        let color = (0..palette.max(1))
            .find(|color| !self.colors.values().any(|used| used == color))
            .unwrap_or(group.0 as usize % palette.max(1));
        self.colors.insert(group, color);
        self.join(panel, group);
        group
    }

    /// Puts `panel` in `group`, out of any other; the next gathering brings
    /// it beside the group's other members.
    pub(crate) fn join(&mut self, panel: P, group: GroupId) {
        if !self.colors.contains_key(&group) || self.group_of(panel) == Some(group) {
            return;
        }
        self.leave(panel);
        self.member.insert(panel, group);
        self.last.remove(&panel);
    }

    /// Takes `panel` out of its group. A group left with a single tab is
    /// no longer a group.
    pub(crate) fn leave(&mut self, panel: P) {
        let Some(group) = self.member.remove(&panel) else {
            return;
        };
        self.last.remove(&panel);
        if self.members(group).count() < 2 {
            self.dissolve(group);
        }
    }

    pub(crate) fn dissolve(&mut self, group: GroupId) {
        self.colors.remove(&group);
        let members: Vec<P> = self.members(group).collect();
        for panel in members {
            self.member.remove(&panel);
            self.last.remove(&panel);
        }
    }

    /// The moves that bring every group back together in `bars`, the dock's
    /// tab bars as they are now. Members no longer in any bar have closed
    /// and leave their group.
    pub(crate) fn gather(&mut self, bars: &[(N, Vec<P>)]) -> Vec<Move<P, N>> {
        let mut bars: Bars<P, N> = bars.to_vec();
        let gone: Vec<P> = self
            .member
            .keys()
            .copied()
            .filter(|panel| locate(&bars, *panel).is_none())
            .collect();
        for panel in gone {
            self.leave(panel);
        }

        let mut moves = Vec::new();
        let groups: Vec<GroupId> = self.groups().collect();
        // Gathering one group can split another it lands in; each pass
        // settles at least one, so as many passes as groups always suffice.
        for _ in 0..=groups.len() {
            let before = moves.len();
            for group in &groups {
                self.gather_group(*group, &mut bars, &mut moves);
            }
            if moves.len() == before {
                break;
            }
        }

        self.last = self
            .member
            .keys()
            .filter_map(|panel| Some((*panel, locate(&bars, *panel)?)))
            .map(|(panel, (bar, ix))| (panel, (bars[bar].0, ix)))
            .collect();
        moves
    }

    fn gather_group(&self, group: GroupId, bars: &mut Bars<P, N>, moves: &mut Vec<Move<P, N>>) {
        // Members where they are now, in display order.
        let mut spots: Vec<(P, usize, usize)> = self
            .members(group)
            .filter_map(|panel| {
                let (bar, ix) = locate(bars, panel)?;
                Some((panel, bar, ix))
            })
            .collect();
        spots.sort_by_key(|(_, bar, ix)| (*bar, *ix));
        if spots.len() < 2 || together(&spots) {
            return;
        }

        // The order the group had: where each member was, newcomers last.
        let mut order: Vec<P> = spots.iter().map(|(panel, ..)| *panel).collect();
        order.sort_by_key(|panel| {
            let last = self.last.get(panel).map(|(_, ix)| *ix);
            (last.is_none(), last)
        });

        // The member that was moved leads: one that changed bars, else the
        // one displaced the furthest, else the leftmost. A newcomer never
        // leads, so joining brings a tab to its group, not the other way.
        let (anchor, ..) = spots
            .iter()
            .enumerate()
            .max_by_key(|(at, (panel, bar, ix))| {
                let last = self.last.get(panel);
                let changed_bar = last.is_some_and(|(node, _)| *node != bars[*bar].0);
                let displaced = last
                    .filter(|(node, _)| *node == bars[*bar].0)
                    .map_or(0, |(_, was)| was.abs_diff(*ix));
                (last.is_some(), changed_bar, displaced, Reverse(*at))
            })
            .map(|(_, spot)| *spot)
            .expect("a group of two has a member");
        let node = bars[locate(bars, anchor).expect("the anchor is in a bar").0].0;
        let lead = order
            .iter()
            .position(|panel| *panel == anchor)
            .expect("the anchor is a member");

        for panel in &order[..lead] {
            let ix = place(bars, *panel, node, anchor, false);
            moves.push(Move {
                panel: *panel,
                node,
                ix,
            });
        }
        let mut previous = anchor;
        for panel in &order[lead + 1..] {
            let ix = place(bars, *panel, node, previous, true);
            moves.push(Move {
                panel: *panel,
                node,
                ix,
            });
            previous = *panel;
        }
    }
}

/// The bar holding `panel` and its index there.
fn locate<P: Eq, N>(bars: &[(N, Vec<P>)], panel: P) -> Option<(usize, usize)> {
    bars.iter().enumerate().find_map(|(bar, (_, panels))| {
        panels
            .iter()
            .position(|candidate| *candidate == panel)
            .map(|ix| (bar, ix))
    })
}

/// Whether members, in display order, sit side by side in one bar.
fn together<P>(spots: &[(P, usize, usize)]) -> bool {
    spots
        .windows(2)
        .all(|pair| pair[0].1 == pair[1].1 && pair[0].2 + 1 == pair[1].2)
}

/// Moves `panel` right before or after `beside` in the bar `node`, as the
/// dock would, and returns the index it was inserted at.
fn place<P: Copy + Eq, N: Copy + Eq>(
    bars: &mut Bars<P, N>,
    panel: P,
    node: N,
    beside: P,
    after: bool,
) -> usize {
    if let Some((bar, ix)) = locate(bars, panel) {
        bars[bar].1.remove(ix);
    }
    let (bar, at) = locate(bars, beside).expect("the member beside is in a bar");
    debug_assert!(bars[bar].0 == node);
    let ix = at + usize::from(after);
    bars[bar].1.insert(ix, panel);
    ix
}

#[cfg(test)]
mod tests;
