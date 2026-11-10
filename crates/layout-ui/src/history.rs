//! Linear snapshot undo with both step and retained-byte budgets.

use nocterm_window_layout::{BarItem, EditOutcome, GroupEntry, LayoutPreset};
use std::collections::VecDeque;

const DEPTH: usize = 100;
const MAX_BYTES: usize = 8 * 1024 * 1024;
type Snapshot = (Vec<LayoutPreset>, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Edit,
    Undo,
    Redo,
}

/// History contains complete snapshots only while their aggregate bytes fit the budget.
/// The current document is compared by reference before another copy is retained.
#[derive(Debug)]
pub(crate) struct History {
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    written: Vec<LayoutPreset>,
    bytes: usize,
}

impl History {
    pub(crate) fn new(presets: Vec<LayoutPreset>) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            written: presets,
            bytes: 0,
        }
    }
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub(crate) fn previous(&self) -> Option<&Vec<LayoutPreset>> {
        self.undo.back().map(|(value, _)| value)
    }
    pub(crate) fn next(&self) -> Option<&Vec<LayoutPreset>> {
        self.redo.back().map(|(value, _)| value)
    }

    pub(crate) fn record(&mut self, step: Step, outcome: &EditOutcome) {
        match step {
            Step::Edit => {
                if outcome.before != outcome.after {
                    for (_, bytes) in self.redo.drain(..) {
                        self.bytes -= bytes;
                    }
                    push(&mut self.undo, &mut self.bytes, &outcome.before);
                }
            }
            Step::Undo => {
                pop(&mut self.undo, &mut self.bytes);
                push(&mut self.redo, &mut self.bytes, &outcome.before);
            }
            Step::Redo => {
                pop(&mut self.redo, &mut self.bytes);
                push(&mut self.undo, &mut self.bytes, &outcome.before);
            }
        }
        while self.bytes > MAX_BYTES {
            let removed = self.undo.pop_front().or_else(|| self.redo.pop_front());
            if let Some((_, bytes)) = removed {
                self.bytes -= bytes;
            } else {
                break;
            }
        }
        self.written = outcome.after.clone();
    }
    pub(crate) fn observe(&mut self, stored: &[LayoutPreset]) -> bool {
        if self.written == stored {
            return false;
        }
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.written = stored.to_vec();
        true
    }
}
fn pop(steps: &mut VecDeque<Snapshot>, bytes: &mut usize) {
    if let Some((_, removed)) = steps.pop_back() {
        *bytes -= removed;
    }
}
fn push(steps: &mut VecDeque<Snapshot>, bytes: &mut usize, presets: &[LayoutPreset]) {
    let size = snapshot_bytes(presets);
    if size > MAX_BYTES {
        for (_, removed) in steps.drain(..) {
            *bytes -= removed;
        }
        return;
    }
    steps.push_back((presets.to_vec(), size));
    *bytes += size;
    if steps.len() > DEPTH
        && let Some((_, removed)) = steps.pop_front()
    {
        *bytes -= removed;
    }
}
/// Counts allocations made by cloning this document, including all nested owned strings.
fn snapshot_bytes(presets: &[LayoutPreset]) -> usize {
    let mut bytes = std::mem::size_of_val(presets);
    for preset in presets {
        bytes += preset.id.as_str().len() + preset.name.len();
        for ids in [&preset.tray, &preset.open] {
            bytes += std::mem::size_of_val(ids.as_slice());
            bytes += ids.iter().map(|id| id.as_str().len()).sum::<usize>();
        }
        for zone in [&preset.bar.left, &preset.bar.center, &preset.bar.right] {
            bytes += std::mem::size_of_val(zone.as_slice());
            for item in zone {
                bytes += match item {
                    BarItem::Item { id } | BarItem::Panel { id, .. } => id.as_str().len(),
                    BarItem::Separator => 0,
                    BarItem::Group {
                        id, name, members, ..
                    } => {
                        id.as_str().len()
                            + name.as_ref().map_or(0, String::len)
                            + std::mem::size_of_val(members.as_slice())
                            + members
                                .iter()
                                .map(|member| match member {
                                    GroupEntry::Panel(id) => id.as_str().len(),
                                    GroupEntry::Separator => 0,
                                })
                                .sum::<usize>()
                    }
                };
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_window_layout::PresetId;

    #[test]
    fn a_snapshot_larger_than_the_byte_budget_is_not_offered_for_undo() {
        let before = vec![LayoutPreset {
            name: "large".repeat(MAX_BYTES / 5 + 1),
            ..Default::default()
        }];
        let mut history = History::new(before.clone());
        history.record(
            Step::Edit,
            &EditOutcome {
                before,
                after: Vec::new(),
                selected: PresetId::new("default"),
            },
        );
        assert!(!history.can_undo());
        assert!(history.previous().is_none());
    }

    #[test]
    fn an_unrelated_publication_keeps_the_same_undo_step() {
        let before = vec![LayoutPreset {
            name: "Original".into(),
            ..Default::default()
        }];
        let mut after = before.clone();
        after[0].name = "Changed".into();
        let mut history = History::new(before.clone());
        history.record(
            Step::Edit,
            &EditOutcome {
                before: before.clone(),
                after: after.clone(),
                selected: PresetId::new("default"),
            },
        );
        assert!(!history.observe(&after));
        assert_eq!(history.previous(), Some(&before));
    }
}
