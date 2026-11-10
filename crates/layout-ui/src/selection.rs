//! Which layout the editor shows and which item of it is selected.

use nocterm_window_layout::{EditOp, EditOutcome, EditTarget, PresetId, Subject};

/// The layout and the item the editor shows.
#[derive(Debug, Default)]
pub(crate) struct Selection {
    /// The layout picked in the picker; `None` follows the layout in use.
    shown: Option<PresetId>,
    item: Option<Subject>,
}

impl Selection {
    pub(crate) fn shown(&self) -> Option<&PresetId> {
        self.shown.as_ref()
    }

    pub(crate) fn item(&self) -> Option<&Subject> {
        self.item.as_ref()
    }

    /// Shows `id`, which follows the layout in use when it is that one.
    pub(crate) fn show(&mut self, id: PresetId, in_use: &PresetId) {
        self.shown = (id != *in_use).then_some(id);
        self.item = None;
    }

    pub(crate) fn pick(&mut self, item: Option<Subject>) {
        self.item = item;
    }

    /// Selects nothing.
    pub(crate) fn clear(&mut self) {
        self.item = None;
    }

    /// The target of an operation on the layout shown: the layout in use is named by its role,
    /// so that an operation queued behind the first edit of a built-in lands on the copy.
    pub(crate) fn target(&self, in_use: &PresetId) -> EditTarget {
        match &self.shown {
            Some(id) if id != in_use => EditTarget::Preset(id.clone()),
            _ => EditTarget::Selected,
        }
    }

    /// Follows a stored edit: the copy a duplicate made is shown, a deleted layout gives way
    /// to the one the outcome names, and undo keeps the layout shown while it exists.
    pub(crate) fn followed(
        &mut self,
        op: &EditOp,
        outcome: &EditOutcome,
        in_use: &PresetId,
        exists: impl Fn(&PresetId) -> bool,
    ) {
        match op {
            EditOp::Duplicate | EditOp::Delete => self.show(outcome.selected.clone(), in_use),
            EditOp::RestorePresets(_) => {
                if self.shown.as_ref().is_some_and(|id| !exists(id)) {
                    self.shown = None;
                }
            }
            _ => {
                if self.shown.is_some() {
                    self.shown = (outcome.selected != *in_use).then(|| outcome.selected.clone());
                }
            }
        }
    }
}
