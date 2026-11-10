//! Context/filter projection stores positions, never saved snippet content.
use gpui_kit::{ListAlignment, ListState, px};
use nocterm_snippets::Library;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    Current,
    Snippet(usize, [u8; 16]),
    Other(usize),
    Empty,
}

pub(super) struct SnippetListing {
    current: Vec<(usize, [u8; 16])>,
    other: Vec<(usize, [u8; 16])>,
    rows: Rc<Vec<Row>>,
    scroll: ListState,
    folded: bool,
    dirty: bool,
}
impl Default for SnippetListing {
    fn default() -> Self {
        Self {
            current: Vec::new(),
            other: Vec::new(),
            rows: Rc::default(),
            scroll: ListState::new(0, ListAlignment::Top, px(100.)),
            folded: false,
            dirty: true,
        }
    }
}
impl SnippetListing {
    pub(super) fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub(super) fn prepare(
        &mut self,
        library: &Library,
        profile: Option<&str>,
        group: Option<&str>,
        query: &str,
    ) {
        if self.dirty {
            let (current, other) = library.partition_indices(profile, group, query);
            let positions = |indices: Vec<usize>| {
                indices
                    .into_iter()
                    .map(|index| (index, *library.snippets[index].id.as_bytes()))
                    .collect()
            };
            (self.current, self.other) = (positions(current), positions(other));
            self.dirty = false;
            self.rebuild();
        }
    }
    pub(super) fn toggle(&mut self) {
        self.folded = !self.folded;
        self.rebuild();
    }
    pub(super) fn folded(&self) -> bool {
        self.folded
    }
    pub(super) fn rows(&self) -> Rc<Vec<Row>> {
        self.rows.clone()
    }
    pub(super) fn scroll(&self) -> ListState {
        self.scroll.clone()
    }
    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        if !self.current.is_empty() {
            rows.push(Row::Current);
            rows.extend(
                self.current
                    .iter()
                    .copied()
                    .map(|(index, id)| Row::Snippet(index, id)),
            );
        }
        if !self.other.is_empty() {
            rows.push(Row::Other(self.other.len()));
            if !self.folded {
                rows.extend(
                    self.other
                        .iter()
                        .copied()
                        .map(|(index, id)| Row::Snippet(index, id)),
                );
            }
        }
        if rows.is_empty() {
            rows.push(Row::Empty);
        }
        let offset = self.scroll.logical_scroll_top();
        // Supply extent for unseen rows; actual visible heights replace the hint.
        self.scroll.reset_with_uniform_height(rows.len(), px(56.));
        self.scroll.scroll_to(offset);
        self.rows = Rc::new(rows);
    }
}
