//! One linear undo history of the layout edits made here.

use nocterm_window_layout::{EditOutcome, LayoutPreset};

/// The most steps kept either way.
const DEPTH: usize = 100;

/// How an outcome came about, which decides where its `before` goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Edit,
    Undo,
    Redo,
}

/// The lists of presets to go back to and to go forward to, and the list this editor last
/// wrote. A list in the settings that is not the one last written was written by someone else,
/// and the steps no longer fit it.
#[derive(Debug)]
pub(crate) struct History {
    undo: Vec<Vec<LayoutPreset>>,
    redo: Vec<Vec<LayoutPreset>>,
    written: Vec<LayoutPreset>,
}

impl History {
    /// An empty history over the presets as they are now.
    pub(crate) fn new(presets: Vec<LayoutPreset>) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            written: presets,
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The presets an undo restores.
    pub(crate) fn previous(&self) -> Option<&Vec<LayoutPreset>> {
        self.undo.last()
    }

    /// The presets a redo restores.
    pub(crate) fn next(&self) -> Option<&Vec<LayoutPreset>> {
        self.redo.last()
    }

    /// Records a write of this editor that was stored.
    pub(crate) fn record(&mut self, step: Step, outcome: &EditOutcome) {
        match step {
            Step::Edit => {
                if outcome.before != outcome.after {
                    push(&mut self.undo, outcome.before.clone());
                    self.redo.clear();
                }
            }
            Step::Undo => {
                self.undo.pop();
                push(&mut self.redo, outcome.before.clone());
            }
            Step::Redo => {
                self.redo.pop();
                push(&mut self.undo, outcome.before.clone());
            }
        }
        self.written = outcome.after.clone();
    }

    /// Forgets every step if `stored` is not what this editor last wrote. Returns whether it
    /// did.
    pub(crate) fn observe(&mut self, stored: &[LayoutPreset]) -> bool {
        if self.written == stored {
            return false;
        }
        self.undo.clear();
        self.redo.clear();
        self.written = stored.to_vec();
        true
    }
}

fn push(steps: &mut Vec<Vec<LayoutPreset>>, presets: Vec<LayoutPreset>) {
    steps.push(presets);
    if steps.len() > DEPTH {
        steps.remove(0);
    }
}
