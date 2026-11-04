//! Row membership and range-anchor transitions shared by both Explorer panes.
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct RowSelection {
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
}
impl RowSelection {
    pub(super) fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }
    pub(super) fn click(&mut self, index: usize, shift: bool, toggle: bool) {
        if shift {
            let anchor = self.anchor.unwrap_or(index);
            self.selected.extend(anchor.min(index)..=anchor.max(index));
        } else if toggle {
            if !self.selected.insert(index) {
                self.selected.remove(&index);
            }
            self.anchor = Some(index);
        } else {
            self.replace(index);
        }
    }
    /// A context click keeps a multi-selection when its row is already a member.
    pub(super) fn context_click(&mut self, index: usize) -> bool {
        if self.contains(index) {
            false
        } else {
            self.replace(index);
            true
        }
    }
    fn replace(&mut self, index: usize) {
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
    }
    pub(super) fn contains(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }
    pub(super) fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected.iter().copied()
    }
    pub(super) fn for_row(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let selected = self.contains(index);
        self.indices()
            .take(if selected { self.selected.len() } else { 0 })
            .chain((!selected).then_some(index))
    }
}
